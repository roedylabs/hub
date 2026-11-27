//! `GET /internal/v1/events?after=<seq>&limit=<n>` (docs/app-contract.md ›
//! Events › Catching up): the calling app's events after `after`, oldest
//! first, each signed anew; `410 events_expired` with `resume_after` when
//! events after `after` were deleted already (retention).
//!
//! A catch-up shows that the app is up: its delivery tries again at once,
//! and continues after `after`, which the app has processed.

use axum::{
    Json,
    extract::{Query, State, rejection::QueryRejection},
    http::StatusCode,
};
use jiff::Timestamp;
use roedy_hub_contract::pairing::EventPage;
use serde::Deserialize;

use super::PairedApp;
use crate::{
    api::ApiError,
    app::AppState,
    db::{
        self,
        repo::events::{self as repo, Page},
    },
    events,
};

/// Events per page without `limit`, and the most.
pub const MAX_LIMIT: usize = 100;

#[derive(Deserialize)]
pub struct Range {
    after: Option<u64>,
    limit: Option<usize>,
}

pub async fn list(
    State(state): State<AppState>,
    PairedApp(app, _): PairedApp,
    range: Result<Query<Range>, QueryRejection>,
) -> Result<Json<EventPage>, ApiError> {
    let Query(range) = range.map_err(|rejection| ApiError::BadRequest(rejection.body_text()))?;
    let limit = range.limit.unwrap_or(MAX_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(ApiError::BadRequest(format!(
            "limit must be 1 to {MAX_LIMIT}"
        )));
    }
    let after = range.after.unwrap_or(0);
    let app_id = app.id.clone();
    let (page, moved) = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        let page = repo::page(&tx, &app_id, after, limit)?;
        let mut moved = repo::resume(&tx, &app_id, false)?;
        if matches!(page, Page::Events { .. }) {
            moved |= repo::acknowledge(&tx, &app_id, after)?;
        }
        tx.commit()?;
        Ok((page, moved))
    })
    .await?;
    if moved {
        state.events.wake();
    }
    match page {
        Page::Expired { resume_after } => {
            Err(ApiError::rejected(StatusCode::GONE, "events_expired")
                .with("resume_after", resume_after))
        }
        Page::Events {
            events: stored,
            more,
        } => {
            let now = Timestamp::now().as_second();
            let events = stored
                .iter()
                .map(|event| events::sign(&state, &app.slug, event, now))
                .collect::<anyhow::Result<_>>()?;
            Ok(Json(EventPage { events, more }))
        }
    }
}
