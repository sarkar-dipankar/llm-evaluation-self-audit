//! Integration and regression tests for PromptDbg.
//!
//! This crate contains:
//! - IR stability tests (ensure IR generation is deterministic)
//! - Runtime trace tests (verify execution behavior)
//! - Template rendering regression tests
//! - Provider adapter behavior tests
//! - End-to-end workflow tests

pub mod fixtures;
pub mod ir_stability;
pub mod runtime_trace;
pub mod template_regression;

/// Test fixture helper to load test prompt files.
pub fn load_fixture(name: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let path = format!("{}/fixtures/{}", manifest_dir, name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to load fixture {}: {}", name, e))
}

/// Test fixture helper to load expected JSON output.
pub fn load_expected_json(name: &str) -> serde_json::Value {
    let content = load_fixture(name);
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("Failed to parse fixture {} as JSON: {}", name, e))
}
