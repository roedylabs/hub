//! Administration › Audit log.
//!
//! - `GET admin/audit`: entries, newest first, with filters `cat`, `q`,
//!   `from`, `to` and paging via `before` and `limit`, plus the hit count per
//!   category. A search (`q`) walks the whole period and counts towards a
//!   per-administrator limit (so it needs `X-Requested-With`).
//! - `POST admin/audit/export`: the same filters as CSV, all hits; is itself
//!   recorded in the audit log, limited per administrator and to 16 MB
//!   (otherwise 422 `export_too_large`: narrow the time range).
//! - `GET`/`PUT admin/audit/retention`: retention of 30, 90 or 365 days.
//!
//! Texts are built here (`texts`), not in the web app: the search has to
//! match what is shown, and the export has to deliver the same columns.
//!
//! Taken over from Roedy Mail (`crates/server/src/api/audit.rs`) with the
//! category `apps` and a limit for the export.

pub mod texts;

use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Query, State, rejection::QueryRejection},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use jiff::{RoundMode, Timestamp, TimestampRound, Unit};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use self::texts::Shown;
use super::{ApiError, FieldErrors, JsonBody, invites::admin_holds, require_requested_with};
use crate::{
    app::AppState,
    auth::{AdminUser, AuthUser},
    client_ip::ClientIp,
    db::{
        self,
        repo::audit_log::{self, Category, Entry, Range, Row},
    },
    mailer::system_mail::timestamp,
    rate_limit::{Key, Policy},
    settings::{self, Language},
};

/// Entries per page without `limit`.
const DEFAULT_LIMIT: usize = 100;
/// Largest page.
const MAX_LIMIT: usize = 500;
/// Largest export; beyond it the administrator narrows the time range. Far
/// more than a year of a small team's entries.
const MAX_EXPORT_BYTES: usize = 16 * 1024 * 1024;
/// Exports per administrator: ten, then one every six minutes. Each walks
/// the whole retention period on the shared database connection.
const EXPORT: Policy = Policy::rate("audit.export", 10, Duration::from_secs(60 * 60));
/// Searches per administrator: a search walks the whole period too
/// (the page searches as one types, so the burst is generous).
const SEARCH: Policy = Policy::rate("audit.search", 120, Duration::from_secs(60));

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/audit", get(list))
        .route("/admin/audit/export", post(export))
        .route(
            "/admin/audit/retention",
            get(show_retention).put(update_retention),
        )
}

/// Filters of list and export.
#[derive(Debug, Default, Deserialize)]
struct FilterQuery {
    /// Category: `login`, `users`, `apps`, `settings`, `system`; without it
    /// all of them.
    cat: Option<String>,
    /// Search in person, action, details and category.
    q: Option<String>,
    /// From this point in time (RFC 3339, inclusive).
    from: Option<String>,
    /// Before this point in time (RFC 3339).
    to: Option<String>,
}

#[derive(Debug)]
struct Filter {
    category: Option<Category>,
    /// Lowercase; empty = no search.
    needle: String,
    range: Range,
}

impl FilterQuery {
    fn parse(self) -> Result<Filter, ApiError> {
        let category = self
            .cat
            .filter(|cat| !cat.is_empty())
            .map(|cat| {
                Category::parse(&cat).ok_or_else(|| ApiError::BadRequest("unknown category".into()))
            })
            .transpose()?;
        Ok(Filter {
            category,
            needle: self.q.unwrap_or_default().trim().to_lowercase(),
            range: Range {
                from: instant(self.from)?,
                to: instant(self.to)?,
            },
        })
    }
}

/// Point in time of the request in the format of the `at` column.
///
/// Rounded up to the millisecond like the stored times: `from` is
/// inclusive and `to` exclusive, so an entry at `.123` is in the range from
/// `.1235` neither way.
fn instant(value: Option<String>) -> Result<Option<String>, ApiError> {
    let invalid = || ApiError::BadRequest("invalid point in time".into());
    value
        .filter(|value| !value.is_empty())
        .map(|value| {
            let at = value.parse::<Timestamp>().map_err(|_| invalid())?;
            let ceil = TimestampRound::new()
                .smallest(Unit::Millisecond)
                .mode(RoundMode::Ceil);
            at.round(ceil).map(timestamp).map_err(|_| invalid())
        })
        .transpose()
}

