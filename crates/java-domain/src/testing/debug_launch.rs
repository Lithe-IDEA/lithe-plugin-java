//! Preparation of a Java test debug launch.
//!
//! Ported from the Swift `JavaTestDebugLaunchService`. Preparation resolves
//! the selected tests through JDT LS, starts the result channel, and builds
//! the generic debug configuration. A result channel that was started is
//! always stopped when a later step fails or the launch is cancelled, so a
//! failed launch never leaves a listening port behind.

use super::jdt_test_items::JavaTestDebugLaunchTarget;
use super::workflow::{JavaTestDebugRequest, StopHandle};
use crate::debug::{java_test_launch, DebugLaunchConfiguration};
use crate::protocol::{CoreError, ErrorCode};

/// Resolves the selected tests into runner arguments.
///
/// TODO(host-api: API-25 language server): production resolution sends the Java
/// Test extension's `vscode.java.test.findTestTypesAndMethods` and
/// `vscode.java.test.junit.argument` commands through the host language-server
/// request channel.
pub trait TargetResolver {
    /// Resolves `request` or explains why it cannot be debugged.
    fn resolve(
        &mut self,
        request: &JavaTestDebugRequest,
    ) -> Result<JavaTestDebugLaunchTarget, CoreError>;
}

/// A started result channel.
pub trait ResultServer: StopHandle {
    /// Starts listening and returns the loopback port.
    fn start(&mut self) -> Result<u16, CoreError>;
}

/// A debug configuration with the result channel it reports to.
#[derive(Debug)]
pub struct PreparedJavaTestDebugLaunch<S: ResultServer> {
    /// Configuration for the generic debugger.
    pub configuration: DebugLaunchConfiguration,
    /// Result channel the runner connects to.
    pub result_server: S,
}

impl<S: ResultServer> PreparedJavaTestDebugLaunch<S> {
    /// Releases the result channel.
    pub fn stop(&mut self) {
        self.result_server.stop();
    }
}

/// Resolves, starts the result channel and configures the debug launch.
///
/// `is_cancelled` is checked after resolution and after the channel started.
pub fn prepare<S: ResultServer>(
    request: &JavaTestDebugRequest,
    resolver: &mut impl TargetResolver,
    make_server: impl FnOnce() -> S,
    is_cancelled: impl Fn() -> bool,
) -> Result<PreparedJavaTestDebugLaunch<S>, CoreError> {
    let target = resolver.resolve(request)?;
    if is_cancelled() {
        return Err(cancelled());
    }
    let mut server = make_server();
    let port = match server.start() {
        Ok(port) => port,
        Err(error) => {
            server.stop();
            return Err(error);
        }
    };
    if is_cancelled() {
        server.stop();
        return Err(cancelled());
    }
    match java_test_launch(target.launch_request(port)) {
        Ok(configuration) => Ok(PreparedJavaTestDebugLaunch {
            configuration,
            result_server: server,
        }),
        Err(error) => {
            server.stop();
            Err(error)
        }
    }
}

fn cancelled() -> CoreError {
    CoreError::new(
        ErrorCode::Cancelled,
        "The Java test debug launch was cancelled.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug::JavaTestFramework;
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::rc::Rc;

    #[derive(Default)]
    struct Resolver {
        requests: Vec<JavaTestDebugRequest>,
        working_directory: String,
    }

    impl TargetResolver for Resolver {
        fn resolve(
            &mut self,
            request: &JavaTestDebugRequest,
        ) -> Result<JavaTestDebugLaunchTarget, CoreError> {
            self.requests.push(request.clone());
            Ok(JavaTestDebugLaunchTarget {
                file_path: request.file_path.display().to_string(),
                name: "UserServiceTest".into(),
                framework: JavaTestFramework::Junit,
                working_directory: self.working_directory.clone(),
                main_class: "org.eclipse.jdt.internal.junit.runner.RemoteTestRunner".into(),
                project_name: Some("service".into()),
                class_paths: vec!["/w/target/classes".into()],
                module_paths: Vec::new(),
                vm_arguments: vec!["-ea".into()],
                program_arguments: vec!["-port".into(), "${resultPort}".into()],
                testng_runner_path: None,
                testng_test_names: Vec::new(),
            })
        }
    }

    #[derive(Debug, Default, Clone)]
    struct Counts {
        starts: Rc<Cell<usize>>,
        stops: Rc<Cell<usize>>,
    }

    #[derive(Debug)]
    struct Server(Counts);

    impl StopHandle for Server {
        fn stop(&mut self) {
            self.0.stops.set(self.0.stops.get() + 1);
        }
    }

    impl ResultServer for Server {
        fn start(&mut self) -> Result<u16, CoreError> {
            self.0.starts.set(self.0.starts.get() + 1);
            Ok(43_128)
        }
    }

    fn request() -> JavaTestDebugRequest {
        JavaTestDebugRequest {
            file_path: PathBuf::from("/w/src/test/java/UserServiceTest.java"),
            test_identifier: None,
        }
    }

    #[test]
    fn file_selection_resolves_the_target_starts_results_and_builds_the_configuration() {
        let counts = Counts::default();
        let mut resolver = Resolver {
            working_directory: "/w".into(),
            ..Default::default()
        };
        let mut prepared = prepare(
            &request(),
            &mut resolver,
            || Server(counts.clone()),
            || false,
        )
        .unwrap();
        assert_eq!(resolver.requests, vec![request()]);
        assert_eq!(counts.starts.get(), 1);
        assert_eq!(counts.stops.get(), 0);
        assert_eq!(prepared.configuration.name, "UserServiceTest");
        let encoded = serde_json::to_string(&prepared.configuration.arguments).unwrap();
        assert!(encoded.contains("43128"), "{encoded}");
        prepared.stop();
        assert_eq!(counts.stops.get(), 1);
    }

    #[test]
    fn a_configuration_failure_stops_the_result_server() {
        let counts = Counts::default();
        let mut resolver = Resolver::default();
        let error = prepare(
            &request(),
            &mut resolver,
            || Server(counts.clone()),
            || false,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert_eq!((counts.starts.get(), counts.stops.get()), (1, 1));
    }

    #[test]
    fn cancellation_after_start_stops_the_result_server() {
        let counts = Counts::default();
        let mut resolver = Resolver {
            working_directory: "/w".into(),
            ..Default::default()
        };
        let checks = Cell::new(0);
        let error = prepare(
            &request(),
            &mut resolver,
            || Server(counts.clone()),
            || {
                checks.set(checks.get() + 1);
                checks.get() == 2
            },
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::Cancelled);
        assert_eq!((counts.starts.get(), counts.stops.get()), (1, 1));
    }

    #[test]
    fn cancellation_before_start_never_creates_a_server() {
        let mut resolver = Resolver::default();
        let error = prepare(
            &request(),
            &mut resolver,
            || -> Server { unreachable!("cancelled before the server") },
            || true,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::Cancelled);
    }
}
