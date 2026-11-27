//! Subcommands of the `roedy-hub` binary. For every command, the
//! configuration comes from the `ROEDY_*` environment variables
//! (docs/ops.md › Configuration).
//!
//! Taken over from Roedy Mail (`crates/server/src/cli.rs`), `reset-admin`
//! with an optional reset link instead of a password.

use anyhow::bail;
use clap::{Parser, Subcommand};

use crate::{
    app,
    auth::{password::Hasher, session::Token},
    config::Config,
    db, healthcheck, logging,
    reset_admin::{self, Reset, Secret},
    setup_token,
};

#[derive(Debug, Parser)]
#[command(
    name = "roedy-hub",
    version,
    about = "Roedy Hub – gateway, sign-in and home page for the Roedy apps",
    long_about = "Roedy Hub – gateway, sign-in and home page for the Roedy apps.\n\n\
                  Configuration via environment variables: ROEDY_PUBLIC_URL, ROEDY_LISTEN, \
                  ROEDY_INTERNAL_LISTEN, ROEDY_DATA_DIR, ROEDY_LOG, ROEDY_LOG_FORMAT, \
                  ROEDY_WEB_DIR, ROEDY_TRUST_PROXY, ROEDY_SECRET_KEY, ROEDY_SECRET_KEY_FILE, \
                  ROEDY_TLS, ROEDY_HTTP_LISTEN, ROEDY_ACME_DIRECTORY, ROEDY_ACME_EMAIL, \
                  ROEDY_ACME_AGREE_TOS, ROEDY_ACME_CHALLENGE, ROEDY_ACME_DNS_PROVIDER, \
                  ROEDY_DESEC_TOKEN, ROEDY_DESEC_TOKEN_FILE, ROEDY_TLS_CERT_FILE, \
                  ROEDY_TLS_KEY_FILE."
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
    /// Restore an administrator's access: new password (or reset link),
    /// authenticator apps removed, signed out, unlocked
    ResetAdmin {
        /// Email address or username; required when there are several
        /// administrators
        #[arg(long, value_name = "EMAIL|USERNAME")]
        user: Option<String>,
        /// Print a password reset link (valid for an hour) instead of a new
        /// password; needs ROEDY_PUBLIC_URL
        #[arg(long)]
        link: bool,
    },
    /// Show the setup token for the initial setup
    SetupToken,
}

