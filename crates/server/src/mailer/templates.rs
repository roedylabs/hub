//! Mail templates: per template an HTML variant (600 px, table based with
//! inline styles, as mail clients expect) and a text variant in
//! `crates/server/templates/<lang>/`, the subject from the server texts.
//! askama escapes `.html`, not `.txt`. In the templates `f` holds the values
//! shared by every mail, `v` (and `t`) those of the template.
//!
//! Taken over from Roedy Mail (`crates/server/src/mailer/templates.rs`) with
//! the templates of invitation, password reset, email change, the security
//! notices and the test email; without digests, system notices, the mail
//! logo and places from GeoIP. The links point to the hub's pages.

use std::net::IpAddr;

use askama::Template;
use jiff::Timestamp;
use serde::Deserialize;
use url::Url;

use super::{
    Mail,
    format::{Format, first_name, full_name, short_ip},
};
use crate::{
    db::repo::users::Role,
    i18n::{t, tf},
    settings::Language,
};

/// Name of the app in mails until the branding names another one.
pub const APP_NAME: &str = "Roedy Hub";

/// Light or dark; the administration preview picks it with
/// `theme=light|dark`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

/// Surface and text colours of a theme.
struct Colors {
    page: &'static str,
    card: &'static str,
    fg: &'static str,
    mfg: &'static str,
    bd: &'static str,
    soft: &'static str,
}

const LIGHT: Colors = Colors {
    page: "#f3eee7",
    card: "#ffffff",
    fg: "#2b221c",
    mfg: "#6f6155",
    bd: "#e9e0d4",
    soft: "#f8f3ec",
};

const DARK: Colors = Colors {
    page: "#181411",
    card: "#221d19",
    fg: "#f4ece2",
    mfg: "#b3a597",
    bd: "#3a322b",
    soft: "#2a241f",
};

/// Brand colour, the same values as `packages/tokens`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accent {
    pub base: &'static str,
    pub ink: &'static str,
}

pub const ACCENTS: [(&str, Accent); 5] = [
    (
        "apricot",
        Accent {
            base: "#f5a06e",
            ink: "#3a1b06",
        },
    ),
    (
        "sage",
        Accent {
            base: "#94c48d",
            ink: "#13300f",
        },
    ),
    (
        "lavender",
        Accent {
            base: "#b0a2f2",
            ink: "#21134d",
        },
    ),
    (
        "sky",
        Accent {
            base: "#82bbec",
            ink: "#0b2a47",
        },
    ),
    (
        "rose",
        Accent {
            base: "#f097ad",
            ink: "#4a0f20",
        },
    ),
];

impl Accent {
    /// By the name from the branding; unknown → apricot.
    pub fn named(name: &str) -> Self {
        ACCENTS
            .iter()
            .find(|(n, _)| *n == name)
            .map_or(ACCENTS[0].1, |(_, accent)| *accent)
    }
}

/// Plate behind the app icon: the brand colour or a fixed light or dark
/// surface.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Plate {
    #[default]
    Accent,
    Light,
    Dark,
}

impl Plate {
    /// By the name from the branding; unknown → accent colour.
    pub fn named(name: &str) -> Self {
        match name {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::Accent,
        }
    }

    /// Surface and ink; the surface is the logo plate in the mail header.
    pub fn colors(self, accent: Accent) -> (&'static str, &'static str) {
        match self {
            Self::Accent => (accent.base, accent.ink),
            Self::Light => ("#fffdfa", "#2b221c"),
            Self::Dark => ("#2b221c", "#fffaf3"),
        }
    }
}

/// Branding and organisation (`AppState::brand`). The default: app name
/// "Roedy Hub" on apricot, no organisation details (then the app name is in
/// the footer).
#[derive(Debug, Clone)]
pub struct Brand {
    pub app: String,
    pub accent: Accent,
    pub plate: Plate,
    pub org: Option<String>,
    pub support: Option<String>,
    pub imprint: Option<String>,
    pub privacy: Option<String>,
}

impl Default for Brand {
    fn default() -> Self {
        Self {
            app: APP_NAME.into(),
            accent: Accent::named("apricot"),
            plate: Plate::Accent,
            org: None,
            support: None,
            imprint: None,
            privacy: None,
        }
    }
}

/// What every mail needs.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    pub brand: &'a Brand,
    pub theme: Theme,
    /// Time zone and formats of the recipient.
    pub format: &'a Format,
    /// Public address of the app; the links use its origin.
    pub public_url: &'a Url,
    /// Reference for "Today".
    pub now: Timestamp,
}

impl Context<'_> {
    /// Language of the recipient.
    fn lang(&self) -> Language {
        self.format.lang()
    }

    fn link(&self, path: &str) -> String {
        format!("{}{path}", self.public_url.origin().ascii_serialization())
    }
}

/// Values shared by every mail: colours, branding, footer.
struct Frame<'a> {
    /// Language code of the recipient for `lang` on `<html>` and the outer
    /// table; the same language picks the template.
    lang: &'static str,
    subject: &'a str,
    page: &'static str,
    card: &'static str,
    fg: &'static str,
    mfg: &'static str,
    bd: &'static str,
    soft: &'static str,
    acc: &'static str,
    acc_ink: &'static str,
    app: &'a str,
    /// Plate of the logo.
    logo_bg: &'static str,
    /// Absolute address of the mail logo if there is one.
    logo_src: Option<String>,
    org: &'a str,
    support: Option<&'a str>,
    imprint: Option<&'a str>,
    privacy: Option<&'a str>,
    /// Recipient for the footer.
    to: &'a str,
}

impl<'a> Frame<'a> {
    fn new(ctx: &Context<'a>, subject: &'a str, to: &'a str) -> Self {
        let colors = match ctx.theme {
            Theme::Light => &LIGHT,
            Theme::Dark => &DARK,
        };
        let brand = ctx.brand;
        let some = |value: &'a Option<String>| value.as_deref().filter(|v| !v.trim().is_empty());
        Self {
            lang: ctx.lang().code(),
            subject,
            page: colors.page,
            card: colors.card,
            fg: colors.fg,
            mfg: colors.mfg,
            bd: colors.bd,
            soft: colors.soft,
            acc: brand.accent.base,
            acc_ink: brand.accent.ink,
            app: &brand.app,
            logo_bg: brand.plate.colors(brand.accent).0,
            // The mail logo follows with the branding; until then the header
            // shows the plate.
            logo_src: None,
            org: some(&brand.org).unwrap_or(&brand.app),
            // The organisation is already checked on save; here only so
            // that no other scheme ever ends up as a link in a mail.
            support: some(&brand.support).filter(|support| is_address(support)),
            imprint: some(&brand.imprint).filter(|url| is_link(url)),
            privacy: some(&brand.privacy).filter(|url| is_link(url)),
            to,
        }
    }

