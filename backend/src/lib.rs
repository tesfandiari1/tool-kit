//! CPU-only HTTP surface for the Tool-Kit conversion service.
//!
//! The current loopback slice exposes authenticated, ephemeral PDF conversion,
//! status polling, and validated Markdown/manifest artifacts in addition to
//! health and capabilities. Durable SQLite state, non-PDF conversion, and
//! remote fallback remain later epic milestones.

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
