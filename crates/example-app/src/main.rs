//! `roedy-hub-example`: runs the example app.
//!
//! Configuration from the environment:
//!
//! - `ROEDY_PUBLIC_URL` (required): `<hub public URL>/<slug>`.
//! - `ROEDY_HUB_INTERNAL_URL`: the hub's internal listener, default
//!   `http://roedy-hub:8081`.
//! - `ROEDY_LISTEN`: default `0.0.0.0:8080`.
//! - `ROEDY_PAIRING_CODE`: pairs at start with this code. The example keeps
//!   its connection in memory only, so every start needs a new code.

use std::{net::SocketAddr, time::Duration};

use anyhow::Context as _;
use roedy_hub_example::Example;

/// The JWKS is refreshed at least this often (contract § 4.4).
const JWKS_EVERY: Duration = Duration::from_secs(60 * 60);

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let public_url = env("ROEDY_PUBLIC_URL").context("ROEDY_PUBLIC_URL is not set")?;
    let hub = env("ROEDY_HUB_INTERNAL_URL").unwrap_or_else(|| "http://roedy-hub:8081".into());
    let listen: SocketAddr = env("ROEDY_LISTEN")
        .unwrap_or_else(|| "0.0.0.0:8080".into())
        .parse()
        .context("ROEDY_LISTEN is not an address")?;
    let example = Example::new(&public_url, &hub)?;
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("address {listen} not available"))?;
    eprintln!(
        "example app on {listen}, public URL {}",
        example.public_url()
    );

    if let Some(code) = env("ROEDY_PAIRING_CODE") {
        let paired = example.pair(&code).await.context("pairing failed")?;
        eprintln!("paired as {} with the hub {}", paired.slug, paired.issuer);
    } else {
        eprintln!("not paired: set ROEDY_PAIRING_CODE to a code from the hub's administration");
    }

    let refresher = example.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(JWKS_EVERY).await;
            if refresher.connection().is_some()
                && let Err(error) = refresher.refresh_jwks().await
            {
                eprintln!("JWKS not refreshed: {error}");
            }
        }
    });

    roedy_hub_example::serve(listener, &example, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}
