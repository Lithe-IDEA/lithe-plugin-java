//! Entry point of the Java Support plugin backend.
//!
//! Stdout carries only protocol frames, and the backend writes nothing to
//! stderr: diagnostics go through the host's `litheLogging` channel.
//!
//! Building the backend only fills the SDK's in-memory function table; no
//! message is sent until the host's `initialize`. It fails only when a
//! function ID constant is invalid, repeated or over the SDK's limit, which
//! is fixed at compile time and rejected by the unit test
//! `the_backend_builds_with_every_command_registered`. A build that passed
//! the tests therefore never takes the error branch; if one did, the process
//! exits with status 1 before the handshake and the host records the failed
//! start itself.

fn main() {
    let Ok(backend) = lithe_java_backend::backend() else {
        std::process::exit(1);
    };
    std::process::exit(backend.listen());
}