    /// Separator between the legal notice and privacy policy links.
    fn legal_sep(&self) -> bool {
        self.imprint.is_some() && self.privacy.is_some()
    }
}

/// A link for mails and the web app: `http://` or `https://` with a host, without
/// whitespace and control characters (which the URL parser would otherwise silently remove) –
/// otherwise no link (e.g. `javascript:` or `data:`). The same rule is checked by
/// Administration › Organisation on save.
pub fn is_link(value: &str) -> bool {
    let scheme = ["http://", "https://"].iter().any(|prefix| {
        value
            .get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
    });
    scheme
        && !value.chars().any(|c| c.is_whitespace() || c.is_control())
        && Url::parse(value).is_ok_and(|url| url.host().is_some())
}

/// An address for `mailto:` – without whitespace, `?` and control characters.
pub fn is_address(value: &str) -> bool {
    value
        .split_once('@')
        .is_some_and(|(local, domain)| !local.is_empty() && !domain.is_empty())
        && !value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '?')
}

/// Mail from subject and rendered templates (HTML, text).
fn assemble(template: &'static str, to: &str, subject: String, pair: (String, String)) -> Mail {
    let (html, text) = pair;
    Mail {
        template,
        to: to.to_owned(),
        subject,
        html,
        text,
    }
}

/// Invitation (`invite`).
#[derive(Debug, Clone, Copy)]
pub struct Invite<'a> {
    /// Address of the invited person.
    pub to: &'a str,
    /// Name if given (only with an address); otherwise "Hi,".
    pub name: Option<&'a str>,
    /// Display name of the inviter; without it "Your administrator".
    pub inviter: Option<&'a str>,
    pub role: Role,
    pub token: &'a str,
    pub expires_at: Timestamp,
}

struct InviteView {
    invitee: Option<String>,
    inviter: String,
    role: &'static str,
    link: String,
    until: String,
}

impl Invite<'_> {
    pub fn render(&self, ctx: &Context<'_>) -> anyhow::Result<Mail> {
        let (lang, app) = (ctx.lang(), &ctx.brand.app);
        let subject = tf(lang, "mail.invite.subject", &[("app", &app)]);
        let f = Frame::new(ctx, &subject, self.to);
        let v = InviteView {
            invitee: self.name.and_then(first_name),
            inviter: self
                .inviter
                .map(full_name)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| your_administrator(lang).into()),
            role: t(
                lang,
                match self.role {
                    Role::Admin => "mail.role.admin",
                    Role::User => "mail.role.user",
                },
            ),
            link: ctx.link(&format!("/invite/{}", self.token)),
            until: ctx.format.date(self.expires_at),
        };
        let pair = invite::pair(ctx.lang(), &f, &v)?;
        Ok(assemble("invite", self.to, subject, pair))
    }
}

/// Reset password (`reset`).
#[derive(Debug, Clone, Copy)]
pub struct Reset<'a> {
    /// Address of the user.
    pub to: &'a str,
    /// Display name; the greeting uses the first name.
    pub name: &'a str,
    pub token: &'a str,
}

/// Greeting and link (`reset`, `email_confirm`).
struct LinkView {
    first: Option<String>,
    link: String,
}

impl Reset<'_> {
    pub fn render(&self, ctx: &Context<'_>) -> anyhow::Result<Mail> {
        let app = &ctx.brand.app;
        let subject = tf(ctx.lang(), "mail.reset.subject", &[("app", &app)]);
        let f = Frame::new(ctx, &subject, self.to);
        let v = LinkView {
            first: first_name(self.name),
            link: ctx.link(&format!("/login/reset/{}", self.token)),
        };
        let pair = reset::pair(ctx.lang(), &f, &v)?;
        Ok(assemble("reset", self.to, subject, pair))
    }
}

/// Confirm new email address (`email_confirm`): to the new address, link
/// valid for 24 hours.
#[derive(Debug, Clone, Copy)]
pub struct EmailConfirm<'a> {
    /// New, not yet confirmed address, also the recipient.
    pub to: &'a str,
    pub name: &'a str,
    pub token: &'a str,
}

impl EmailConfirm<'_> {
    pub fn render(&self, ctx: &Context<'_>) -> anyhow::Result<Mail> {
        let app = &ctx.brand.app;
        let subject = tf(ctx.lang(), "mail.email_confirm.subject", &[("app", &app)]);
        let f = Frame::new(ctx, &subject, self.to);
        let v = LinkView {
            first: first_name(self.name),
            link: ctx.link(&format!("/account/confirm-email/{}", self.token)),
        };
        let pair = email_confirm::pair(ctx.lang(), &f, &v)?;
        Ok(assemble("email_confirm", self.to, subject, pair))
    }
}

/// Where a session is: the truncated IP address (the hub has no GeoIP, so
/// there is no place as in Roedy Mail).
#[derive(Debug, Clone, Copy)]
pub struct Place {
    pub ip: IpAddr,
}

impl Place {
    /// Row "IP address".
    fn row(self, lang: Language) -> (&'static str, String) {
        (t(lang, "mail.place.ip"), short_ip(self.ip))
    }
}

/// Who acts when no name is known (invitation, multi-factor
/// reset).
fn your_administrator(lang: Language) -> &'static str {
    t(lang, "mail.your_administrator")
}

/// Security notice "New sign-in" (`security_login`).
#[derive(Debug, Clone, Copy)]
pub struct SecurityLogin<'a> {
    pub to: &'a str,
    pub name: &'a str,
    /// Device, e.g. "Safari on iPhone".
    pub device: &'a str,
    pub place: Place,
    pub at: Timestamp,
}

