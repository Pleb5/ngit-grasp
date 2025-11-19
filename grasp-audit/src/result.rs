//! Test result types

use std::time::{Duration, Instant};

/// Result of a single test
#[derive(Debug, Clone)]
pub struct TestResult {
    pub name: String,
    pub spec_ref: String,
    pub requirement: String,
    pub passed: bool,
    pub error: Option<String>,
    pub duration: Duration,
}

impl TestResult {
    /// Create a new test result
    pub fn new(name: &str, spec_ref: &str, requirement: &str) -> Self {
        TestResult {
            name: name.to_string(),
            spec_ref: spec_ref.to_string(),
            requirement: requirement.to_string(),
            passed: false,
            error: None,
            duration: Duration::default(),
        }
    }

    /// Run a test function and capture the result
    pub async fn run<F, Fut>(mut self, test_fn: F) -> Self
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(), String>>,
    {
        let start = Instant::now();

        match test_fn().await {
            Ok(()) => {
                self.passed = true;
            }
            Err(e) => {
                self.passed = false;
                self.error = Some(e);
            }
        }

        self.duration = start.elapsed();
        self
    }

    /// Mark test as passed
    pub fn pass(mut self) -> Self {
        self.passed = true;
        self
    }

    /// Mark test as failed with error
    pub fn fail(mut self, error: impl Into<String>) -> Self {
        self.passed = false;
        self.error = Some(error.into());
        self
    }
}

/// Collection of test results for a spec
#[derive(Debug, Clone)]
pub struct AuditResult {
    pub spec: String,
    pub results: Vec<TestResult>,
}

impl AuditResult {
    /// Create a new audit result
    pub fn new(spec: impl Into<String>) -> Self {
        Self {
            spec: spec.into(),
            results: Vec::new(),
        }
    }

    /// Add a test result
    pub fn add(&mut self, result: TestResult) {
        self.results.push(result);
    }

    /// Merge another audit result
    pub fn merge(&mut self, other: AuditResult) {
        self.results.extend(other.results);
    }

    /// Check if all tests passed
    pub fn all_passed(&self) -> bool {
        self.results.iter().all(|r| r.passed)
    }

    /// Get count of passed tests
    pub fn passed_count(&self) -> usize {
        self.results.iter().filter(|r| r.passed).count()
    }

    /// Get count of failed tests
    pub fn failed_count(&self) -> usize {
        self.results.iter().filter(|r| !r.passed).count()
    }

    /// Get total count of tests
    pub fn total_count(&self) -> usize {
        self.results.len()
    }

    /// Print a detailed report
    pub fn print_report(&self) {
        println!("\n{}", self.spec);
        println!("{}", "═".repeat(60));
        println!();

        let passed = self.passed_count();
        let total = self.total_count();

        for result in &self.results {
            let status = if result.passed { "✓" } else { "✗" };

            println!("{} {} ({})", status, result.name, result.spec_ref);
            println!("  Requirement: {}", result.requirement);

            if let Some(error) = &result.error {
                println!("  Error: {}", error);
            }

            println!("  Duration: {:?}", result.duration);
            println!();
        }

        println!(
            "Results: {}/{} passed ({:.1}%)",
            passed,
            total,
            (passed as f64 / total as f64) * 100.0
        );
        println!();
    }

    /// Get a summary string
    pub fn summary(&self) -> String {
        format!(
            "{}: {}/{} passed",
            self.spec,
            self.passed_count(),
            self.total_count()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_result_pass() {
        let result = TestResult::new("test", "SPEC:1", "Must work")
            .run(|| async { Ok(()) })
            .await;

        assert!(result.passed);
        assert!(result.error.is_none());
    }

    #[tokio::test]
    async fn test_result_fail() {
        let result = TestResult::new("test", "SPEC:1", "Must work")
            .run(|| async { Err("Failed".to_string()) })
            .await;

        assert!(!result.passed);
        assert_eq!(result.error, Some("Failed".to_string()));
    }

    #[test]
    fn test_audit_result() {
        let mut audit = AuditResult::new("Test Spec");

        audit.add(TestResult::new("test1", "SPEC:1", "Req1").pass());
        audit.add(TestResult::new("test2", "SPEC:2", "Req2").fail("Error"));

        assert_eq!(audit.total_count(), 2);
        assert_eq!(audit.passed_count(), 1);
        assert_eq!(audit.failed_count(), 1);
        assert!(!audit.all_passed());
    }
}
