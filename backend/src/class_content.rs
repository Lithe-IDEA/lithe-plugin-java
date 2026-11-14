//! Read-only binary sources owned by JDT LS. Only the plugin understands the
//! `jdt:` scheme; the host owns tabs, cancellation and subscription cleanup.

use crate::settings_session::{SettingsSession, REQUEST_METHOD, SERVER_ID};
use lithe_plugin_sdk::{
    BackendHandle, Context, FunctionError, PluginError, SdkError, Subscription, Value,
    WorkspaceContext,
};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, PoisonError};
use url::Url;

pub const CONTENT_FUNCTION: &str = "java.classFileContents";
const REGISTER_METHOD: &str = "lithe/host/documents/registerContentProvider";
const CONTENT_METHOD: &str = "java/classFileContents";
const CONTENT_TIMEOUT_MS: u64 = 25_000;
const MAX_URI_BYTES: usize = 16 * 1024;
const MAX_CONTENT_BYTES: usize = 1024 * 1024;
pub const REGISTRATION_FAILED: &str = "java.content_registration_failed";

struct Registration {
    generation: u64,
    subscription: Subscription,
}

/// Bindings belong to workspace generations, not individual JDT processes.
/// A stopped process leaves its binding in place; reads fail until JDT restarts.
pub struct ClassContentSession {
    handle: BackendHandle,
    registrations: Mutex<HashMap<String, Registration>>,
}
impl ClassContentSession {
    pub fn new(handle: BackendHandle) -> Self {
        Self {
            handle,
            registrations: Mutex::new(HashMap::new()),
        }
    }