/// Security notice "Password changed" (`security_password`).
#[derive(Debug, Clone, Copy)]
pub struct SecurityPassword<'a> {
    pub to: &'a str,
    pub name: &'a str,
    pub device: &'a str,
    /// Newly set via the link from "Reset password": all devices are
    /// already signed out, own advice and button to Account › Security.
    /// Otherwise the advice goes via "Forgot password" (signs out all
    /// devices without a code), the button to `/login/forgot`.
    pub reset: bool,
    /// The recipient is the sole active administrator:
    /// nobody in the app can lock their account, so the advice names
    /// the server operator. No effect with `reset`.
    pub sole_admin: bool,
    pub at: Timestamp,
}

/// Security notice "Email address is being changed" (`security_email`): to
/// the previous address when a change is requested.
#[derive(Debug, Clone, Copy)]
pub struct SecurityEmail<'a> {
    /// Previous address, also the recipient.
    pub to: &'a str,
    pub name: &'a str,
    /// New, not yet confirmed address.
    pub new_email: &'a str,
    /// Device of the session that requests the change.
    pub device: &'a str,
    pub place: Place,
    pub at: Timestamp,
}

/// Security notice "Authenticator app added" (`security_totp_add`).
#[derive(Debug, Clone, Copy)]
pub struct SecurityTotpAdded<'a> {
    pub to: &'a str,
    pub name: &'a str,
    /// Name of the app as in Account › Security.
    pub app: &'a str,
    /// The first app: multi-factor is thereby on, removing it
    /// and changing the password require a code from exactly this app – the advice
    /// therefore goes via "Forgot password" (ends all sessions without a code)
    /// to the administration, the button to `/login/forgot`.
    pub first_app: bool,
    /// The recipient is the sole active administrator: instead of the
    /// administration, the mail advises contacting the server operator. Only
    /// effective with `first_app`.
    pub sole_admin: bool,
    /// Device of the session that added it.
    pub device: &'a str,
    pub place: Place,
    pub at: Timestamp,
}

/// Security notice "Authenticator app removed" (`security_totp_remove`).
#[derive(Debug, Clone, Copy)]
pub struct SecurityTotpRemoved<'a> {
    pub to: &'a str,
    pub name: &'a str,
    pub app: &'a str,
    /// The last app: multi-factor is off, the recovery codes
    /// are deleted.
    pub last: bool,
    pub device: &'a str,
    pub place: Place,
    pub at: Timestamp,
}

/// Security notice "Signed in with a recovery code"
/// (`security_recovery_used`).
#[derive(Debug, Clone, Copy)]
pub struct SecurityRecoveryUsed<'a> {
    pub to: &'a str,
    pub name: &'a str,
    /// Still unused codes out of `total` ("‹n› of 10"; 0 has its own
    /// advice).
    pub remaining: u32,
    pub total: u32,
    /// From a previously unknown device: then only this mail goes out,
    /// not also "New sign-in".
    pub new_device: bool,
    pub device: &'a str,
    pub place: Place,
    pub at: Timestamp,
}

/// Security notice "New recovery codes generated"
/// (`security_recovery_codes`): after "Generate new codes".
#[derive(Debug, Clone, Copy)]
pub struct SecurityRecoveryCodes<'a> {
    pub to: &'a str,
    pub name: &'a str,
    pub device: &'a str,
    pub place: Place,
    pub at: Timestamp,
}

/// Security notice "Multi-factor reset" (`security_mfa_reset`): to the
/// person affected after "Reset second factor" in Administration › Users.
/// Taken over from Roedy Mail.
#[derive(Debug, Clone, Copy)]
pub struct SecurityMfaReset<'a> {
    pub to: &'a str,
    pub name: &'a str,
    /// Display name of the administrator; without it "Your
    /// administrator".
    pub by: Option<&'a str>,
    pub at: Timestamp,
}

/// Values of `security_mfa_reset`.
struct MfaResetView {
    by: String,
    /// "5 November 2026 at 09:26".
    when: String,
}

/// Shared values of all security notices; the template reads each variant's
/// own ones from its input `t`.
struct SecurityView<'a> {
    first: Option<String>,
    device: &'a str,
    loc_key: &'static str,
    loc_value: String,
    when: String,
    /// Button target: [`SECURITY_PATH`], for "Password changed" and the
    /// first authenticator app [`FORGOT_PATH`].
    link: String,
}

/// Account › Security.
const SECURITY_PATH: &str = "/account/security";
/// "Forgot password": signs out all devices without a code.
const FORGOT_PATH: &str = "/login/forgot";

impl<'a> SecurityView<'a> {
    /// With the button to Account › Security.
    fn new(
        ctx: &Context<'_>,
        name: &str,
        device: &'a str,
        place: Option<Place>,
        at: Timestamp,
    ) -> Self {
        Self::linked(ctx, name, device, place, at, SECURITY_PATH)
    }

    /// With the button to `path`.
    fn linked(
        ctx: &Context<'_>,
        name: &str,
        device: &'a str,
        place: Option<Place>,
        at: Timestamp,
        path: &str,
    ) -> Self {
        let (loc_key, loc_value) = place.map(|place| place.row(ctx.lang())).unwrap_or_default();
        Self {
            first: first_name(name),
            device,
            loc_key,
            loc_value,
            when: ctx.format.when(at, ctx.now),
            link: ctx.link(path),
        }
    }
}

impl SecurityLogin<'_> {
    pub fn render(&self, ctx: &Context<'_>) -> anyhow::Result<Mail> {
        let app = &ctx.brand.app;
        let subject = tf(ctx.lang(), "mail.security_login.subject", &[("app", &app)]);
        let f = Frame::new(ctx, &subject, self.to);
        let v = SecurityView::new(ctx, self.name, self.device, Some(self.place), self.at);
        let pair = security_login::pair(ctx.lang(), &f, &v)?;
        Ok(assemble("security_login", self.to, subject, pair))
    }
}

impl SecurityPassword<'_> {
    pub fn render(&self, ctx: &Context<'_>) -> anyhow::Result<Mail> {
        let subject = t(
            ctx.lang(),
            if self.reset {
                "mail.security_password.subject_reset"
            } else {
                "mail.security_password.subject"
            },
        )
        .to_owned();
        let f = Frame::new(ctx, &subject, self.to);
        // Without `reset` the button leads to "Forgot password".
        let path = if self.reset {
            SECURITY_PATH
        } else {
            FORGOT_PATH
        };
        let v = SecurityView::linked(ctx, self.name, self.device, None, self.at, path);
        let pair = security_password::pair(ctx.lang(), &f, &v, self)?;
        Ok(assemble("security_password", self.to, subject, pair))
    }
}

