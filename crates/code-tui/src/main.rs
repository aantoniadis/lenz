#![forbid(unsafe_code)]

mod app;
mod terminal;
mod render;
mod runtime;
mod source;

#[cfg(all(test, feature = "gui-test"))]
mod parity_tests;
#[cfg(all(test, feature = "gui-test"))]
mod foundation_parity_tests;

fn main() -> std::process::ExitCode {
    runtime::main()
}

#[cfg(test)]
mod explorer_tests;
#[cfg(test)]
mod results_bench;
