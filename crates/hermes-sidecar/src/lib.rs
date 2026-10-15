//! Attaches to a running Hermes webapp or spawns one, and stops it again.
//!
//! Blocking by design: call [`connect`] and [`Backend::shutdown`] from a background thread.

mod backend;
mod config;
mod connect;
mod error;
mod health;
mod launch_link;
mod output;
mod phase;
mod process;

pub use backend::{Backend, ShutdownReport};
pub use config::SidecarConfig;
pub use connect::{connect, connect_with_cancel, Cancel};
pub use error::SidecarError;
pub use launch_link::{is_attach_url, redact};
pub use phase::Phase;
