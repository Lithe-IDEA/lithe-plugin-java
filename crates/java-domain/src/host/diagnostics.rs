//! The only exit through which the domain reports handled failures to the
//! plugin's unified host log.
//!
//! The domain crate does not depend on the plugin SDK, so it cannot call the
//! host logger itself. Instead the backend installs a [`Reporter`] around each
//! command it executes ([`scoped`]), and domain code reports through
//! [`degraded`] or [`cleanup_failed`]. The reporter forwards the fact to the
//! host's `degraded(operation, code, count)` log entry.
//!
//! Only a controlled category code and a count are accepted: never a message,
//! a path, an error body or any user content. The host log only takes codes
//! made of ASCII letters, digits and `._:/-` (at most 64 bytes);
//! [`is_category`] checks the same shape.
//!
//! Without a reporter (unit tests, or code that does not run on the backend's
//! command-execution thread) the fact is dropped. There is no fallback to
//! standard error: the plugin has no free-text log channel.

use std::cell::RefCell;
use std::rc::Rc;

/// Receives one handled-failure fact: a controlled category code and a count.
pub type Reporter = Rc<dyn Fn(&str, u64)>;

/// Longest category code, in bytes, that the host log accepts.
const MAX_CATEGORY_BYTES: usize = 64;

thread_local! {
    /// Reporter of the command currently executing on this thread, if any.
    static REPORTER: RefCell<Option<Reporter>> = const { RefCell::new(None) };
}

/// Puts the previous reporter back when a [`scoped`] call ends, including when
/// its body panics.
struct RestorePrevious(Option<Reporter>);

impl Drop for RestorePrevious {
    fn drop(&mut self) {
        let previous = self.0.take();
        // The reporter being replaced is dropped after the borrow has ended,
        // and a thread that is already shutting down has nothing to restore.
        let _ = REPORTER.try_with(|slot| slot.replace(previous));
    }
}

/// Runs `body` with `reporter` receiving the facts the domain reports on this
/// thread, and restores the previous reporter afterwards, also when `body` panics.
pub fn scoped<R>(reporter: Reporter, body: impl FnOnce() -> R) -> R {
    let previous = REPORTER.with(|slot| slot.replace(Some(reporter)));
    let _restore = RestorePrevious(previous);
    body()
}

/// Reports a handled degradation to the current thread's reporter; does nothing without one.
///
/// `code` must be a controlled category (see [`is_category`]) and never carry
/// a path, an error body or user content.
pub fn degraded(code: &str, count: u64) {
    // The reporter is cloned out first so the cell is not borrowed while it
    // runs: a reporter may itself enter a `scoped` call.
    let reporter = REPORTER
        .try_with(|slot| slot.borrow().clone())
        .ok()
        .flatten();
    if let Some(reporter) = reporter {
        reporter(code, count);
    }
}

/// Reports `count` failed removals of the cleanup `stage`.
/// The category is `storage.cleanup_failed.<stage>`.
pub fn cleanup_failed(stage: &str, count: u64) {
    degraded(&format!("storage.cleanup_failed.{stage}"), count);
}

/// Whether `code` has the shape of a log category: non-empty, at most 64
/// bytes, and made only of ASCII letters, digits and `._:/-`.
pub fn is_category(code: &str) -> bool {
    !code.is_empty() && code.len() <= MAX_CATEGORY_BYTES && code.bytes().all(is_category_byte)
}