impl SecurityEmail<'_> {
    pub fn render(&self, ctx: &Context<'_>) -> anyhow::Result<Mail> {
        let app = &ctx.brand.app;
        let subject = tf(ctx.lang(), "mail.security_email.subject", &[("app", &app)]);
        let f = Frame::new(ctx, &subject, self.to);
        let v = SecurityView::new(ctx, self.name, self.device, Some(self.place), self.at);
        let pair = security_email::pair(ctx.lang(), &f, &v, self)?;
        Ok(assemble("security_email", self.to, subject, pair))
    }
}

impl SecurityTotpAdded<'_> {
    pub fn render(&self, ctx: &Context<'_>) -> anyhow::Result<Mail> {
        let subject = t(ctx.lang(), "mail.security_totp_add.subject").to_owned();
        let f = Frame::new(ctx, &subject, self.to);
        // For the first app the button leads to "Forgot password".
        let path = if self.first_app {
            FORGOT_PATH
        } else {
            SECURITY_PATH
        };
        let v = SecurityView::linked(ctx, self.name, self.device, Some(self.place), self.at, path);
        let pair = security_totp_add::pair(ctx.lang(), &f, &v, self)?;
        Ok(assemble("security_totp_add", self.to, subject, pair))
    }
}

impl SecurityTotpRemoved<'_> {
    pub fn render(&self, ctx: &Context<'_>) -> anyhow::Result<Mail> {
        let subject = t(ctx.lang(), "mail.security_totp_remove.subject").to_owned();
        let f = Frame::new(ctx, &subject, self.to);
        let v = SecurityView::new(ctx, self.name, self.device, Some(self.place), self.at);
        let pair = security_totp_remove::pair(ctx.lang(), &f, &v, self)?;
        Ok(assemble("security_totp_remove", self.to, subject, pair))
    }
}

impl SecurityRecoveryUsed<'_> {
    pub fn render(&self, ctx: &Context<'_>) -> anyhow::Result<Mail> {
        let app = &ctx.brand.app;
        let subject = tf(
            ctx.lang(),
            "mail.security_recovery_used.subject",
            &[("app", &app)],
        );
        let f = Frame::new(ctx, &subject, self.to);
        let v = SecurityView::new(ctx, self.name, self.device, Some(self.place), self.at);
        let pair = security_recovery_used::pair(ctx.lang(), &f, &v, self)?;
        Ok(assemble("security_recovery_used", self.to, subject, pair))
    }
}

impl SecurityRecoveryCodes<'_> {
    pub fn render(&self, ctx: &Context<'_>) -> anyhow::Result<Mail> {
        let subject = t(ctx.lang(), "mail.security_recovery_codes.subject").to_owned();
        let f = Frame::new(ctx, &subject, self.to);
        let v = SecurityView::new(ctx, self.name, self.device, Some(self.place), self.at);
        let pair = security_recovery_codes::pair(ctx.lang(), &f, &v)?;
        Ok(assemble("security_recovery_codes", self.to, subject, pair))
    }
}

impl SecurityMfaReset<'_> {
    pub fn render(&self, ctx: &Context<'_>) -> anyhow::Result<Mail> {
        let lang = ctx.lang();
        let subject = t(lang, "mail.security_mfa_reset.subject").to_owned();
        let f = Frame::new(ctx, &subject, self.to);
        let v = SecurityView::new(ctx, self.name, "", None, self.at);
        let t = MfaResetView {
            by: self
                .by
                .map(full_name)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| your_administrator(lang).into()),
            when: tf(
                lang,
                "mail.date_at_time",
                &[
                    ("date", &ctx.format.date(self.at)),
                    ("time", &ctx.format.time(self.at)),
                ],
            ),
        };
        let pair = security_mfa_reset::pair(lang, &f, &v, &t)?;
        Ok(assemble("security_mfa_reset", self.to, subject, pair))
    }
}

/// Test email from Administration › Email (`system_test`): without a
/// greeting, because the address is arbitrary.
#[derive(Debug, Clone, Copy)]
pub struct SystemTest<'a> {
    pub to: &'a str,
}

impl SystemTest<'_> {
    pub fn render(&self, ctx: &Context<'_>) -> anyhow::Result<Mail> {
        let app = &ctx.brand.app;
        let subject = tf(ctx.lang(), "mail.system_test.subject", &[("app", &app)]);
        let f = Frame::new(ctx, &subject, self.to);
        let pair = system_test::pair(ctx.lang(), &f)?;
        Ok(assemble("system_test", self.to, subject, pair))
    }
}

/// HTML and text template of a mail in each language, under
/// `templates/<lang>/`, all with the same values. `pair` renders them in the
/// recipient's language.
macro_rules! templates {
    ($($name:ident($en_html:tt, $en_text:tt, $de_html:tt, $de_text:tt) { $($field:ident: $ty:ty),* })*) => {$(
        mod $name {
            use super::*;

            #[derive(Template)]
            #[template(path = $en_html)]
            struct EnglishHtml<'a> {
                f: &'a Frame<'a>,
                $($field: $ty,)*
            }

            #[derive(Template)]
            #[template(path = $en_text)]
            struct EnglishText<'a> {
                f: &'a Frame<'a>,
                $($field: $ty,)*
            }

            #[derive(Template)]
            #[template(path = $de_html)]
            struct GermanHtml<'a> {
                f: &'a Frame<'a>,
                $($field: $ty,)*
            }

            #[derive(Template)]
            #[template(path = $de_text)]
            struct GermanText<'a> {
                f: &'a Frame<'a>,
                $($field: $ty,)*
            }

            /// HTML and text in `lang`.
            pub(super) fn pair<'a>(
                lang: Language,
                f: &'a Frame<'a>,
                $($field: $ty,)*
            ) -> askama::Result<(String, String)> {
                Ok(match lang {
                    // `System` is resolved before (`settings::Stored::lang`).
                    Language::English | Language::System => (
                        EnglishHtml { f, $($field,)* }.render()?,
                        EnglishText { f, $($field,)* }.render()?,
                    ),
                    Language::German => (
                        GermanHtml { f, $($field,)* }.render()?,
                        GermanText { f, $($field,)* }.render()?,
                    ),
                })
            }
        }
    )*};
}

