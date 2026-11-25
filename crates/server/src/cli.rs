//! Command line of the `roedy-hub` binary. The configuration comes from the
//! `ROEDY_*` environment variables (docs/ops.md › Configuration); the
//! commands (`serve`, `healthcheck`, `migrate`, `setup-token`, …) follow with
//! the server base.

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "roedy-hub",
    version,
    about = "Roedy Hub – gateway, sign-in and home page for the Roedy apps"
)]
pub struct Cli {}

/// Runs the parsed command line. Without commands yet, `--help` and
/// `--version` are all there is; clap answers them while parsing.
pub fn run(_cli: Cli) -> Result<(), String> {
    Err("no command given – see `roedy-hub --help`".to_owned())
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory as _, error::ErrorKind};

    use super::*;

    #[test]
    fn command_line_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn version_shows_the_package_version() {
        let err = Cli::try_parse_from(["roedy-hub", "--version"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
        assert_eq!(
            err.to_string().trim(),
            format!("roedy-hub {}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn without_a_command_it_fails() {
        let cli = Cli::try_parse_from(["roedy-hub"]).unwrap();
        assert!(run(cli).is_err());
    }
}
