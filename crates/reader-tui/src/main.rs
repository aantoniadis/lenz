#![forbid(unsafe_code)]

mod app;
mod render;
mod runtime;
mod source;

#[cfg(all(test, feature = "gui-test"))]
mod parity_tests;

fn main() -> std::process::ExitCode {
    runtime::main()
}
