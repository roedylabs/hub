//! Security notices by mail: "New sign-in", "Password changed",
//! authenticator app added or removed, recovery code used and new recovery
//! codes. They always go out – a hijacked session must not be able to switch
//! the warning off.
//!
//! Callers report an event in the transaction of the change ([`security`]);
//! the mail is collected in an [`Outbox`] and sent after the commit with
//! [`Outbox::send`] – in the background, so the response does not wait for
//! the mail server. Without a sending account the mail is dropped (only in
//! the log); a sending error does not change the event. The notices of the
//! email change (`security_email`, `email_confirm`) are sent by its flow
//! itself (`api::email`), before the link is valid.
//!
//! Taken over from Roedy Mail (`crates/server/src/events/mod.rs` and
//! `events/mail.rs`) and simplified: no notifications in the app, no bell, no
//! push and no digests – only the mails.

use std::net::IpAddr;

use jiff::Timestamp;
use rusqlite::Connection;

use crate::{
    app::AppState,
    auth::{AuthUser, device::Device, session::Origin},
    db::repo::users::{self, Profile},
    mailer::{
        Mail,
        delivery::{Delivery, Unavailable},
        format::Format,
        templates::{
            Context, Place, SecurityLogin, SecurityPassword, SecurityRecoveryCodes,
            SecurityRecoveryUsed, SecurityTotpAdded, SecurityTotpRemoved,
        },
    },
    settings,
};

/// Recipient of a notice.
#[derive(Debug, Clone, Copy)]
pub struct Recipient<'a> {
    pub id: &'a str,
    pub email: &'a str,
    pub name: &'a str,
}

impl<'a> From<&'a AuthUser> for Recipient<'a> {
    fn from(user: &'a AuthUser) -> Self {
        Self {
            id: &user.id,
            email: &user.email,
            name: &user.display_name,
        }
    }
}

impl<'a> From<&'a Profile> for Recipient<'a> {
    fn from(user: &'a Profile) -> Self {
        Self {
            id: &user.id,
            email: &user.email,
            name: &user.display_name,
        }
    }
}

/// Where a session is: device and IP address (shortened in the mail).
#[derive(Debug, Clone, Copy)]
pub struct Whereabouts {
    pub device: Device,
    pub ip: IpAddr,
}

impl From<&Origin> for Whereabouts {
    fn from(origin: &Origin) -> Self {
        Self {
            device: origin.device(),
            ip: origin.ip,
        }
    }
}

/// Security events (to the person affected), each with its own mail.
#[derive(Debug, Clone, Copy)]
pub enum Security<'a> {
    /// "New sign-in" from an unknown device (`security_login`).
    Login { at: Whereabouts },
    /// "Password changed" (`security_password`). `device` is the session that
    /// changed it, or the browser that opened the link from "Forgot password"
    /// (`reset`). Without `reset` the mail advises "Forgot password" and the
    /// administration, the sole administrator `roedy-hub reset-admin`
    /// ([`users::sole_admin`]).
    PasswordChanged { device: Device, reset: bool },
    /// "Authenticator app added" (`security_totp_add`). `first`: multi-factor
    /// is thereby on; the mail's advice then depends on whether the person is
    /// the sole administrator.
    TotpAdded {
        name: &'a str,
        first: bool,
        at: Whereabouts,
    },
    /// "Authenticator app removed" (`security_totp_remove`). `last`:
    /// multi-factor is off, the recovery codes are gone.
    TotpRemoved {
        name: &'a str,
        last: bool,
        at: Whereabouts,
    },
    /// "Recovery code used" (sign-in, `security_recovery_used`). From an
    /// unknown device this is stated here (`new_device`) instead of in a
    /// second mail "New sign-in".
    RecoveryUsed {
        /// Still unused out of `total`.
        remaining: u32,
        total: u32,
        new_device: bool,
        at: Whereabouts,
    },
    /// "New recovery codes generated" (`security_recovery_codes`): the
    /// previous ones are no longer valid.
    RecoveryCodesRenewed { at: Whereabouts },
}

impl Security<'_> {
    /// Whether the mail's advice depends on whether the person is the sole
    /// active administrator: "Password changed" (not after "Reset password")
    /// and the first authenticator app.
    fn depends_on_sole_admin(&self) -> bool {
        matches!(
            self,
            Self::PasswordChanged { reset: false, .. } | Self::TotpAdded { first: true, .. }
        )
    }

    fn mail(&self, at: Timestamp, sole_admin: bool) -> Notice {
        match *self {
            Self::Login { at: seen } => Notice::Login { seen, at },
            Self::PasswordChanged { device, reset } => Notice::Password {
                device,
                reset,
                sole_admin,
                at,
            },
            Self::TotpAdded {
                name,
                first,
                at: seen,
            } => Notice::TotpAdded {
                app: name.to_owned(),
                first,
                sole_admin,
                seen,
                at,
            },
            Self::TotpRemoved {
                name,
                last,
                at: seen,
            } => Notice::TotpRemoved {
                app: name.to_owned(),
                last,
                seen,
                at,
            },
            Self::RecoveryUsed {
                remaining,
                total,
                new_device,
                at: seen,
            } => Notice::RecoveryUsed {
                remaining,
                total,
                new_device,
                seen,
                at,
            },
            Self::RecoveryCodesRenewed { at: seen } => Notice::RecoveryCodes { seen, at },
        }
    }
}

