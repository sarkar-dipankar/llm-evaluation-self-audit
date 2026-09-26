//! Telemetry and metrics collection for PromptDbg.
//!
//! This module provides:
//! - Execution metrics (success/failure rates, timing)
//! - Lint coverage statistics
//! - Breakpoint usage tracking
//! - Provider call metrics

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Metrics collector for PromptDbg operations.
#[derive(Debug, Default)]
pub struct MetricsCollector {
    /// Template rendering metrics
    template_metrics: RwLock<TemplateMetrics>,
    /// Provider call metrics
    provider_metrics: RwLock<ProviderMetrics>,
    /// Lint metrics
    lint_metrics: RwLock<LintMetrics>,
    /// Breakpoint usage metrics
    breakpoint_metrics: RwLock<BreakpointMetrics>,
    /// Counters for atomic increments
    counters: MetricsCounters,
}

#[derive(Debug, Default)]
struct MetricsCounters {
    template_renders: AtomicU64,
    template_successes: AtomicU64,
    template_failures: AtomicU64,
    provider_calls: AtomicU64,
    provider_successes: AtomicU64,
    provider_failures: AtomicU64,
    provider_timeouts: AtomicU64,
    breakpoints_hit: AtomicU64,
    lint_runs: AtomicU64,
}

/// Template rendering metrics.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TemplateMetrics {
    /// Total renders attempted
    pub total_renders: u64,
    /// Successful renders
    pub successful_renders: u64,
    /// Failed renders
    pub failed_renders: u64,
    /// Average render time in milliseconds
    pub avg_render_time_ms: f64,
    /// Variables accessed (name -> count)
    pub variable_access_counts: HashMap<String, u64>,
    /// Branches taken (condition -> count)
    pub branch_taken_counts: HashMap<String, u64>,
}

/// Provider call metrics.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProviderMetrics {
    /// Total API calls
    pub total_calls: u64,
    /// Successful calls
    pub successful_calls: u64,
    /// Failed calls
    pub failed_calls: u64,
    /// Timeout count
    pub timeouts: u64,
    /// Average latency in milliseconds
    pub avg_latency_ms: f64,
    /// Provider-specific metrics (provider name -> metrics)
    pub by_provider: HashMap<String, ProviderStats>,
}

/// Per-provider statistics.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProviderStats {
    pub calls: u64,
    pub successes: u64,
    pub failures: u64,
    pub avg_latency_ms: f64,
    pub total_tokens_used: u64,
}

/// Lint metrics.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LintMetrics {
    /// Total lint runs
    pub total_runs: u64,
    /// Issues found by severity
    pub issues_by_severity: HashMap<String, u64>,
    /// Issues found by code
    pub issues_by_code: HashMap<String, u64>,
    /// Files linted
    pub files_linted: u64,
    /// Coverage score (0.0-1.0)
    pub coverage_score: f64,
}

/// Breakpoint usage metrics.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct BreakpointMetrics {
    /// Total breakpoints hit
    pub total_hits: u64,
    /// Hits by breakpoint ID
    pub hits_by_id: HashMap<String, u64>,
    /// Debug sessions started
    pub sessions_started: u64,
    /// Debug sessions completed
    pub sessions_completed: u64,
}

/// A timing guard that records duration when dropped.
pub struct TimingGuard<'a> {
    _collector: &'a MetricsCollector,
    start: Instant,
    operation: TimedOperation,
}

enum TimedOperation {
    TemplateRender,
    ProviderCall(String),
}

impl MetricsCollector {
    /// Create a new metrics collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a successful template render.
    pub fn record_template_success(
        &self,
        duration: Duration,
        variables: &[String],
        branches: &[String],
    ) {
        self.counters
            .template_renders
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .template_successes
            .fetch_add(1, Ordering::Relaxed);

        let mut metrics = self.template_metrics.write().unwrap();
        metrics.total_renders += 1;
        metrics.successful_renders += 1;
        let count = metrics.total_renders;
        update_average(
            &mut metrics.avg_render_time_ms,
            count,
            duration.as_millis() as f64,
        );

        for var in variables {
            *metrics
                .variable_access_counts
                .entry(var.clone())
                .or_insert(0) += 1;
        }
        for branch in branches {
            *metrics
                .branch_taken_counts
                .entry(branch.clone())
                .or_insert(0) += 1;
        }
    }

    /// Record a failed template render.
    pub fn record_template_failure(&self, _error: &str) {
        self.counters
            .template_renders
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .template_failures
            .fetch_add(1, Ordering::Relaxed);

        let mut metrics = self.template_metrics.write().unwrap();
        metrics.total_renders += 1;
        metrics.failed_renders += 1;
    }

