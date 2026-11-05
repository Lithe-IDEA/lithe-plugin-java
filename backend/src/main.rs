//! Entry point of the Java Support plugin backend.
//!
//! Stdout carries only protocol frames. Diagnostics are sent through the
//! host's `litheLogging` channel, never printed. Stderr gets a single fixed
//! line with the SDK error code, and only when registering the functions
//! fails before the protocol starts.

fn main() {
    let backend = match lithe_java_backend::backend() {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!(
                "lithe-java-backend: could not register functions ({})",
                error.code()
            );
            std::process::exit(1);
        }
    };
    std::process::exit(backend.listen());
}
