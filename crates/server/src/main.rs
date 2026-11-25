//! Binary `roedy-hub`.

use std::process::ExitCode;

use clap::Parser as _;
use roedy_hub_server::cli::{self, Cli};

fn main() -> ExitCode {
    match cli::run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