/// Reports a security event to `to` by mail via `outbox`, in their language.
/// In the event's transaction: the advice names `reset-admin` only if nobody
/// else can lock the account.
pub fn security(
    conn: &Connection,
    outbox: &mut Outbox,
    to: Recipient<'_>,
    event: Security<'_>,
) -> anyhow::Result<()> {
    let sole_admin = event.depends_on_sole_admin() && users::sole_admin(conn, to.id)?;
    let mail = event.mail(Timestamp::now(), sole_admin);
    outbox.0.push(Pending {
        to: to.email.to_owned(),
        name: to.name.to_owned(),
        format: settings::format_of(conn, to.id)?,
        mail,
    });
    Ok(())
}

/// Content of a mail for an event.
#[derive(Debug, Clone)]
enum Notice {
    Login {
        seen: Whereabouts,
        at: Timestamp,
    },
    Password {
        device: Device,
        reset: bool,
        sole_admin: bool,
        at: Timestamp,
    },
    TotpAdded {
        app: String,
        first: bool,
        sole_admin: bool,
        seen: Whereabouts,
        at: Timestamp,
    },
    TotpRemoved {
        app: String,
        last: bool,
        seen: Whereabouts,
        at: Timestamp,
    },
    RecoveryUsed {
        remaining: u32,
        total: u32,
        new_device: bool,
        seen: Whereabouts,
        at: Timestamp,
    },
    RecoveryCodes {
        seen: Whereabouts,
        at: Timestamp,
    },
}

#[derive(Debug, Clone)]
struct Pending {
    to: String,
    name: String,
    /// Language and time zone of the person.
    format: Format,
    mail: Notice,
}

impl Pending {
    fn template(&self) -> &'static str {
        match self.mail {
            Notice::Login { .. } => "security_login",
            Notice::Password { .. } => "security_password",
            Notice::TotpAdded { .. } => "security_totp_add",
            Notice::TotpRemoved { .. } => "security_totp_remove",
            Notice::RecoveryUsed { .. } => "security_recovery_used",
            Notice::RecoveryCodes { .. } => "security_recovery_codes",
        }
    }

    fn render(&self, ctx: &Context<'_>) -> anyhow::Result<Mail> {
        let (to, name) = (self.to.as_str(), self.name.as_str());
        let lang = ctx.format.lang();
        let place = |seen: &Whereabouts| Place { ip: seen.ip };
        match &self.mail {
            Notice::Login { seen, at } => SecurityLogin {
                to,
                name,
                device: &seen.device.label(lang),
                place: place(seen),
                at: *at,
            }
            .render(ctx),
            Notice::Password {
                device,
                reset,
                sole_admin,
                at,
            } => SecurityPassword {
                to,
                name,
                device: &device.label(lang),
                reset: *reset,
                sole_admin: *sole_admin,
                at: *at,
            }
            .render(ctx),
            Notice::TotpAdded {
                app,
                first,
                sole_admin,
                seen,
                at,
            } => SecurityTotpAdded {
                to,
                name,
                app,
                first_app: *first,
                sole_admin: *sole_admin,
                device: &seen.device.label(lang),
                place: place(seen),
                at: *at,
            }
            .render(ctx),
            Notice::TotpRemoved {
                app,
                last,
                seen,
                at,
            } => SecurityTotpRemoved {
                to,
                name,
                app,
                last: *last,
                device: &seen.device.label(lang),
                place: place(seen),
                at: *at,
            }
            .render(ctx),
            Notice::RecoveryUsed {
                remaining,
                total,
                new_device,
                seen,
                at,
            } => SecurityRecoveryUsed {
                to,
                name,
                remaining: *remaining,
                total: *total,
                new_device: *new_device,
                device: &seen.device.label(lang),
                place: place(seen),
                at: *at,
            }
            .render(ctx),
            Notice::RecoveryCodes { seen, at } => SecurityRecoveryCodes {
                to,
                name,
                device: &seen.device.label(lang),
                place: place(seen),
                at: *at,
            }
            .render(ctx),
        }
    }
}

/// Mails that go out after the commit. If it is not sent (error, rollback),
/// nothing goes out.
#[derive(Debug, Default)]
#[must_use = "send with Outbox::send after the commit"]
pub struct Outbox(Vec<Pending>);

