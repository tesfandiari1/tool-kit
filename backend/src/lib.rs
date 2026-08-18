//! CPU-only HTTP surface for the Tool-Kit conversion service.
//!
//! The current loopback slice exposes authenticated, durable PDF conversion,
//! status polling, and validated Markdown/manifest artifacts in addition to
//! health and capabilities. Accepted jobs, idempotency records, and artifacts
//! live in SQLite and under the data root, so they survive a restart. Non-PDF
//! conversion and remote fallback remain later epic milestones.

pub mod config;
pub mod persistence;
pub mod worker_protocol;

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
