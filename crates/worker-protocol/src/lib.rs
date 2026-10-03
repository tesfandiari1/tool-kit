//! Wire contracts for the workers the converter spawns.
//!
//! One module per engine, and they share no types. A PDF report carries an
//! inspection and a Vision report does not, and each engine pins its own
//! identity handshake, which is what rejects a stale worker binary. What they
//! share is a home. The audio worker is the third engine, and copies of these
//! conventions drifting apart across one file per engine is the failure this
//! crate prevents.
//!
//! serde is the only dependency, and it stays that way. Every worker binary
//! links this crate, so anything added here is linked into all of them.

pub mod anydoc;
pub mod audio;
pub mod pdf;
pub mod vision;
