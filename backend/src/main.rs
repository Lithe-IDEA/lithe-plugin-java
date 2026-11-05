//! Entry point of the Java Support plugin backend.
//!
//! Stdout carries only protocol frames; diagnostics go to stderr.

fn main() {
    let backend = match lithe_java_backend::backend() {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("lithe-java-backend: could not register functions: {error}");
            std::process::exit(1);
        }
    };
    std::process::exit(backend.listen());
}