/// One byte a category may contain: an ASCII letter, a digit or one of `._:/-`.
fn is_category_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    type Seen = Rc<RefCell<Vec<(String, u64)>>>;

    /// A reporter that records every fact it receives, and the record.
    fn recorder() -> (Reporter, Seen) {
        let seen: Seen = Rc::default();
        let sink = Rc::clone(&seen);
        let reporter: Reporter = Rc::new(move |code: &str, count: u64| {
            sink.borrow_mut().push((code.to_string(), count));
        });
        (reporter, seen)
    }

    fn facts(seen: &Seen) -> Vec<(String, u64)> {
        seen.borrow().clone()
    }

    #[test]
    fn reporting_without_a_reporter_does_nothing() {
        degraded("storage.cleanup_failed.jdt-settings", 1);
        cleanup_failed("jdt-settings", 1);
    }

    #[test]
    fn a_scope_receives_facts_and_ends_with_no_reporter() {
        let (reporter, seen) = recorder();
        let result = scoped(reporter, || {
            degraded("example.category", 3);
            "body result"
        });
        assert_eq!(result, "body result");
        assert_eq!(facts(&seen), vec![("example.category".to_string(), 3)]);

        degraded("example.after", 1);
        assert_eq!(facts(&seen).len(), 1);
    }

    #[test]
    fn an_inner_scope_gives_the_outer_reporter_back() {
        let (outer, outer_seen) = recorder();
        let (inner, inner_seen) = recorder();
        scoped(outer, || {
            degraded("example.first", 1);
            scoped(inner, || degraded("example.inner", 2));
            degraded("example.second", 3);
        });
        assert_eq!(
            facts(&outer_seen),
            vec![
                ("example.first".to_string(), 1),
                ("example.second".to_string(), 3)
            ]
        );
        assert_eq!(facts(&inner_seen), vec![("example.inner".to_string(), 2)]);
    }

    #[test]
    fn a_panicking_body_still_restores_the_previous_reporter() {
        let (outer, outer_seen) = recorder();
        let (inner, inner_seen) = recorder();
        scoped(outer, || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                scoped(inner, || -> u8 { panic!("reporter scope test") })
            }));
            assert!(result.is_err());
            degraded("example.after_panic", 1);
        });
        assert_eq!(
            facts(&outer_seen),
            vec![("example.after_panic".to_string(), 1)]
        );
        assert!(facts(&inner_seen).is_empty());

        // Without an outer reporter the panic leaves none behind either.
        let (lone, lone_seen) = recorder();
        let result = catch_unwind(AssertUnwindSafe(|| {
            scoped(lone, || -> u8 { panic!("reporter scope test") })
        }));
        assert!(result.is_err());
        degraded("example.after_panic", 1);
        assert!(facts(&lone_seen).is_empty());
    }

    #[test]
    fn a_reporter_may_enter_a_scope_while_it_runs() {
        let (inner, inner_seen) = recorder();
        let outer: Reporter = Rc::new(move |_code: &str, _count: u64| {
            scoped(Rc::clone(&inner), || degraded("example.nested", 1));
        });
        scoped(outer, || degraded("example.trigger", 1));
        assert_eq!(facts(&inner_seen), vec![("example.nested".to_string(), 1)]);
    }

    #[test]
    fn cleanup_failures_use_the_stage_in_the_category() {
        let (reporter, seen) = recorder();
        scoped(reporter, || cleanup_failed("jdt-configuration", 2));
        assert_eq!(
            facts(&seen),
            vec![("storage.cleanup_failed.jdt-configuration".to_string(), 2)]
        );
    }

    #[test]
    fn every_cleanup_stage_in_use_makes_a_valid_category() {
        let (reporter, seen) = recorder();
        scoped(reporter, || {
            for stage in ["jdt-configuration", "jdt-settings", "java-argfile-remove"] {
                cleanup_failed(stage, 1);
            }
        });
        let seen = facts(&seen);
        assert_eq!(seen.len(), 3);
        for (code, _) in seen {
            assert!(is_category(&code), "{code} is not a valid category");
        }
    }

    #[test]
    fn categories_have_a_controlled_shape() {
        assert!(is_category("storage.cleanup_failed.jdt-settings"));
        assert!(is_category("a.b_c:d/e-F9"));
        assert!(is_category(&"a".repeat(64)));

        assert!(!is_category(""));
        assert!(!is_category(&"a".repeat(65)));
        assert!(!is_category("has space"));
        assert!(!is_category("back\\slash"));
        assert!(!is_category("中文类别"));
        assert!(!is_category("line\nbreak"));
    }
}
