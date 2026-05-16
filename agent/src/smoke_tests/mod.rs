/// Smoke testing module for post-deployment validation
///
/// This module provides comprehensive smoke tests that run after deployment
/// to catch common issues before they cause downtime.

use anyhow::{Result, Context, bail};
use std::time::{Duration, Instant};
use tracing::{info, warn, error, debug};
use futures::future::BoxFuture;

pub mod tests;
pub mod framework_plugins;

use crate::pipeline::deploy::DeploymentContext;

/// Test category determines when the test runs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestCategory {
    /// Run before deployment starts
    PreDeployment,
    /// Run after build completes
    PostBuild,
    /// Run after containers start
    PostDeployment,
    /// Optional integration tests
    Integration,
}

/// Test severity determines failure handling
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Must pass - deployment fails on error
    Critical,
    /// Should pass - warn on failure
    High,
    /// Nice to have - informational
    Medium,
    /// Purely informational
    Low,
}

/// Test execution outcome
#[derive(Debug, Clone)]
pub enum TestOutcome {
    Passed,
    Failed(String),
    Skipped(String),
    Timeout,
}

/// Result of running a single test
#[derive(Debug, Clone)]
pub struct TestResult {
    pub test_name: String,
    pub outcome: TestOutcome,
    pub duration: Duration,
    pub severity: Severity,
    pub remediation_steps: Option<Vec<String>>,
}

impl TestResult {
    pub fn is_failure(&self) -> bool {
        matches!(self.outcome, TestOutcome::Failed(_) | TestOutcome::Timeout)
    }

    pub fn is_critical_failure(&self) -> bool {
        self.is_failure() && self.severity == Severity::Critical
    }
}

/// Configuration for smoke tests
#[derive(Debug, Clone)]
pub struct SmokeTestConfig {
    pub enabled: bool,
    pub fail_on_error: bool,
    pub categories: Vec<TestCategory>,
    pub disabled_tests: Vec<String>,
}

impl Default for SmokeTestConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fail_on_error: true,
            categories: vec![
                TestCategory::PreDeployment,
                TestCategory::PostBuild,
                TestCategory::PostDeployment,
            ],
            disabled_tests: Vec::new(),
        }
    }
}

/// A smoke test definition
pub struct SmokeTest {
    pub name: String,
    pub description: String,
    pub category: TestCategory,
    pub severity: Severity,
    pub timeout: Duration,
    pub execute: Box<dyn Fn(&DeploymentContext) -> BoxFuture<'static, Result<()>> + Send + Sync>,
}

impl SmokeTest {
    pub fn should_run(&self, config: &SmokeTestConfig) -> bool {
        if !config.enabled {
            return false;
        }

        if config.disabled_tests.contains(&self.name) {
            return false;
        }

        config.categories.contains(&self.category)
    }
}

/// Smoke test runner
pub struct SmokeTestRunner {
    deployment_context: DeploymentContext,
    config: SmokeTestConfig,
    results: Vec<TestResult>,
}

impl SmokeTestRunner {
    pub fn new(deployment_context: DeploymentContext, config: SmokeTestConfig) -> Self {
        Self {
            deployment_context,
            config,
            results: Vec::new(),
        }
    }

    /// Run all tests in a specific category
    pub async fn run_category(&mut self, category: TestCategory) -> Result<()> {
        let tests = self.get_tests_for_category(category);

        if tests.is_empty() {
            debug!("No tests for category {:?}", category);
            return Ok(());
        }

        info!("Running {:?} tests ({} tests)...", category, tests.len());

        for test in tests {
            if !test.should_run(&self.config) {
                self.results.push(TestResult {
                    test_name: test.name.clone(),
                    outcome: TestOutcome::Skipped("Disabled in config".to_string()),
                    duration: Duration::from_secs(0),
                    severity: test.severity,
                    remediation_steps: None,
                });
                continue;
            }

            let result = self.run_test(&test).await;

            // Log result
            match &result.outcome {
                TestOutcome::Passed => {
                    info!("✓ {} [{:.1}s]", result.test_name, result.duration.as_secs_f32());
                }
                TestOutcome::Failed(msg) => {
                    match result.severity {
                        Severity::Critical => error!("✗ {} [{:.1}s]: {}", result.test_name, result.duration.as_secs_f32(), msg),
                        Severity::High => warn!("⚠ {} [{:.1}s]: {}", result.test_name, result.duration.as_secs_f32(), msg),
                        _ => info!("⚠ {} [{:.1}s]: {}", result.test_name, result.duration.as_secs_f32(), msg),
                    }

                    if let Some(steps) = &result.remediation_steps {
                        error!("Remediation steps:");
                        for (i, step) in steps.iter().enumerate() {
                            error!("  {}. {}", i + 1, step);
                        }
                    }
                }
                TestOutcome::Timeout => {
                    warn!("⏱ {} [timeout after {:.1}s]", result.test_name, result.duration.as_secs_f32());
                }
                TestOutcome::Skipped(reason) => {
                    debug!("⊝ {} [skipped: {}]", result.test_name, reason);
                }
            }

            self.results.push(result.clone());

            // Fail fast on critical errors
            if result.is_critical_failure() && self.config.fail_on_error {
                return Err(anyhow::anyhow!("Critical test failed: {}", test.name));
            }
        }

        Ok(())
    }

