//! Background jobs of `roedy-hub serve`: [`scheduler`] runs them, [`start`]
//! registers them.
//!
//! Taken over from Roedy Mail (`crates/server/src/jobs/mod.rs`) with the two
//! cleanup jobs the hub needs.

pub mod audit_retention;
pub mod scheduler;
pub mod session_cleanup;

use crate::app::AppState;
pub use scheduler::{Job, Scheduler};

/// Starts the server's jobs; [`Scheduler::shutdown`] stops them.
pub fn start(state: &AppState) -> Scheduler {
    Scheduler::start(state, [session_cleanup::job(), audit_retention::job()])
}