impl Outbox {
    /// Sends the mails one after the other in the background (the mailer
    /// sends one at a time anyway).
    pub fn send(self, state: &AppState) {
        if self.0.is_empty() {
            return;
        }
        let worker = state.clone();
        state.background.spawn(async move {
            for pending in self.0 {
                if let Err(error) = deliver(&worker, &pending).await {
                    tracing::warn!(template = pending.template(), "notice not sent: {error:#}");
                }
            }
        });
    }

    /// Templates of the collected mails, for tests.
    pub fn templates(&self) -> Vec<&'static str> {
        self.0.iter().map(Pending::template).collect()
    }
}

async fn deliver(state: &AppState, pending: &Pending) -> anyhow::Result<()> {
    let delivery = match Delivery::prepare(state).await? {
        Ok(delivery) => delivery,
        Err(Unavailable::NoSystemMail) => {
            tracing::debug!(
                template = pending.template(),
                "no system mail configured, notice dropped"
            );
            return Ok(());
        }
    };
    // The mailer writes the result to the log itself.
    let _ = delivery
        .send(state, &pending.format, |ctx| pending.render(ctx))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo::users::{
        Role,
        tests::{add, conn},
    };

    fn seen() -> Whereabouts {
        Whereabouts {
            device: Device::parse(Some("Mozilla/5.0 (X11; Linux x86_64) Firefox/140.0")),
            ip: "198.51.100.56".parse().unwrap(),
        }
    }

    fn texts(outbox: &Outbox) -> Vec<String> {
        use crate::mailer::templates::{Brand, Theme};
        let (brand, url) = (
            Brand::default(),
            url::Url::parse("https://hub.example.com").unwrap(),
        );
        outbox
            .0
            .iter()
            .map(|pending| {
                let ctx = Context {
                    brand: &brand,
                    theme: Theme::Light,
                    format: &pending.format,
                    public_url: &url,
                    now: Timestamp::now(),
                };
                pending.render(&ctx).unwrap().text
            })
            .collect()
    }

    #[test]
    fn every_event_has_its_mail() {
        let conn = conn();
        let id = add(&conn, "max", "max@example.com", Role::User);
        let profile = users::credentials_by_id(&conn, &id).unwrap().unwrap().user;
        let mut outbox = Outbox::default();
        let device = seen().device;
        for event in [
            Security::Login { at: seen() },
            Security::PasswordChanged {
                device,
                reset: false,
            },
            Security::TotpAdded {
                name: "Phone",
                first: true,
                at: seen(),
            },
            Security::TotpRemoved {
                name: "Phone",
                last: true,
                at: seen(),
            },
            Security::RecoveryUsed {
                remaining: 9,
                total: 10,
                new_device: true,
                at: seen(),
            },
            Security::RecoveryCodesRenewed { at: seen() },
        ] {
            security(&conn, &mut outbox, (&profile).into(), event).unwrap();
        }
        assert_eq!(
            outbox.templates(),
            [
                "security_login",
                "security_password",
                "security_totp_add",
                "security_totp_remove",
                "security_recovery_used",
                "security_recovery_codes",
            ]
        );
        let texts = texts(&outbox);
        assert!(texts[0].contains("Firefox on Linux"), "{}", texts[0]);
        assert!(texts[0].contains("198.51.x.x"), "{}", texts[0]);
        assert!(texts[4].contains("Unused: 9 of 10"), "{}", texts[4]);
    }

    /// The advice for the sole administrator is decided in the transaction.
    #[test]
    fn sole_admin_gets_the_reset_admin_advice() {
        let conn = conn();
        let admin = add(&conn, "anna", "anna@example.com", Role::Admin);
        let user = add(&conn, "max", "max@example.com", Role::User);
        let mut outbox = Outbox::default();
        for id in [&admin, &user] {
            let profile = users::credentials_by_id(&conn, id).unwrap().unwrap().user;
            let event = Security::PasswordChanged {
                device: seen().device,
                reset: false,
            };
            security(&conn, &mut outbox, (&profile).into(), event).unwrap();
        }
        let texts = texts(&outbox);
        assert!(texts[0].contains("roedy-hub reset-admin"), "{}", texts[0]);
        assert!(!texts[1].contains("reset-admin"), "{}", texts[1]);
    }

    /// German users get German mails.
    #[test]
    fn mails_in_the_users_language() {
        let conn = conn();
        let id = add(&conn, "max", "max@example.com", Role::User);
        users::set_sign_in_lang(&conn, &id, settings::Language::German).unwrap();
        let settings = settings::Settings {
            lang: settings::Language::German,
            ..settings::Settings::default()
        };
        settings::save(&conn, &id, 0, &settings).unwrap();
        let profile = users::credentials_by_id(&conn, &id).unwrap().unwrap().user;
        let mut outbox = Outbox::default();
        let event = Security::Login { at: seen() };
        security(&conn, &mut outbox, (&profile).into(), event).unwrap();
        assert!(
            texts(&outbox)[0].starts_with("Hallo"),
            "{}",
            texts(&outbox)[0]
        );
    }
}