    /// Serialize registration mutations, but never fetch class text while holding
    /// this gate. The host records registrations without calling their callback.
    pub fn sync(&self, settings: &SettingsSession) -> Result<u64, SdkError> {
        // Serialize the snapshot too: a delayed old context list must not
        // dispose a binding that a newer synchronization already registered.
        let mut bindings = self
            .registrations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let host = self.handle.host();
        let listed = host.request("lithe/host/workspace/listContexts", json!({}))?;
        let contexts = listed
            .get("contexts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut seen = HashSet::new();
        let mut failures = 0;
        for value in contexts {
            let context = WorkspaceContext::from_value(&value)?;
            let id = context.context_id().to_owned();
            seen.insert(id.clone());
            if bindings
                .get(&id)
                .is_some_and(|b| b.generation == context.generation())
            {
                continue;
            }
            if !settings.server_is_running(&context) {
                continue;
            }
            if self.bind(&mut bindings, &context).is_err() {
                failures += 1;
            }
        }
        let gone: Vec<_> = bindings
            .keys()
            .filter(|id| !seen.contains(*id))
            .cloned()
            .collect();
        for id in gone {
            if let Some(old) = bindings.remove(&id) {
                if old.subscription.dispose().is_err() {
                    failures += 1;
                }
            }
        }
        Ok(failures)
    }

    /// Complete the registration before returning a valid JDT launch plan.
    /// This prevents a fast first definition from beating the state callback.
    pub fn ensure(&self, context: &WorkspaceContext) -> Result<(), SdkError> {
        let mut bindings = self
            .registrations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.bind(&mut bindings, context)
    }

    fn bind(
        &self,
        bindings: &mut HashMap<String, Registration>,
        context: &WorkspaceContext,
    ) -> Result<(), SdkError> {
        let id = context.context_id();
        if bindings
            .get(id)
            .is_some_and(|b| b.generation == context.generation())
        {
            return Ok(());
        }
        if let Some(old) = bindings.remove(id) {
            old.subscription.dispose()?;
        }
        let subscription = self.handle.host().subscribe(
            REGISTER_METHOD,
            json!({"context":context.to_value(),"scheme":"jdt","function":CONTENT_FUNCTION}),
        )?;
        bindings.insert(
            id.to_owned(),
            Registration {
                generation: context.generation(),
                subscription,
            },
        );
        Ok(())
    }

    /// One restricted diagnostic per failed synchronization boundary, no paths
    /// or upstream responses. Expected context disposal remains a successful no-op.
    pub fn sync_reporting(&self, settings: &SettingsSession, report: &dyn Fn(&str, u64)) {
        match self.sync(settings) {
            Ok(0) => {}
            Ok(count) => report(REGISTRATION_FAILED, count),
            Err(_) => report(REGISTRATION_FAILED, 1),
        }
    }
}

fn failure(code: &str, message: &str) -> FunctionError {
    PluginError::new(code, message).into()
}
fn validated_uri(params: &Value) -> Result<&str, FunctionError> {
    let uri = params
        .as_object()
        .filter(|p| p.len() == 1)
        .and_then(|p| p.get("uri"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            failure(
                "java.class_content.invalid_params",
                "Invalid Java class content request",
            )
        })?;
    if uri.len() > MAX_URI_BYTES
        || uri.chars().any(char::is_control)
        || !Url::parse(uri).is_ok_and(|u| {
            u.scheme() == "jdt"
                && !u.path().is_empty()
                && u.username().is_empty()
                && u.password().is_none()
        })
    {
        return Err(failure(
            "java.class_content.invalid_params",
            "Invalid Java class content request",
        ));
    }
    Ok(uri)
}
fn content_result(answer: Value) -> Result<Value, FunctionError> {
    let content = match answer {
        Value::Object(mut object) => object.remove("value"),
        _ => None,
    };
    let Some(Value::String(content)) = content else {
        return Err(failure(
            "java.class_content.invalid_result",
            "Java class source is unavailable",
        ));
    };
    if content.trim().is_empty() {
        return Err(failure(
            "java.class_content.empty_result",
            "Java class source is unavailable",
        ));
    }
    if content.len() > MAX_CONTENT_BYTES {
        return Err(failure(
            "java.class_content.too_large",
            "Java class source exceeds the content limit",
        ));
    }
    Ok(Value::String(content))
}

/// Reuse JDT's own decompiler and source attachment resolution. Keep the exact
/// opaque URI, including its query encoding, and hold no text cache or files.
pub fn read(
    settings: &SettingsSession,
    params: Value,
    context: &Context,
) -> Result<Value, FunctionError> {
    context.check_cancelled()?;
    let uri = validated_uri(&params)?;
    let workspace = context.workspace_context().ok_or_else(|| {
        failure(
            "java.class_content.context_missing",
            "A workspace context is required",
        )
    })?;
    let generation = settings
        .running_server_generation(workspace)
        .ok_or_else(|| {
            failure(
                "java.class_content.unavailable",
                "The Java language server is unavailable",
            )
        })?;
    let answer=context.host().request(REQUEST_METHOD,json!({"context":workspace.to_value(),"serverID":SERVER_ID,"method":CONTENT_METHOD,"params":{"uri":uri},"timeoutMilliseconds":CONTENT_TIMEOUT_MS}));
    context.check_cancelled()?;
    let answer = answer.map_err(|_| {
        failure(
            "java.class_content.unavailable",
            "Java class source is unavailable",
        )
    })?;
    if settings.running_server_generation(workspace) != Some(generation) {
        return Err(failure(
            "java.class_content.unavailable",
            "The Java language server changed during the request",
        ));
    }
    content_result(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn class_identity_is_validated_but_never_reencoded() {
        let uri = "jdt://contents/lib.jar/pkg/Type.java?=project/lib%5C/lib.jar%3Cpkg(Type.class";
        let input = json!({"uri":uri});
        assert_eq!(validated_uri(&input).unwrap(), uri);
        for input in [
            json!({"uri":"file:///private.java"}),
            json!({"uri":"https://example.com"}),
            json!({"uri":"jdt://contents/a\n"}),
            json!({"uri":uri,"extra":true}),
            json!({"uri":"jdt:"}),
            json!({"uri":format!("jdt://contents/{}","x".repeat(MAX_URI_BYTES))}),
        ] {
            assert!(validated_uri(&input).is_err());
        }
    }
    #[test]
    fn source_result_requires_bounded_text() {
        assert_eq!(
            content_result(json!({"value":"class Type {}"})).unwrap(),
            json!("class Type {}")
        );
        assert!(content_result(json!({"value":null})).is_err());
        assert!(content_result(json!({"value":""})).is_err());
        assert!(content_result(json!({"value":"x".repeat(MAX_CONTENT_BYTES+1)})).is_err());
    }
}
