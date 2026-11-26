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
    db::{
        self,
        repo::apps::{self as repo, Health},
    },
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
    let stored: Vec<(String, Health)> = results
        .iter()
        .map(|(app, health)| (app.id.clone(), *health))
        .collect();
    let table = state.apps.clone();
    // One transaction: the table never shows a part of the results.
    let changed = db::call(&state.db, move |conn| {
        let tx = conn.transaction()?;
        let mut changed = Vec::new();
        for (id, health) in &stored {
            let before = repo::by_id(&tx, id)?.map(|app| app.health);
            if repo::set_health(&tx, id, *health)? {
                let before = before.flatten();
                if before != Some(*health) {
                    changed.push((id.clone(), *health));
                }
            }
        }
        tx.commit()?;
        api::apps::reload(&table, conn);
        Ok(changed)
    })
    .await?;
    for (id, health) in changed {
        if let Some((app, _)) = results.iter().find(|(app, _)| app.id == id) {
            tracing::info!(
                slug = app.slug,
                health = health.as_str(),
                "app health changed"
            );
        }
    }
    let mut by_slug: Vec<(String, Health)> = results
        .into_iter()
        .map(|(app, health)| (app.slug.clone(), health))
        .collect();
    by_slug.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(by_slug)
}
