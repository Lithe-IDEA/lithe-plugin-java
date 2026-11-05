//! Java test discovery, debug launch and result-channel rules.
//!
//! These modules replace the Java branches of the Swift host's test workflow:
//! [`jdt_test_items`] interprets the Java Test extension's answers,
//! [`workflow`] decides which operation is current, [`debug_launch`] prepares
//! a debug session and [`result_channel`] fixes the result listener's
//! lifecycle.

pub mod debug_launch;
pub mod jdt_test_items;
pub mod result_channel;
pub mod workflow;