pub async fn run(cli: Cli) -> anyhow::Result<()> {
    let config = Config::load()?;
    logging::init(&config)?;
    match cli.command {
        Command::Serve => app::serve(config).await,
        Command::Healthcheck => {
            match config.tls_host().filter(|_| config.tls.enabled()) {
                Some(host) => {
                    healthcheck::check_tls(config.listen, &host, healthcheck::DEFAULT_TIMEOUT)
                        .await?;
                }
                None => healthcheck::check(config.listen, healthcheck::DEFAULT_TIMEOUT).await?,
            }
            println!("ok");
            Ok(())
        }
        Command::Migrate => {
            let version = migrate(config).await?;
            println!("Database schema: version {version}");
            Ok(())
        }
        Command::ResetAdmin { user, link } => {
            let reset = reset_admin(config, user, link).await?;
            print!("{}", describe(&reset));
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

/// `reset-admin`: selects the administrator, then resets in one
/// transaction. With `link` the output is a link to the hub's address
/// instead of the password.
async fn reset_admin(config: Config, user: Option<String>, link: bool) -> anyhow::Result<Reset> {
    // Before any change: without the address there is no link.
    let public_url = if link {
        Some(config.require_public_url()?)
    } else {
        None
    };
    let data_dir = config.data_dir.clone();
    let admin =
        db::blocking(move || reset_admin::select(&db::open_existing(&data_dir)?, user.as_deref()))
            .await?;
    let password = reset_admin::generate_password()?;
    let hash = Hasher::default().hash(&password).await?;
    let secret = if link {
        Secret::Link(Token::generate()?)
    } else {
        Secret::Password(password)
    };
    let mut reset = db::blocking(move || {
        let mut conn = db::open_existing(&config.data_dir)?;
        reset_admin::reset(&mut conn, &admin, secret, &hash)
    })
    .await?;
    if let (Secret::Link(token), Some(url)) = (&reset.secret, public_url) {
        reset.link = Some(format!(
            "{}/login/reset/{}",
            url.origin().ascii_serialization(),
            token.as_str()
        ));
    }
    Ok(reset)
}

/// The output of `reset-admin`.
fn describe(reset: &Reset) -> String {
    let mut out = format!(
        "Access of {} ({}) reset.\n",
        reset.display_name, reset.email
    );
    match (&reset.secret, &reset.link) {
        (Secret::Password(password), _) => out.push_str(&format!("New password: {password}\n")),
        (Secret::Link(_), Some(link)) => {
            out.push_str(&format!("Set a new password within an hour: {link}\n"))
        }
        (Secret::Link(_), None) => {}
    }
    out.push_str(&format!(
        "Authenticator apps removed: {}, sessions ended: {}{}.\n",
        reset.devices,
        reset.sessions,
        if reset.unlocked {
            ", account unlocked"
        } else {
            ""
        }
    ));
    out
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
        for name in [
            "serve",
            "healthcheck",
            "migrate",
            "reset-admin",
            "setup-token",
        ] {
            Cli::try_parse_from(["roedy-hub", name]).unwrap();
        }
        let cli =
            Cli::try_parse_from(["roedy-hub", "reset-admin", "--user", "max", "--link"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::ResetAdmin { user: Some(u), link: true } if u == "max"
        ));
        assert!(Cli::try_parse_from(["roedy-hub"]).is_err());
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

    /// `reset-admin` against a real data directory: the administrator gets
    /// back in with the printed password, or with the link.
    #[tokio::test]
    async fn reset_admin_prints_a_password_or_a_link() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let err = reset_admin(config(&data_dir), None, false)
            .await
            .map(drop)
            .unwrap_err();
        assert!(err.to_string().contains("ROEDY_DATA_DIR"), "{err}");
        migrate(config(&data_dir)).await.unwrap();
        let err = reset_admin(config(&data_dir), None, false)
            .await
            .map(drop)
            .unwrap_err();
        assert!(err.to_string().contains("setup-token"), "{err}");

        let dir = data_dir.clone();
        db::blocking(move || {
            let conn = db::open_existing(&dir)?;
            let id = crate::db::repo::users::tests::add(
                &conn,
                "max",
                "max@example.com",
                crate::db::repo::users::Role::Admin,
            );
            conn.execute("UPDATE users SET status = 'locked' WHERE id = ?1", [id])?;
            Ok(())
        })
        .await
        .unwrap();

        let reset = reset_admin(config(&data_dir), None, false).await.unwrap();
        let Secret::Password(password) = &reset.secret else {
            panic!("no password");
        };
        // The messages never show the output: it carries the password.
        let out = describe(&reset);
        assert!(out.contains(&format!("New password: {password}")));
        assert!(out.contains("account unlocked"));
        let dir = data_dir.clone();
        let hash = db::blocking(move || {
            let conn = db::open_existing(&dir)?;
            Ok(crate::db::repo::users::find_for_login(&conn, "max")?
                .unwrap()
                .password_hash)
        })
        .await
        .unwrap();
        assert!(
            Hasher::default()
                .verify(password, Some(&hash))
                .await
                .unwrap()
        );

        // A link needs the public address.
        assert!(
            reset_admin(config(&data_dir), Some("max".into()), true)
                .await
                .is_err()
        );
        let with_url = Config {
            public_url: Some("https://hub.example.com/".parse().unwrap()),
            ..config(&data_dir)
        };
        let reset = reset_admin(with_url, Some("max".into()), true)
            .await
            .unwrap();
        let out = describe(&reset);
        assert!(
            out.contains("Set a new password within an hour: https://hub.example.com/login/reset/"),
            "{out}"
        );
        assert!(!out.contains("New password"), "{out}");
        assert!(!out.contains("account unlocked"), "{out}");
    }
}