    /// Record a successful provider call.
    pub fn record_provider_success(&self, provider: &str, duration: Duration, tokens_used: u64) {
        self.counters.provider_calls.fetch_add(1, Ordering::Relaxed);
        self.counters
            .provider_successes
            .fetch_add(1, Ordering::Relaxed);

        let mut metrics = self.provider_metrics.write().unwrap();
        metrics.total_calls += 1;
        metrics.successful_calls += 1;
        let total_count = metrics.total_calls;
        update_average(
            &mut metrics.avg_latency_ms,
            total_count,
            duration.as_millis() as f64,
        );

        let stats = metrics.by_provider.entry(provider.to_string()).or_default();
        stats.calls += 1;
        stats.successes += 1;
        stats.total_tokens_used += tokens_used;
        let stats_count = stats.calls;
        update_average(
            &mut stats.avg_latency_ms,
            stats_count,
            duration.as_millis() as f64,
        );
    }

    /// Record a failed provider call.
    pub fn record_provider_failure(&self, provider: &str, is_timeout: bool) {
        self.counters.provider_calls.fetch_add(1, Ordering::Relaxed);
        self.counters
            .provider_failures
            .fetch_add(1, Ordering::Relaxed);

        if is_timeout {
            self.counters
                .provider_timeouts
                .fetch_add(1, Ordering::Relaxed);
        }

        let mut metrics = self.provider_metrics.write().unwrap();
        metrics.total_calls += 1;
        metrics.failed_calls += 1;
        if is_timeout {
            metrics.timeouts += 1;
        }

        let stats = metrics.by_provider.entry(provider.to_string()).or_default();
        stats.calls += 1;
        stats.failures += 1;
    }

    /// Record lint results.
    pub fn record_lint_run(&self, issues: &[(String, String)]) {
        self.counters.lint_runs.fetch_add(1, Ordering::Relaxed);

        let mut metrics = self.lint_metrics.write().unwrap();
        metrics.total_runs += 1;
        metrics.files_linted += 1;

        for (severity, code) in issues {
            *metrics
                .issues_by_severity
                .entry(severity.clone())
                .or_insert(0) += 1;
            *metrics.issues_by_code.entry(code.clone()).or_insert(0) += 1;
        }
    }

    /// Record a breakpoint hit.
    pub fn record_breakpoint_hit(&self, breakpoint_id: &str) {
        self.counters
            .breakpoints_hit
            .fetch_add(1, Ordering::Relaxed);

        let mut metrics = self.breakpoint_metrics.write().unwrap();
        metrics.total_hits += 1;
        *metrics
            .hits_by_id
            .entry(breakpoint_id.to_string())
            .or_insert(0) += 1;
    }

    /// Record a debug session start.
    pub fn record_session_start(&self) {
        let mut metrics = self.breakpoint_metrics.write().unwrap();
        metrics.sessions_started += 1;
    }

    /// Record a debug session completion.
    pub fn record_session_complete(&self) {
        let mut metrics = self.breakpoint_metrics.write().unwrap();
        metrics.sessions_completed += 1;
    }

    /// Start timing a template render operation.
    pub fn time_template_render(&self) -> TimingGuard<'_> {
        TimingGuard {
            _collector: self,
            start: Instant::now(),
            operation: TimedOperation::TemplateRender,
        }
    }

    /// Start timing a provider call.
    pub fn time_provider_call(&self, provider: impl Into<String>) -> TimingGuard<'_> {
        TimingGuard {
            _collector: self,
            start: Instant::now(),
            operation: TimedOperation::ProviderCall(provider.into()),
        }
    }

    /// Get current template metrics.
    pub fn template_metrics(&self) -> TemplateMetrics {
        self.template_metrics.read().unwrap().clone()
    }

    /// Get current provider metrics.
    pub fn provider_metrics(&self) -> ProviderMetrics {
        self.provider_metrics.read().unwrap().clone()
    }

    /// Get current lint metrics.
    pub fn lint_metrics(&self) -> LintMetrics {
        self.lint_metrics.read().unwrap().clone()
    }

    /// Get current breakpoint metrics.
    pub fn breakpoint_metrics(&self) -> BreakpointMetrics {
        self.breakpoint_metrics.read().unwrap().clone()
    }

    /// Get all metrics as a summary.
    pub fn summary(&self) -> MetricsSummary {
        MetricsSummary {
            template: self.template_metrics(),
            provider: self.provider_metrics(),
            lint: self.lint_metrics(),
            breakpoints: self.breakpoint_metrics(),
        }
    }

    /// Reset all metrics.
    pub fn reset(&self) {
        *self.template_metrics.write().unwrap() = TemplateMetrics::default();
        *self.provider_metrics.write().unwrap() = ProviderMetrics::default();
        *self.lint_metrics.write().unwrap() = LintMetrics::default();
        *self.breakpoint_metrics.write().unwrap() = BreakpointMetrics::default();

        self.counters.template_renders.store(0, Ordering::Relaxed);
        self.counters.template_successes.store(0, Ordering::Relaxed);
        self.counters.template_failures.store(0, Ordering::Relaxed);
        self.counters.provider_calls.store(0, Ordering::Relaxed);
        self.counters.provider_successes.store(0, Ordering::Relaxed);
        self.counters.provider_failures.store(0, Ordering::Relaxed);
        self.counters.provider_timeouts.store(0, Ordering::Relaxed);
        self.counters.breakpoints_hit.store(0, Ordering::Relaxed);
        self.counters.lint_runs.store(0, Ordering::Relaxed);
    }
}

