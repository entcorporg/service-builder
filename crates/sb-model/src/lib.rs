//! Domain model of a Service Builder project.
//!
//! A [`Project`] is what the visual editor edits and what `sb-codegen` turns into a
//! Cargo workspace of event-driven Rust services (gRPC for RPC, Kafka for events).

pub mod names;
pub mod project;
pub mod sample;
pub mod typing;
pub mod validate;

pub use project::*;
pub use validate::{Diagnostic, Severity, has_errors, validate};