templates! {
    invite("en/invite.html", "en/invite.txt", "de/invite.html", "de/invite.txt") {
        v: &'a InviteView
    }
    reset("en/reset.html", "en/reset.txt", "de/reset.html", "de/reset.txt") { v: &'a LinkView }
    email_confirm(
        "en/email_confirm.html", "en/email_confirm.txt",
        "de/email_confirm.html", "de/email_confirm.txt"
    ) {
        v: &'a LinkView
    }
    security_login(
        "en/security_login.html", "en/security_login.txt",
        "de/security_login.html", "de/security_login.txt"
    ) {
        v: &'a SecurityView<'a>
    }
    security_password(
        "en/security_password.html", "en/security_password.txt",
        "de/security_password.html", "de/security_password.txt"
    ) {
        v: &'a SecurityView<'a>, t: &'a SecurityPassword<'a>
    }
    security_email(
        "en/security_email.html", "en/security_email.txt",
        "de/security_email.html", "de/security_email.txt"
    ) {
        v: &'a SecurityView<'a>, t: &'a SecurityEmail<'a>
    }
    security_totp_add(
        "en/security_totp_add.html", "en/security_totp_add.txt",
        "de/security_totp_add.html", "de/security_totp_add.txt"
    ) {
        v: &'a SecurityView<'a>, t: &'a SecurityTotpAdded<'a>
    }
    security_totp_remove(
        "en/security_totp_remove.html", "en/security_totp_remove.txt",
        "de/security_totp_remove.html", "de/security_totp_remove.txt"
    ) {
        v: &'a SecurityView<'a>, t: &'a SecurityTotpRemoved<'a>
    }
    security_recovery_used(
        "en/security_recovery_used.html", "en/security_recovery_used.txt",
        "de/security_recovery_used.html", "de/security_recovery_used.txt"
    ) {
        v: &'a SecurityView<'a>, t: &'a SecurityRecoveryUsed<'a>
    }
    security_recovery_codes(
        "en/security_recovery_codes.html", "en/security_recovery_codes.txt",
        "de/security_recovery_codes.html", "de/security_recovery_codes.txt"
    ) {
        v: &'a SecurityView<'a>
    }
    security_mfa_reset(
        "en/security_mfa_reset.html", "en/security_mfa_reset.txt",
        "de/security_mfa_reset.html", "de/security_mfa_reset.txt"
    ) {
        v: &'a SecurityView<'a>, t: &'a MfaResetView
    }
    system_test(
        "en/system_test.html", "en/system_test.txt",
        "de/system_test.html", "de/system_test.txt"
    ) {}
}

#[cfg(test)]
pub(super) mod tests {
    use std::{fmt::Display, fs, path::PathBuf};

    use super::*;
    use crate::auth::device;

    /// Snapshots under `src/mailer/snapshots`, rendered in English: `*.txt`
    /// and `*.light.html`/`*.dark.html` per sample of [`mails`]. Regenerate
    /// them with `ROEDY_UPDATE_SNAPSHOTS=1 cargo test -p roedy-hub-server mailer`
    /// and review the diff.
    fn snapshot(name: &str, actual: &str) {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/mailer/snapshots")
            .join(name);
        let actual = format!("{actual}\n");
        if std::env::var_os("ROEDY_UPDATE_SNAPSHOTS").is_some() {
            fs::write(&path, &actual).unwrap();
            return;
        }
        let expected = fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("snapshot {} missing", path.display()));
        assert!(
            expected == actual,
            "Snapshot {name} differs.\n--- expected\n{expected}\n--- actual\n{actual}"
        );
    }

    /// Whether `text` still contains a `{name}` placeholder.
    fn has_placeholder(text: &str) -> bool {
        text.match_indices('{').any(|(i, _)| {
            let rest = &text[i + 1..];
            rest.find('}').is_some_and(|end| {
                end > 0
                    && rest[..end]
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c == '_')
            })
        })
    }

    /// `html` declares `lang` on `<html>` and on the outer layout table, so
    /// clients that drop `<html>` (webmailers) keep the language.
    fn assert_lang(html: &str, lang: &str) {
        let doctype = format!("<!DOCTYPE html>\n<html lang=\"{lang}\">");
        assert!(html.starts_with(&doctype), "<html lang> is not {lang}");
        let outer = &html[html.find("<table").expect("no table")..];
        let table = format!("<table role=\"presentation\" lang=\"{lang}\" width=\"100%\"");
        assert!(outer.starts_with(&table), "outer table lang is not {lang}");
    }

    /// `german` is the German rendering of the same mail as `english`: the
    /// subject comes from the German texts, body and HTML are translated,
    /// every placeholder is filled and the HTML declares German, the English
    /// one English.
    fn assert_german(english: &Mail, german: &Mail, app: &str) {
        let name = english.template;
        assert_eq!(german.template, name);
        assert_eq!(german.to, english.to, "{name}");
        let args: [(&str, &dyn Display); 1] = [("app", &app)];
        let keys = [
            format!("mail.{name}.subject"),
            "mail.security_password.subject_reset".to_owned(),
        ];
        let key = keys
            .iter()
            .find(|key| tf(Language::English, key, &args) == english.subject)
            .unwrap_or_else(|| panic!("{name}: no key for {:?}", english.subject));
        assert_eq!(german.subject, tf(Language::German, key, &args), "{name}");
        assert_ne!(german.text, english.text, "{name}: text not translated");
        assert_ne!(german.html, english.html, "{name}: HTML not translated");
        assert_lang(&german.html, "de");
        assert_lang(&english.html, "en");
        for mail in [english, german] {
            for text in [&mail.subject, &mail.text, &mail.html] {
                assert!(!has_placeholder(text), "{name}: placeholder in {text}");
            }
        }
    }

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    /// Branding with every organisation detail filled in.
    fn brand() -> Brand {
        Brand {
            org: Some("Example Ltd".into()),
            support: Some("support@example.com".into()),
            imprint: Some("https://example.com/imprint".into()),
            privacy: Some("https://example.com/privacy".into()),
            ..Brand::default()
        }
    }

    fn with_context<T>(brand: &Brand, theme: Theme, f: impl FnOnce(&Context<'_>) -> T) -> T {
        with_context_in(brand, theme, Language::English, f)
    }

    fn with_context_in<T>(
        brand: &Brand,
        theme: Theme,
        lang: Language,
        f: impl FnOnce(&Context<'_>) -> T,
    ) -> T {
        let format = Format::default().with_lang(lang);
        let public_url = Url::parse("https://hub.example.com/").unwrap();
        f(&Context {
            brand,
            theme,
            format: &format,
            public_url: &public_url,
            // 24 October 2026, 18:00 in Berlin; "Today" is the 24th.
            now: at("2026-10-24T16:00:00Z"),
        })
    }

    /// 15:57 in Berlin (summer time).
    const AT: &str = "2026-10-24T13:57:00Z";
    /// Documentation address (RFC 5737) of the sample sessions.
    const IP: &str = "198.51.100.56";

    fn place() -> Place {
        Place {
            ip: IP.parse().unwrap(),
        }
    }

    fn invite() -> Invite<'static> {
        Invite {
            to: "jonas@example.com",
            name: Some("Jonas"),
            inviter: Some("Max Example"),
            role: Role::User,
            token: "k7Qe2xR9",
            expires_at: at("2026-10-30T14:57:00Z"),
        }
    }

    fn password(reset: bool, sole_admin: bool) -> SecurityPassword<'static> {
        SecurityPassword {
            to: "max@example.net",
            name: "Max Example",
            device: "Safari on iPhone",
            reset,
            sole_admin,
            at: at(AT),
        }
    }

    fn totp_added(first_app: bool, sole_admin: bool) -> SecurityTotpAdded<'static> {
        SecurityTotpAdded {
            to: "max@example.net",
            name: "Max Example",
            app: "Authenticator app",
            first_app,
            sole_admin,
            device: "Safari on iPhone",
            place: place(),
            at: at(AT),
        }
    }

    fn totp_removed(last: bool) -> SecurityTotpRemoved<'static> {
        SecurityTotpRemoved {
            to: "max@example.net",
            name: "Max Example",
            app: "Authenticator app",
            last,
            device: "Safari on iPhone",
            place: place(),
            at: at(AT),
        }
    }

    fn recovery_used(remaining: u32, new_device: bool) -> SecurityRecoveryUsed<'static> {
        SecurityRecoveryUsed {
            to: "max@example.net",
            name: "Max Example",
            remaining,
            total: 10,
            new_device,
            device: "Safari on iPhone",
            place: place(),
            at: at(AT),
        }
    }

    /// One sample per template and variant, in the language of `ctx`.
    fn mails(ctx: &Context<'_>) -> Vec<(&'static str, Mail)> {
        let lang = ctx.lang();
        let safari = device::label(Some("Safari"), Some("iPhone"), lang);
        let chrome = device::label(Some("Chrome"), Some("macOS"), lang);
        let authenticator = t(lang, "totp.default_name");

        vec![
            ("invite", invite().render(ctx).unwrap()),
            (
                "reset",
                Reset {
                    to: "max@example.net",
                    name: "Max Example",
                    token: "Zm4pT1",
                }
                .render(ctx)
                .unwrap(),
            ),
            (
                "email_confirm",
                EmailConfirm {
                    to: "max.example@example.com",
                    name: "Max Example",
                    token: "b3Xr8kQ2vN",
                }
                .render(ctx)
                .unwrap(),
            ),
            (
                "security_login",
                SecurityLogin {
                    to: "max@example.net",
                    name: "Max Example",
                    device: &safari,
                    place: place(),
                    at: at(AT),
                }
                .render(ctx)
                .unwrap(),
            ),
            (
                "security_password",
                SecurityPassword {
                    device: &safari,
                    ..password(false, false)
                }
                .render(ctx)
                .unwrap(),
            ),
            (
                "security_password_sole_admin",
                SecurityPassword {
                    device: &safari,
                    ..password(false, true)
                }
                .render(ctx)
                .unwrap(),
            ),
            (
                "security_password_reset",
                SecurityPassword {
                    device: &safari,
                    ..password(true, false)
                }
                .render(ctx)
                .unwrap(),
            ),
            (
                "security_email",
                SecurityEmail {
                    to: "max@example.net",
                    name: "Example, Max",
                    new_email: "max.example@example.com",
                    device: &chrome,
                    place: Place {
                        ip: "203.0.113.2".parse().unwrap(),
                    },
                    at: at(AT),
                }
                .render(ctx)
                .unwrap(),
            ),
            (
                "security_totp_add",
                SecurityTotpAdded {
                    app: authenticator,
                    device: &safari,
                    ..totp_added(false, false)
                }
                .render(ctx)
                .unwrap(),
            ),
            (
                "security_totp_add_first",
                SecurityTotpAdded {
                    app: authenticator,
                    device: &safari,
                    ..totp_added(true, false)
                }
                .render(ctx)
                .unwrap(),
            ),
            (
                "security_totp_add_first_sole_admin",
                SecurityTotpAdded {
                    app: authenticator,
                    device: &safari,
                    ..totp_added(true, true)
                }
                .render(ctx)
                .unwrap(),
            ),
            (
                "security_totp_remove",
                SecurityTotpRemoved {
                    app: authenticator,
                    device: &safari,
                    ..totp_removed(false)
                }
                .render(ctx)
                .unwrap(),
            ),
            (
                "security_totp_remove_last",
                SecurityTotpRemoved {
                    app: authenticator,
                    device: &safari,
                    ..totp_removed(true)
                }
                .render(ctx)
                .unwrap(),
            ),
            (
                "security_recovery_used",
                SecurityRecoveryUsed {
                    device: &safari,
                    ..recovery_used(7, false)
                }
                .render(ctx)
                .unwrap(),
            ),
            (
                "security_recovery_used_new_device",
                SecurityRecoveryUsed {
                    device: &safari,
                    ..recovery_used(7, true)
                }
                .render(ctx)
                .unwrap(),
            ),
            (
                "security_recovery_used_none",
                SecurityRecoveryUsed {
                    device: &safari,
                    ..recovery_used(0, false)
                }
                .render(ctx)
                .unwrap(),
            ),
            (
                "security_recovery_codes",
                SecurityRecoveryCodes {
                    to: "max@example.net",
                    name: "Max Example",
                    device: &chrome,
                    place: place(),
                    at: at(AT),
                }
                .render(ctx)
                .unwrap(),
            ),
            (
                "security_mfa_reset",
                SecurityMfaReset {
                    to: "max@example.net",
                    name: "Max Example",
                    by: Some("Example, Anna"),
                    at: at(AT),
                }
                .render(ctx)
                .unwrap(),
            ),
            (
                "system_test",
                SystemTest {
                    to: "max@example.net",
                }
                .render(ctx)
                .unwrap(),
            ),
        ]
    }

    #[test]
    fn text_snapshots() {
        let brand = brand();
        with_context(&brand, Theme::Light, |ctx| {
            for (name, mail) in mails(ctx) {
                snapshot(&format!("{name}.txt"), &mail.text);
            }
        });
    }

    #[test]
    fn html_snapshots_light_and_dark() {
        let brand = brand();
        for (theme, suffix) in [(Theme::Light, "light"), (Theme::Dark, "dark")] {
            with_context(&brand, theme, |ctx| {
                for (name, mail) in mails(ctx) {
                    snapshot(&format!("{name}.{suffix}.html"), &mail.html);
                }
            });
        }
    }

    /// Every snapshot belongs to a sample: none is left over from a removed
    /// one.
    #[test]
    fn no_stale_snapshots() {
        let brand = brand();
        let names: Vec<_> = with_context(&brand, Theme::Light, mails)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/mailer/snapshots");
        for entry in fs::read_dir(dir).unwrap() {
            let file = entry.unwrap().file_name().to_string_lossy().into_owned();
            if file.starts_with('.') {
                continue;
            }
            let stem = file.split('.').next().unwrap();
            assert!(names.contains(&stem), "stale snapshot {file}");
        }
    }

    #[test]
    fn subjects_and_recipients() {
        let brand = brand();
        with_context(&brand, Theme::Light, |ctx| {
            let subjects: Vec<_> = mails(ctx)
                .into_iter()
                .map(|(_, mail)| (mail.template, mail.to, mail.subject))
                .collect();
            let net = "max@example.net";
            let expected = [
                ("invite", "jonas@example.com", "Invitation to Roedy Hub"),
                ("reset", net, "Reset your Roedy Hub password"),
                (
                    "email_confirm",
                    "max.example@example.com",
                    "Confirm your new email address for Roedy Hub",
                ),
                ("security_login", net, "New sign-in to Roedy Hub"),
                ("security_password", net, "Your password was changed"),
                ("security_password", net, "Your password was changed"),
                ("security_password", net, "Your password was reset"),
                (
                    "security_email",
                    net,
                    "Your email address for Roedy Hub is being changed",
                ),
                ("security_totp_add", net, "Authenticator app added"),
                ("security_totp_add", net, "Authenticator app added"),
                ("security_totp_add", net, "Authenticator app added"),
                ("security_totp_remove", net, "Authenticator app removed"),
                ("security_totp_remove", net, "Authenticator app removed"),
                (
                    "security_recovery_used",
                    net,
                    "Recovery code used to sign in to Roedy Hub",
                ),
                (
                    "security_recovery_used",
                    net,
                    "Recovery code used to sign in to Roedy Hub",
                ),
                (
                    "security_recovery_used",
                    net,
                    "Recovery code used to sign in to Roedy Hub",
                ),
                (
                    "security_recovery_codes",
                    net,
                    "New recovery codes generated",
                ),
                (
                    "security_mfa_reset",
                    net,
                    "Multi-factor authentication reset",
                ),
                ("system_test", net, "Test email from Roedy Hub"),
            ];
            let expected: Vec<_> = expected
                .into_iter()
                .map(|(t, to, s)| (t, to.to_owned(), s.to_owned()))
                .collect();
            assert_eq!(subjects, expected);
        });
    }

    #[test]
    fn german_renderings() {
        let brand = brand();
        let english = with_context(&brand, Theme::Light, mails);
        let german = with_context_in(&brand, Theme::Light, Language::German, mails);
        assert_eq!(english.len(), german.len());
        for ((name, english), (german_name, german)) in english.iter().zip(&german) {
            assert_eq!(name, german_name);
            assert_german(english, german, &brand.app);
        }
    }

    #[test]
    fn dark_uses_the_dark_colors() {
        let brand = brand();
        let light = with_context(&brand, Theme::Light, |ctx| invite().render(ctx).unwrap());
        let dark = with_context(&brand, Theme::Dark, |ctx| invite().render(ctx).unwrap());
        assert!(light.html.contains(LIGHT.page) && !light.html.contains(DARK.page));
        assert!(dark.html.contains(DARK.page) && !dark.html.contains(LIGHT.page));
        assert_eq!(light.text, dark.text);
    }

    /// Without organisation details the footer names the app and links
    /// nothing; the header shows only the plate.
    #[test]
    fn defaults_without_organisation() {
        let brand = Brand::default();
        let mail = with_context(&brand, Theme::Light, |ctx| invite().render(ctx).unwrap());
        assert!(mail.text.contains("\n--\nRoedy Hub"), "{}", mail.text);
        assert!(!mail.html.contains("mailto:") && !mail.html.contains("<img"));
        assert!(!mail.text.contains("Legal notice"));
    }

    #[test]
    fn html_escapes_values_text_does_not() {
        let brand = Brand {
            app: "<b>A&B</b>".into(),
            ..Brand::default()
        };
        let mail = with_context(&brand, Theme::Light, |ctx| {
            Invite {
                name: Some("Jo\"<i>"),
                ..invite()
            }
            .render(ctx)
            .unwrap()
        });
        // askama escapes numerically.
        assert!(mail.html.contains("Hi Jo&#34;&#60;i&#62;,"));
        assert!(
            mail.html
                .contains("<title>Invitation to &#60;b&#62;A&#38;B&#60;/b&#62;</title>")
        );
        assert!(!mail.html.contains("<b>A&B</b>"));
        assert!(mail.text.starts_with("Hi Jo\"<i>,"));
        assert_eq!(mail.subject, "Invitation to <b>A&B</b>");
    }

    #[test]
    fn links_only_http_with_host_and_without_spaces() {
        for ok in ["https://example.com/imprint", "HTTP://example.com"] {
            assert!(is_link(ok), "{ok}");
        }
        for bad in [
            "javascript:alert(1)",
            "data:text/html,x",
            "https://",
            "https://exa mple.com",
            "https://example.com/\n",
            "ftp://example.com",
        ] {
            assert!(!is_link(bad), "{bad}");
        }
        assert!(is_address("support@example.com"));
        for bad in [
            "support",
            "@example.com",
            "a@b?subject=x",
            "a b@example.com",
        ] {
            assert!(!is_address(bad), "{bad}");
        }
    }

    /// The links lead to the hub's pages at the origin of the public address.
    #[test]
    fn links_use_the_origin() {
        let brand = Brand::default();
        let format = Format::default();
        let public_url = Url::parse("https://hub.example.com/subpath/").unwrap();
        let ctx = Context {
            brand: &brand,
            theme: Theme::Light,
            format: &format,
            public_url: &public_url,
            now: at("2026-10-24T16:00:00Z"),
        };
        let link = |mail: Mail, path: &str| {
            let url = format!("\nhttps://hub.example.com{path}");
            assert!(mail.text.contains(&url), "{}: {}", mail.template, mail.text);
        };
        link(invite().render(&ctx).unwrap(), "/invite/k7Qe2xR9\n");
        let reset = Reset {
            to: "max@example.net",
            name: "",
            token: "Zm4pT1",
        };
        link(reset.render(&ctx).unwrap(), "/login/reset/Zm4pT1");
        let confirm = EmailConfirm {
            to: "max@example.net",
            name: "",
            token: "b3Xr8kQ2vN",
        };
        link(
            confirm.render(&ctx).unwrap(),
            "/account/confirm-email/b3Xr8kQ2vN",
        );
        link(
            password(true, false).render(&ctx).unwrap(),
            "/account/security",
        );
        link(
            password(false, false).render(&ctx).unwrap(),
            "/login/forgot",
        );
        link(
            totp_added(true, false).render(&ctx).unwrap(),
            "/login/forgot",
        );
        link(
            totp_added(false, false).render(&ctx).unwrap(),
            "/account/security",
        );

        let mail = SecurityPassword {
            name: "",
            ..password(true, false)
        }
        .render(&ctx)
        .unwrap();
        assert!(mail.text.starts_with("Hi,\n"));
        assert!(mail.text.contains("Time: Today, 15:57\n"), "{}", mail.text);
        let mail = invite().render(&ctx).unwrap();
        assert!(
            mail.text
                .contains("The link is valid until 30 October 2026."),
            "{}",
            mail.text
        );
    }

    /// The sole administrator is sent to the server operator and
    /// `roedy-hub reset-admin`, nobody else; after "Reset password" and for
    /// further apps it makes no difference.
    #[test]
    fn sole_admin_only_changes_its_variants() {
        let brand = brand();
        with_context(&brand, Theme::Light, |ctx| {
            let same = |a: Mail, b: Mail| a.text == b.text && a.html == b.html;
            let render = |mail: &dyn Fn(bool) -> Mail| same(mail(false), mail(true));
            assert!(render(&|sole| password(true, sole).render(ctx).unwrap()));
            assert!(render(&|sole| totp_added(false, sole).render(ctx).unwrap()));
            assert!(!render(&|sole| password(false, sole).render(ctx).unwrap()));
            assert!(!render(&|sole| totp_added(true, sole).render(ctx).unwrap()));
            for sole in [false, true] {
                for mail in [
                    password(false, sole).render(ctx).unwrap(),
                    totp_added(true, sole).render(ctx).unwrap(),
                ] {
                    assert_eq!(mail.html.contains("server operator"), sole);
                    assert_eq!(mail.text.contains("server operator"), sole);
                    assert_eq!(mail.text.contains("roedy-hub reset-admin"), sole);
                    assert_eq!(mail.html.contains("roedy-hub reset-admin"), sole);
                }
            }
        });
    }

    #[test]
    fn accents_and_plates_by_name() {
        assert_eq!(Accent::named("sage"), ACCENTS[1].1);
        assert_eq!(Accent::named("unknown"), ACCENTS[0].1);
        assert_eq!(Plate::named("dark"), Plate::Dark);
        assert_eq!(Plate::named("x"), Plate::Accent);
        let accent = Accent::named("apricot");
        assert_eq!(Plate::Accent.colors(accent), (accent.base, accent.ink));
    }

    fn spans(text: &str, open: &str, close: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find(open) {
            let Some(len) = rest[start..].find(close) else {
                break;
            };
            let span = &rest[start..start + len + close.len()];
            found.push(span.split_whitespace().collect::<Vec<_>>().join(" "));
            rest = &rest[start + len + close.len()..];
        }
        found
    }

    #[test]
    fn translations_share_the_template_structure() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates");
        let names = |lang: &str| {
            let mut names: Vec<_> = fs::read_dir(dir.join(lang))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };
        let english_names = names("en");
        assert!(english_names.len() > 1, "no templates found");
        assert_eq!(english_names, names("de"));
        // Language-specific parts: the language of the layout path and of the
        // `lang` attribute.
        let neutral = |text: &str| {
            text.replace("\"en/", "\"lang/")
                .replace("\"de/", "\"lang/")
                .replace("lang=\"en\"", "lang=\"\"")
                .replace("lang=\"de\"", "lang=\"\"")
        };
        for name in english_names {
            let english = neutral(&fs::read_to_string(dir.join("en").join(&name)).unwrap());
            let german = neutral(&fs::read_to_string(dir.join("de").join(&name)).unwrap());
            assert_eq!(
                spans(&english, "{%", "%}"),
                spans(&german, "{%", "%}"),
                "{name}: askama tags"
            );
            let sorted = |text: &str| {
                let mut found = spans(text, "{{", "}}");
                found.sort();
                found
            };
            assert_eq!(sorted(&english), sorted(&german), "{name}: expressions");
            if name.ends_with(".html") {
                assert_eq!(
                    spans(&english, "<", ">"),
                    spans(&german, "<", ">"),
                    "{name}: HTML tags"
                );
            }
            assert_ne!(english, german, "{name}: not translated");
        }
    }
}