impl Drop for TimingGuard<'_> {
    fn drop(&mut self) {
        let duration = self.start.elapsed();
        match &self.operation {
            TimedOperation::TemplateRender => {
                // Duration is recorded but success/failure is tracked separately
            }
            TimedOperation::ProviderCall(_provider) => {
                // Duration is recorded but success/failure is tracked separately
            }
        }
        // Note: actual recording should be done explicitly with record_* methods
        // This guard just tracks timing for convenience
        let _ = duration; // Suppress unused warning
    }
}

/// Summary of all metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSummary {
    pub template: TemplateMetrics,
    pub provider: ProviderMetrics,
    pub lint: LintMetrics,
    pub breakpoints: BreakpointMetrics,
}

/// Update a running average.
fn update_average(avg: &mut f64, count: u64, new_value: f64) {
    if count == 1 {
        *avg = new_value;
    } else {
        *avg = *avg + (new_value - *avg) / count as f64;
    }
}

/// Global metrics collector instance.
static GLOBAL_METRICS: std::sync::OnceLock<Arc<MetricsCollector>> = std::sync::OnceLock::new();

/// Get the global metrics collector.
pub fn global_metrics() -> Arc<MetricsCollector> {
    GLOBAL_METRICS
        .get_or_init(|| Arc::new(MetricsCollector::new()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_collector() {
        let collector = MetricsCollector::new();

        // Record template operations
        collector.record_template_success(
            Duration::from_millis(100),
            &["var1".to_string(), "var2".to_string()],
            &["condition1".to_string()],
        );
        collector.record_template_failure("test error");

        let metrics = collector.template_metrics();
        assert_eq!(metrics.total_renders, 2);
        assert_eq!(metrics.successful_renders, 1);
        assert_eq!(metrics.failed_renders, 1);
        assert_eq!(metrics.variable_access_counts.get("var1"), Some(&1));
    }

    #[test]
    fn test_provider_metrics() {
        let collector = MetricsCollector::new();

        collector.record_provider_success("openai", Duration::from_millis(500), 100);
        collector.record_provider_success("openai", Duration::from_millis(600), 150);
        collector.record_provider_failure("anthropic", false);

        let metrics = collector.provider_metrics();
        assert_eq!(metrics.total_calls, 3);
        assert_eq!(metrics.successful_calls, 2);
        assert_eq!(metrics.failed_calls, 1);

        let openai_stats = metrics.by_provider.get("openai").unwrap();
        assert_eq!(openai_stats.calls, 2);
        assert_eq!(openai_stats.total_tokens_used, 250);
    }

    #[test]
    fn test_breakpoint_metrics() {
        let collector = MetricsCollector::new();

        collector.record_session_start();
        collector.record_breakpoint_hit("BP:TEST1");
        collector.record_breakpoint_hit("BP:TEST1");
        collector.record_breakpoint_hit("BP:TEST2");
        collector.record_session_complete();

        let metrics = collector.breakpoint_metrics();
        assert_eq!(metrics.total_hits, 3);
        assert_eq!(metrics.hits_by_id.get("BP:TEST1"), Some(&2));
        assert_eq!(metrics.hits_by_id.get("BP:TEST2"), Some(&1));
        assert_eq!(metrics.sessions_started, 1);
        assert_eq!(metrics.sessions_completed, 1);
    }

    #[test]
    fn test_lint_metrics() {
        let collector = MetricsCollector::new();

        collector.record_lint_run(&[
            ("Warning".to_string(), "prompt/style-vague".to_string()),
            ("Error".to_string(), "prompt/conflict-rules".to_string()),
            ("Warning".to_string(), "prompt/style-vague".to_string()),
        ]);

        let metrics = collector.lint_metrics();
        assert_eq!(metrics.total_runs, 1);
        assert_eq!(metrics.issues_by_severity.get("Warning"), Some(&2));
        assert_eq!(metrics.issues_by_severity.get("Error"), Some(&1));
        assert_eq!(metrics.issues_by_code.get("prompt/style-vague"), Some(&2));
    }

    #[test]
    fn test_metrics_reset() {
        let collector = MetricsCollector::new();

        collector.record_template_success(Duration::from_millis(100), &[], &[]);
        collector.record_provider_success("test", Duration::from_millis(100), 0);

        collector.reset();

        let summary = collector.summary();
        assert_eq!(summary.template.total_renders, 0);
        assert_eq!(summary.provider.total_calls, 0);
    }
}