impl Filter {
    /// Does the search match the entry? The category does not count: the
    /// numbers on the filters apply to all categories.
    fn found(&self, row: &Row, shown: &Shown, lang: Language) -> bool {
        self.needle.is_empty()
            || shown
                .searchable(row.category, lang)
                .to_lowercase()
                .contains(&self.needle)
    }

    fn in_category(&self, row: &Row) -> bool {
        self.category
            .is_none_or(|category| category == row.category)
    }
}

#[derive(Debug, Deserialize)]
struct PageQuery {
    /// `next` of the previous page.
    before: Option<i64>,
    limit: Option<usize>,
}

/// An entry of the page, `at` in UTC.
#[derive(Debug, Serialize)]
struct Item {
    id: i64,
    at: String,
    cat: &'static str,
    icon: &'static str,
    who: String,
    act: String,
    meta: String,
    warn: bool,
}

/// Hit counts of the category filters.
#[derive(Debug, Default, Serialize)]
struct Counts {
    all: u64,
    login: u64,
    users: u64,
    apps: u64,
    settings: u64,
    system: u64,
}

impl Counts {
    fn add(&mut self, category: Category, n: u64) {
        self.all += n;
        *match category {
            Category::Login => &mut self.login,
            Category::Users => &mut self.users,
            Category::Apps => &mut self.apps,
            Category::Settings => &mut self.settings,
            Category::System => &mut self.system,
        } += n;
    }
}

impl Item {
    fn new(row: Row, shown: Shown) -> Self {
        Self {
            id: row.id,
            at: row.at,
            cat: row.category.as_str(),
            icon: shown.icon,
            who: shown.who,
            act: shown.act,
            meta: shown.meta,
            warn: shown.warn,
        }
    }
}

#[derive(Debug, Serialize)]
struct Page {
    entries: Vec<Item>,
    counts: Counts,
    /// For `before` of the next page; `null` on the last one.
    next: Option<i64>,
}

fn bad_query(rejection: QueryRejection) -> ApiError {
    ApiError::BadRequest(rejection.body_text())
}

fn audit_entry(category: Category, action: &'static str, user: &AuthUser, ip: ClientIp) -> Entry {
    Entry::new(category, action).actor(user).ip(ip.0)
}

async fn list(
    State(state): State<AppState>,
    AdminUser(user): AdminUser,
    headers: HeaderMap,
    filter: Result<Query<FilterQuery>, QueryRejection>,
    page: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Json<Page>, ApiError> {
    let filter = filter.map_err(bad_query)?.0.parse()?;
    let Query(page) = page.map_err(bad_query)?;
    if !filter.needle.is_empty() {
        // A search counts towards a limit: only from the web app.
        require_requested_with(&headers)?;
        state.limiter.acquire(&SEARCH, Key::User(user.id.clone()))?;
    }
    let limit = page.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let reply = db::call(&state.db, move |conn| {
        let lang = settings::lang_of(conn, &user.id)?;
        let keep = audit_log::keep_days(conn)?;
        // One row more than the page shows whether there is more.
        let mut entries = Vec::new();
        let mut counts = Counts::default();
        if filter.needle.is_empty() {
            for (category, n) in audit_log::counts(conn, keep, &filter.range)? {
                counts.add(category, n);
            }
            let rows = audit_log::page(
                conn,
                keep,
                &filter.range,
                filter.category,
                page.before,
                limit + 1,
            )?;
            for row in rows {
                let shown = texts::render(&row, lang);
                entries.push(Item::new(row, shown));
            }
        } else {
            audit_log::scan(conn, keep, &filter.range, |row| {
                let shown = texts::render(&row, lang);
                if !filter.found(&row, &shown, lang) {
                    return;
                }
                counts.add(row.category, 1);
                let on_page = filter.in_category(&row)
                    && page.before.is_none_or(|before| row.id < before)
                    && entries.len() <= limit;
                if on_page {
                    entries.push(Item::new(row, shown));
                }
            })?;
        }
        let more = entries.len() > limit;
        entries.truncate(limit);
        let next = entries.last().filter(|_| more).map(|item| item.id);
        Ok(Page {
            entries,
            counts,
            next,
        })
    })
    .await?;
    Ok(Json(reply))
}

/// All hits as CSV: semicolon, quotation marks, CRLF, BOM for
/// Excel. The export is itself recorded in the audit log because it carries
/// names and places out of the house; hence `POST` (CSRF check).
async fn export(
    State(state): State<AppState>,
    AdminUser(user): AdminUser,
    ip: ClientIp,
    filter: Result<Query<FilterQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let filter = filter.map_err(bad_query)?.0.parse()?;
    state.limiter.acquire(&EXPORT, Key::User(user.id.clone()))?;
    let csv = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        if let Err(error) = admin_holds(&tx, &user)? {
            return Ok(Err(error));
        }
        let lang = settings::lang_of(&tx, &user.id)?;
        let Some((csv, count)) = build_csv(&tx, &filter, lang, MAX_EXPORT_BYTES)? else {
            return Ok(Err(export_too_large()));
        };
        audit_log::insert(
            &tx,
            &audit_entry(Category::System, "audit.export", &user, ip)
                .details(json!({ "count": count })),
        )?;
        tx.commit()?;
        Ok(Ok(csv))
    })
    .await??;
    let name = Timestamp::now().strftime("audit-log-%Y-%m-%d.csv");
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
        csv,
    )
        .into_response())
}

