//! Domain model of a Service Builder project.
//!
//! A [`Project`] is what the visual editor edits and what `sb-codegen` turns into a
//! Cargo workspace of event-driven Rust services (gRPC for RPC, Kafka for events).

pub mod fragments;
pub mod maildesign;
pub mod names;
pub mod pods;
pub mod project;
pub mod sample;
pub mod typing;
pub mod validate;

#[cfg(test)]
mod journal_tests;

pub use maildesign::{
    MailAlign, MailBlock, MailColumn, MailDesign, MailFont, MailSettings, render_html, render_text,
};
pub use pods::{Frontend, PodsConfig, TraefikConfig, TraefikHosts};
pub use project::*;
pub use validate::{Diagnostic, Severity, has_errors, validate};
