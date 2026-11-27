//! The DNS-01 challenge (RFC 8555 § 8.4) with deSEC as the DNS provider
//! (`ROEDY_ACME_CHALLENGE=dns-01`, decided on 2026-11-27): the CA looks up
//! a TXT record at `_acme-challenge.<host>` instead of connecting to the
//! hub, so neither port 80 nor 443 has to be reachable for validation.
//!
//! One validation:
//!
//! 1. [`Dns01::prepare`] finds the deSEC domain that owns the name
//!    ([`desec`]) and the addresses of its name servers (its `NS` records).
//! 2. [`Dns01::add`] adds the value to the TXT record set (TTL: the set's
//!    own, the domain's minimum for a new set).
//! 3. [`Dns01::wait`] asks every name server ([`dns`], no recursion) until
//!    each serves the value authoritatively – only then may the CA look – or
//!    gives up after [`Settings::propagation_timeout`].
//! 4. After the CA's validation, passed or failed, and after any error
//!    since step 2, [`Dns01::remove`] takes the value out again
//!    (`tls::issuer`).

use std::{collections::HashMap, net::SocketAddr, time::Duration};

use anyhow::{Context as _, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest as _, Sha256};
use url::Url;

use super::{
    desec::{self, Desec, Zone},
    dns,
};
use crate::config::Secret;

/// The label in front of the domain (§ 8.4).
pub const LABEL: &str = "_acme-challenge";
/// How long the name servers may take to serve a new record.
pub const PROPAGATION_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// Waiting between two rounds of questions to the name servers.
pub const POLL: Duration = Duration::from_secs(5);
/// How long one name server may take to answer.
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(3);

/// Where the hub puts the record.
#[derive(Clone)]
pub enum Provider {
    /// deSEC's API at `api` with `token`.
    Desec { api: Url, token: Secret },
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // `Secret` hides itself as well; the token is left out entirely.
            Self::Desec { api, .. } => f
                .debug_struct("Desec")
                .field("api", &api.as_str())
                .finish_non_exhaustive(),
        }
    }
}

/// What the challenge works with.
#[derive(Debug, Clone)]
pub struct Settings {
    pub provider: Provider,
    pub propagation_timeout: Duration,
    pub poll: Duration,
    pub query_timeout: Duration,
    /// Addresses of name servers by name (lower case, without the final
    /// dot), instead of looking them up (tests).
    pub name_servers: HashMap<String, Vec<SocketAddr>>,
}

impl Settings {
    /// deSEC with its public API and the default waits.
    pub fn desec(token: Secret) -> Self {
        Self {
            provider: Provider::Desec {
                api: Url::parse(desec::API).unwrap_or_else(|_| unreachable!()),
                token,
            },
            propagation_timeout: PROPAGATION_TIMEOUT,
            poll: POLL,
            query_timeout: QUERY_TIMEOUT,
            name_servers: HashMap::new(),
        }
    }
}

/// The TXT value for a key authorization (§ 8.4): its SHA-256 in base64url.
pub fn txt_value(key_authorization: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(key_authorization.as_bytes()))
}

/// A name server of the zone.
#[derive(Debug, Clone)]
pub struct NameServer {
    pub name: String,
    pub addrs: Vec<SocketAddr>,
}

/// Where the record of one validation goes.
#[derive(Debug, Clone)]
pub struct Target {
    /// `_acme-challenge.<domain>`.
    pub name: String,
    pub zone: Zone,
    /// [`Self::name`] within the zone.
    pub subname: String,
    pub name_servers: Vec<NameServer>,
}

pub struct Dns01 {
    settings: Settings,
    desec: Desec,
}

impl Dns01 {
    pub fn new(settings: &Settings, http: reqwest::Client) -> anyhow::Result<Self> {
        let Provider::Desec { api, token } = &settings.provider;
        Ok(Self {
            desec: Desec::new(http, api, token)?,
            settings: settings.clone(),
        })
    }

    /// The zone of `_acme-challenge.<domain>` and its name servers.
    pub async fn prepare(&self, domain: &str) -> anyhow::Result<Target> {
        let name = format!("{LABEL}.{domain}");
        let zone = self.desec.zone_for(&name).await?;
        let subname = zone
            .subname(&name)
            .ok_or_else(|| anyhow!("{name} is not in the deSEC domain {}", zone.name))?;
        let names = self.desec.records(&zone.name, "", "NS").await?;
        if names.is_empty() {
            bail!("the deSEC domain {} has no NS records", zone.name);
        }
        let mut name_servers = Vec::with_capacity(names.len());
        for ns in names {
            let ns = ns.trim_end_matches('.').to_ascii_lowercase();
            let addrs = self.addresses(&ns).await?;
            name_servers.push(NameServer { name: ns, addrs });
        }
        Ok(Target {
            name,
            zone,
            subname,
            name_servers,
        })
    }