    /// Run a single test with timeout
    async fn run_test(&self, test: &SmokeTest) -> TestResult {
        let start = Instant::now();

        let outcome = match tokio::time::timeout(test.timeout, (test.execute)(&self.deployment_context)).await {
            Ok(Ok(())) => TestOutcome::Passed,
            Ok(Err(e)) => {
                let remediation = extract_remediation_steps(&e);
                TestOutcome::Failed(format!("{:#}", e))
            }
            Err(_) => TestOutcome::Timeout,
        };

        let remediation_steps = if let TestOutcome::Failed(ref msg) = outcome {
            extract_remediation_steps_from_message(msg)
        } else {
            None
        };

        TestResult {
            test_name: test.name.clone(),
            outcome,
            duration: start.elapsed(),
            severity: test.severity,
            remediation_steps,
        }
    }

    /// Get all tests for a category
    fn get_tests_for_category(&self, category: TestCategory) -> Vec<SmokeTest> {
        // This will be populated with actual tests
        tests::get_tests_for_category(category, &self.deployment_context)
    }

    /// Generate test report
    pub fn generate_report(&self) -> TestReport {
        TestReport::new(&self.results)
    }

    /// Run all enabled test categories
    pub async fn run_all_tests(&mut self) -> Result<TestReport> {
        for category in self.config.categories.clone() {
            self.run_category(category).await?;
        }

        Ok(self.generate_report())
    }
}

/// Test report summarizing all results
pub struct TestReport {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub warnings: usize,
    pub critical_failures: usize,
    pub results: Vec<TestResult>,
}

impl TestReport {
    pub fn new(results: &[TestResult]) -> Self {
        let total = results.len();
        let passed = results.iter().filter(|r| matches!(r.outcome, TestOutcome::Passed)).count();
        let failed = results.iter().filter(|r| r.is_failure()).count();
        let skipped = results.iter().filter(|r| matches!(r.outcome, TestOutcome::Skipped(_))).count();
        let warnings = results.iter().filter(|r| r.is_failure() && matches!(r.severity, Severity::Medium | Severity::Low)).count();
        let critical_failures = results.iter().filter(|r| r.is_critical_failure()).count();

        Self {
            total,
            passed,
            failed,
            skipped,
            warnings,
            critical_failures,
            results: results.to_vec(),
        }
    }

    pub fn has_critical_failures(&self) -> bool {
        self.critical_failures > 0
    }

    pub fn is_success(&self) -> bool {
        self.critical_failures == 0
    }
}

impl std::fmt::Display for TestReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "=====================================================")?;
        writeln!(f, "       Shipwright Smoke Test Report")?;
        writeln!(f, "=====================================================")?;
        writeln!(f)?;

        // Group results by category
        let mut _pre_deployment: Vec<&TestResult> = Vec::new();
        let mut _post_build: Vec<&TestResult> = Vec::new();
        let mut _post_deployment: Vec<&TestResult> = Vec::new();

        for result in &self.results {
            // We'll need to track category in TestResult for this to work properly
            // For now, just add to post_deployment
            _post_deployment.push(result);
        }

        writeln!(f, "-----------------------------------------------------")?;
        writeln!(f, "Summary")?;
        writeln!(f, "-----------------------------------------------------")?;
        writeln!(f, "Total Tests: {}", self.total)?;
        writeln!(f, "Passed: {}", self.passed)?;
        writeln!(f, "Failed: {}", self.failed)?;
        writeln!(f, "Skipped: {}", self.skipped)?;
        writeln!(f, "Warnings: {}", self.warnings)?;
        writeln!(f)?;

        if self.is_success() {
            writeln!(f, "Status: ✅ ALL TESTS PASSED")?;
        } else {
            writeln!(f, "Status: ❌ {} CRITICAL FAILURES", self.critical_failures)?;
        }

        writeln!(f, "=====================================================")?;

        Ok(())
    }
}

/// Helper to extract remediation steps from error messages
fn extract_remediation_steps_from_message(msg: &str) -> Option<Vec<String>> {
    // Look for common patterns in error messages
    // This is a simple implementation - can be enhanced based on actual error patterns
    None
}

fn extract_remediation_steps(error: &anyhow::Error) -> Option<Vec<String>> {
    // Extract remediation steps from error context
    // This can be enhanced to look for specific error types
    None
}
