//! CPU-only HTTP surface for the Tool-Kit conversion service.
//!
//! The current loopback slice exposes authenticated, durable PDF conversion,
//! status polling, and validated Markdown/manifest artifacts in addition to
//! health and capabilities. Accepted jobs, idempotency records, and artifacts
//! live in SQLite and under the data root, so they survive a restart. Non-PDF
//! conversion and remote fallback remain later epic milestones.

pub mod config;
pub mod faults;
pub mod persistence;

// The worker wire contracts moved to their own crate so the workers coming
// after Vision cannot each grow a private copy. Re-exported under the names
// they already had, because every call site here and in the worker binaries
// names them that way and a rename would be churn, not a change.
pub use tool_kit_worker_protocol::{pdf as worker_protocol, vision as vision_protocol};

mod api;
mod app;
mod artifacts;
mod auth;
mod conversion;
mod engines;
mod error;
mod jobs;

pub use api::router;
pub use app::AppState;