    async fn addresses(&self, name_server: &str) -> anyhow::Result<Vec<SocketAddr>> {
        if let Some(addrs) = self.settings.name_servers.get(name_server) {
            return Ok(addrs.clone());
        }
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((name_server, 53))
            .await
            .with_context(|| format!("name server {name_server} not found"))?
            .collect();
        if addrs.is_empty() {
            bail!("name server {name_server} has no address");
        }
        Ok(addrs)
    }

    /// Adds `value` to the TXT record set.
    pub async fn add(&self, target: &Target, value: &str) -> anyhow::Result<()> {
        let quoted = desec::quote(value);
        self.desec
            .modify_txt(&target.zone, &target.subname, |records| {
                (!records.contains(&quoted)).then(|| {
                    let mut records = records.to_vec();
                    records.push(quoted.clone());
                    records
                })
            })
            .await?;
        tracing::info!(
            name = target.name,
            zone = target.zone.name,
            "DNS-01: TXT record set"
        );
        Ok(())
    }

    /// Takes `value` out of the TXT record set; deletes the set if nothing
    /// else is left.
    pub async fn remove(&self, target: &Target, value: &str) -> anyhow::Result<()> {
        let quoted = desec::quote(value);
        let removed = self
            .desec
            .modify_txt(&target.zone, &target.subname, |records| {
                records.contains(&quoted).then(|| {
                    records
                        .iter()
                        .filter(|record| **record != quoted)
                        .cloned()
                        .collect()
                })
            })
            .await?;
        if removed {
            tracing::info!(name = target.name, "DNS-01: TXT record removed");
        }
        Ok(())
    }

    /// Waits until every name server serves `value`. A name server counts
    /// once one of its addresses answered authoritatively and each that
    /// answered has the value (an error answer such as `SERVFAIL` counts as
    /// not having it); addresses that cannot be reached (IPv6 on a host
    /// without it) are left out. No query runs past the deadline.
    pub async fn wait(&self, target: &Target, value: &str) -> anyhow::Result<()> {
        let started = tokio::time::Instant::now();
        let deadline = started + self.settings.propagation_timeout;
        loop {
            let mut missing = Vec::new();
            for ns in &target.name_servers {
                if let Err(reason) = self.serves(ns, &target.name, value, deadline).await {
                    missing.push(format!("{} ({reason})", ns.name));
                }
            }
            if missing.is_empty() {
                tracing::info!(
                    name = target.name,
                    waited = ?started.elapsed(),
                    "DNS-01: TXT record on all name servers"
                );
                return Ok(());
            }
            if tokio::time::Instant::now() + self.settings.poll > deadline {
                bail!(
                    "DNS-01: the TXT record {} is not on all name servers after {:?}: {}",
                    target.name,
                    self.settings.propagation_timeout,
                    missing.join(", ")
                );
            }
            tracing::debug!(
                name = target.name,
                missing = missing.join(", "),
                "DNS-01: waiting for the name servers"
            );
            tokio::time::sleep(self.settings.poll).await;
        }
    }

    /// Whether `ns` serves `value` at `name`; why not otherwise.
    async fn serves(
        &self,
        ns: &NameServer,
        name: &str,
        value: &str,
        deadline: tokio::time::Instant,
    ) -> Result<(), String> {
        let (mut answered, mut lacking, mut unreachable) = (false, None, None);
        for addr in &ns.addrs {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                unreachable.get_or_insert_with(|| "out of time".to_owned());
                break;
            }
            let timeout = self.settings.query_timeout.min(remaining);
            match dns::lookup_txt(*addr, name, timeout).await {
                Ok(response)
                    if response.authoritative
                        && matches!(response.rcode, dns::RCODE_NOERROR | dns::RCODE_NXDOMAIN) =>
                {
                    answered = true;
                    // Only a positive answer counts, whatever an NXDOMAIN
                    // carries.
                    if response.rcode != dns::RCODE_NOERROR
                        || !response.txt.iter().any(|txt| txt == value)
                    {
                        lacking.get_or_insert_with(|| "not there yet".to_owned());
                    }
                }
                Ok(response) if !response.authoritative => {
                    lacking.get_or_insert_with(|| format!("{addr} answered without authority"));
                }
                Ok(response) => {
                    lacking.get_or_insert_with(|| {
                        format!("{addr} answered with code {}", response.rcode)
                    });
                }
                Err(error) => {
                    unreachable.get_or_insert_with(|| format!("{error:#}"));
                }
            }
        }
        match (lacking, answered) {
            (Some(reason), _) => Err(reason),
            (None, true) => Ok(()),
            (None, false) => Err(unreachable.unwrap_or_else(|| "no address".to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn txt_value_is_the_base64url_sha256() {
        // SHA-256("abc") from FIPS 180-2, in base64url without padding.
        assert_eq!(
            txt_value("abc"),
            "ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0"
        );
    }

    #[test]
    fn settings_never_show_the_token() {
        let settings = Settings::desec(Secret::new("s3cr3t-token"));
        let debug = format!("{settings:?}");
        assert!(debug.contains("desec.io"), "{debug}");
        assert!(!debug.contains("s3cr3t"), "{debug}");
    }
}
