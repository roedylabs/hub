//! Subcommands of the `roedy-hub` binary. For every command, the
//! configuration comes from the `ROEDY_*` environment variables
//! (docs/ops.md › Configuration).
//!
//! Taken over from Roedy Mail (`crates/server/src/cli.rs`) without
//! `reset-admin`, which follows with the users.

use anyhow::bail;
use clap::{Parser, Subcommand};

use crate::{app, config::Config, db, healthcheck, logging, setup_token};

#[derive(Debug, Parser)]
#[command(
    name = "roedy-hub",
    version,
    about = "Roedy Hub – gateway, sign-in and home page for the Roedy apps",
    long_about = "Roedy Hub – gateway, sign-in and home page for the Roedy apps.\n\n\
                  Configuration via environment variables: ROEDY_PUBLIC_URL, ROEDY_LISTEN, \
                  ROEDY_INTERNAL_LISTEN, ROEDY_DATA_DIR, ROEDY_LOG, ROEDY_LOG_FORMAT, \
                  ROEDY_WEB_DIR, ROEDY_TRUST_PROXY, ROEDY_SECRET_KEY, ROEDY_SECRET_KEY_FILE."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start the HTTP server
    Serve,
    /// Check whether the running server responds (exit code 0 = healthy)
    Healthcheck,
    /// Apply pending database migrations
    Migrate,
    /// Show the setup token for the initial setup
    SetupToken,
}

pub async fn run(cli: Cli) -> anyhow::Result<()> {
    let config = Config::load()?;
    logging::init(&config)?;
    match cli.command {
        Command::Serve => app::serve(config).await,
        Command::Healthcheck => {
            healthcheck::check(config.listen, healthcheck::DEFAULT_TIMEOUT).await?;
            println!("ok");
            Ok(())
        }
        Command::Migrate => {
            let version = migrate(config).await?;
            println!("Database schema: version {version}");
            Ok(())
        }
        Command::SetupToken => match setup_token(config).await? {
            Some(token) => {
                println!("Setup-Token: {token}");
                Ok(())
            }
            None => bail!("Setup is complete; there is no setup token anymore."),
        },
    }
}

/// `migrate`: opens (and creates) the database, which applies pending
/// migrations; returns the schema version.
async fn migrate(config: Config) -> anyhow::Result<i64> {
    db::blocking(move || {
        let conn = db::open(&config.data_dir)?;
        Ok(db::migrations::current_version(&conn)?)
    })
    .await
}

/// `setup-token`: the token while no administrator exists. Needs the
/// server's database and never creates one.
async fn setup_token(config: Config) -> anyhow::Result<Option<String>> {
    db::blocking(move || setup_token::ensure(&db::open_existing(&config.data_dir)?)).await
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
    fn parses_all_subcommands() {
        for name in ["serve", "healthcheck", "migrate", "setup-token"] {
            Cli::try_parse_from(["roedy-hub", name]).unwrap();
        }
        assert!(Cli::try_parse_from(["roedy-hub"]).is_err());
        assert!(Cli::try_parse_from(["roedy-hub", "reset-admin"]).is_err());
    }

    fn config(data_dir: &std::path::Path) -> Config {
        Config {
            data_dir: data_dir.to_path_buf(),
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn migrate_creates_the_database() {
        let tmp = tempfile::tempdir().unwrap();
        let version = migrate(config(&tmp.path().join("data"))).await.unwrap();
        assert_eq!(version, db::migrations::latest_version());
        assert!(db::db_path(&tmp.path().join("data")).is_file());
    }

    #[tokio::test]
    async fn setup_token_needs_an_existing_database() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let err = setup_token(config(&data_dir)).await.unwrap_err();
        assert!(err.to_string().contains("ROEDY_DATA_DIR"), "{err}");
        assert!(!data_dir.exists(), "creates nothing");

        migrate(config(&data_dir)).await.unwrap();
        let first = setup_token(config(&data_dir)).await.unwrap().unwrap();
        let again = setup_token(config(&data_dir)).await.unwrap();
        assert_eq!(again.as_deref(), Some(first.as_str()));
    }
}