/// 422 `export_too_large`: the page asks to narrow the time range or the
/// search.
fn export_too_large() -> ApiError {
    ApiError::rejected(StatusCode::UNPROCESSABLE_ENTITY, "export_too_large")
        .with("max_bytes", MAX_EXPORT_BYTES)
}

/// The hits as CSV with the number of rows, or `None` once the CSV would
/// grow past `budget` bytes (then nothing more is kept in memory).
fn build_csv(
    conn: &Connection,
    filter: &Filter,
    lang: Language,
    budget: usize,
) -> anyhow::Result<Option<(String, u64)>> {
    let keep = audit_log::keep_days(conn)?;
    let mut csv = String::from("\u{feff}");
    csv_line(&mut csv, texts::csv_head(lang));
    let mut count = 0_u64;
    let mut over = false;
    audit_log::scan(conn, keep, &filter.range, |row| {
        if over {
            return;
        }
        let shown = texts::render(&row, lang);
        if filter.found(&row, &shown, lang) && filter.in_category(&row) {
            csv_line(
                &mut csv,
                [
                    &row.at,
                    texts::label(row.category, lang),
                    &shown.who,
                    shown.act.trim_start_matches([':', ' ']),
                    &shown.meta,
                ],
            );
            count += 1;
            if csv.len() > budget {
                over = true;
                csv = String::new();
            }
        }
    })?;
    Ok((!over).then_some((csv, count)))
}

fn csv_line<const N: usize>(csv: &mut String, fields: [&str; N]) {
    for (i, field) in fields.into_iter().enumerate() {
        if i > 0 {
            csv.push(';');
        }
        csv_field(csv, field);
    }
    csv.push_str("\r\n");
}

