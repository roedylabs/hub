//! Health of the paired apps (every 30 s; docs/plan.md › Keys and events):
//! `GET /<slug>/readyz` at each app's internal address, all at once, stored
//! in `apps.health` and `health_at` and in the app table. A change between
//! online and offline is logged.

use std::{sync::Arc, time::Duration};

use tokio::task::JoinSet;

use super::Job;
use crate::{
    api,
    app::AppState,
    apps::{self, App},
    db::{self, repo::apps::Health},
};

/// Interval of the checks.
pub const EVERY: Duration = Duration::from_secs(30);

pub fn job() -> Job<AppState> {
    Job {
        name: "health_poll",
        first: Duration::from_secs(5),
        every: EVERY,
        run: |state| Box::pin(async move { run(&state).await.map(|_| ()) }),
    }
}

/// One run: checks every paired app and returns their health by slug.
pub async fn run(state: &AppState) -> anyhow::Result<Vec<(String, Health)>> {
    check(state, state.apps.paired()).await
}

/// Checks one app right away (after pairing); nothing if it is not paired.
pub async fn check_one(state: &AppState, slug: &str) -> anyhow::Result<Option<Health>> {
    let Some(app) = state
        .apps
        .get(slug)
        .filter(|app| app.status == db::repo::apps::Status::Paired)
    else {
        return Ok(None);
    };
    let results = check(state, vec![app]).await?;
    Ok(results.into_iter().next().map(|(_, health)| health))
}

async fn check(state: &AppState, apps: Vec<Arc<App>>) -> anyhow::Result<Vec<(String, Health)>> {
    if apps.is_empty() {
        return Ok(Vec::new());
    }
    let mut checks = JoinSet::new();
    for app in apps {
        let client = state.probe.clone();
        checks.spawn(async move {
            let health = apps::check_health(&client, &app).await;
            (app, health)
        });
    }
    let mut results = Vec::new();
    while let Some(result) = checks.join_next().await {
        results.push(result?);
    }
    for (app, health) in &results {
        if app.health != Some(*health) {
            tracing::info!(
                slug = app.slug,
                health = health.as_str(),
                "app health changed"
            );
        }
    }
    let stored: Vec<(String, Health)> = results
        .iter()
        .map(|(app, health)| (app.id.clone(), *health))
        .collect();
    let table = state.apps.clone();
    db::call(&state.db, move |conn| {
        for (id, health) in &stored {
            db::repo::apps::set_health(conn, id, *health)?;
        }
        api::apps::reload(&table, conn);
        Ok(())
    })
    .await?;
    let mut by_slug: Vec<(String, Health)> = results
        .into_iter()
        .map(|(app, health)| (app.slug.clone(), health))
        .collect();
    by_slug.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(by_slug)
}
