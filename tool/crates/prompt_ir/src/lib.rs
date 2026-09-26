//! Shared IR structs and data models for PromptDbg.
//!
//! This crate defines the core types used across all PromptDbg components:
//! - `PromptIR` - Intermediate representation of prompt files
//! - `ProviderConfig` - LLM provider configuration
//! - `PromptTrace` - Runtime execution traces
//! - Diagnostics, test cases, and breakpoint suggestions

// Re-export all types at crate root
mod config;
mod diagnostic;
mod ir;
mod trace;

pub use config::*;
pub use diagnostic::*;
pub use ir::*;
pub use trace::*;