/// A field in quotation marks. Names and titles come from users; whatever
/// starts like a formula gets a `'` in front, otherwise the spreadsheet would
/// execute it on opening.
fn csv_field(csv: &mut String, field: &str) {
    csv.push('"');
    if field.starts_with(['\t', '\r']) || field.trim_start().starts_with(['=', '+', '-', '@']) {
        csv.push('\'');
    }
    csv.push_str(&field.replace('"', "\"\""));
    csv.push('"');
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetentionBody {
    days: u32,
}

async fn show_retention(
    State(state): State<AppState>,
    _: AdminUser,
) -> Result<Json<Value>, ApiError> {
    let days = db::call(&state.db, |conn| audit_log::keep_days(conn)).await?;
    Ok(Json(json!({ "days": days })))
}

/// Changes the retention period. Deletion only happens on the next run of
/// `audit_retention`; an accidentally shorter period can be undone until
/// then.
async fn update_retention(
    State(state): State<AppState>,
    AdminUser(user): AdminUser,
    ip: ClientIp,
    JsonBody(body): JsonBody<RetentionBody>,
) -> Result<Json<Value>, ApiError> {
    if !audit_log::KEEP_CHOICES.contains(&body.days) {
        let mut errors = FieldErrors::default();
        errors.code("days", "invalid");
        return Err(errors.into_error());
    }
    let days = body.days;
    db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        // Still an administrator with a valid session: a shorter period
        // deletes entries on the next cleanup.
        if let Err(error) = admin_holds(&tx, &user)? {
            return Ok(Err(error));
        }
        let previous = audit_log::keep_days(&tx)?;
        if previous != days {
            audit_log::set_keep_days(&tx, days)?;
            audit_log::insert(
                &tx,
                &audit_entry(Category::Settings, "audit.retention", &user, ip)
                    .details(json!({ "days": days, "previous": previous })),
            )?;
            tx.commit()?;
        }
        Ok(Ok(()))
    })
    .await??;
    Ok(Json(json!({ "days": days })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(value: &str) -> String {
        let mut csv = String::new();
        csv_field(&mut csv, value);
        csv
    }

    #[test]
    fn csv_quotes_and_defuses_formulas() {
        assert_eq!(field("Anna Becker"), r#""Anna Becker""#);
        assert_eq!(field(r#"Say "Hello""#), r#""Say ""Hello""""#);
        assert_eq!(field("a;b"), r#""a;b""#);
        for formula in ["=1+1", "+1", "-1", "@SUM(A1)", "\t=1", "\r=1", " =1+1"] {
            assert_eq!(field(formula), format!("\"'{formula}\""));
        }
        assert_eq!(field("a=1"), r#""a=1""#);
    }

    #[test]
    fn csv_line_joins_with_semicolon_and_crlf() {
        let mut csv = String::new();
        csv_line(&mut csv, ["a", "b"]);
        assert_eq!(csv, "\"a\";\"b\"\r\n");
    }

    #[test]
    fn filter_rejects_unknown_category_and_time() {
        let parse = |cat: Option<&str>, from: Option<&str>| {
            FilterQuery {
                cat: cat.map(str::to_owned),
                from: from.map(str::to_owned),
                ..FilterQuery::default()
            }
            .parse()
        };
        assert!(parse(Some("Sign-in"), None).is_err());
        assert!(parse(None, Some("yesterday")).is_err());
        let filter = parse(Some(""), Some("2026-11-05T10:00:00+01:00")).unwrap();
        assert_eq!(filter.category, None);
        assert_eq!(
            filter.range.from.as_deref(),
            Some("2026-11-05T09:00:00.000Z")
        );
        // Sub-millisecond bounds are rounded up, so `.123` is before `.1235`
        // at both ends.
        let range = FilterQuery {
            from: Some("2026-11-05T09:00:00.1235Z".into()),
            to: Some("2026-11-05T09:00:00.1230001Z".into()),
            ..FilterQuery::default()
        }
        .parse()
        .unwrap()
        .range;
        assert_eq!(range.from.as_deref(), Some("2026-11-05T09:00:00.124Z"));
        assert_eq!(range.to.as_deref(), Some("2026-11-05T09:00:00.124Z"));
    }

    #[test]
    fn export_stays_within_its_budget() {
        let conn = crate::db::repo::users::tests::conn();
        for _ in 0..3 {
            audit_log::insert(&conn, &Entry::new(Category::System, "setup.complete")).unwrap();
        }
        let filter = FilterQuery::default().parse().unwrap();
        let (csv, count) = build_csv(&conn, &filter, Language::English, 1024)
            .unwrap()
            .unwrap();
        assert_eq!(count, 3);
        assert_eq!(csv.lines().count(), 4);
        assert!(
            build_csv(&conn, &filter, Language::English, 100)
                .unwrap()
                .is_none(),
            "over the budget"
        );
    }
}
