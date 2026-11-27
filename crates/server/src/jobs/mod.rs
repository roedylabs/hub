//! Background jobs of `roedy-hub serve`: [`scheduler`] runs them, [`start`]
//! registers them.
//!
//! Taken over from Roedy Mail (`crates/server/src/jobs/mod.rs`) with the two
//! cleanup jobs the hub needs, plus the rotation of the signing keys, the
//! health checks of the apps, the end of the streams of ended sessions and
//! the retention of the events. The event delivery itself runs apart from
//! these (`events::start`).

pub mod audit_retention;
pub mod event_retention;
pub mod gateway_streams;
pub mod health_poll;
pub mod key_rotation;
pub mod scheduler;
pub mod session_cleanup;

use crate::app::AppState;
pub use scheduler::{Job, Scheduler};

/// Starts the server's jobs; [`Scheduler::shutdown`] stops them.
pub fn start(state: &AppState) -> Scheduler {
    Scheduler::start(
        state,
        [
            session_cleanup::job(),
            audit_retention::job(),
            event_retention::job(),
            key_rotation::job(),
            health_poll::job(),
            gateway_streams::job(),
        ],
    )
}
