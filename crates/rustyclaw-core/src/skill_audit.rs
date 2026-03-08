//! Skill Security Auditing via oxidized-skills
//!
//! Integrates the `oxidized-skills` crate to scan skill directories for:
//! - Dangerous bash patterns (RCE, credential exfiltration, reverse shells)
//! - Prompt injection attempts
//! - Exposed secrets
//! - Unsafe package installations
//! - Frontmatter validation issues
//!
//! Can be used to audit skills at load time or on-demand.

use oxidized_skills::{audit, config::Config, finding::AuditReport, output};
use std::path::Path;
use tracing::{debug, warn};

/// Result of auditing a skill directory.
#[derive(Debug)]
pub struct SkillAuditResult {
    /// Name of the skill that was audited
    pub skill_name: String,
    /// Path to the skill directory
    pub skill_path: String,
    /// Whether the audit passed (no errors)
    pub passed: bool,
    /// Security score (0-100)
    pub score: u8,
    /// Letter grade (A-F)
    pub grade: char,
    /// Number of errors found
    pub error_count: usize,
    /// Number of warnings found
    pub warning_count: usize,
    /// Summary of findings (human-readable)
    pub summary: String,
    /// Full report for detailed inspection
    pub report: Option<AuditReport>,
}

impl SkillAuditResult {
    /// Check if the skill should be blocked from loading based on score
    pub fn should_block(&self, min_score: u8) -> bool {
        self.score < min_score || self.error_count > 0
    }

    /// Get a one-line status for logging
    pub fn status_line(&self) -> String {
        if self.passed {
            format!(
                "✓ {} — Score: {}/100 ({})",
                self.skill_name, self.score, self.grade
            )
        } else {
            format!(
                "✗ {} — Score: {}/100 ({}) — {} errors, {} warnings",
                self.skill_name, self.score, self.grade, self.error_count, self.warning_count
            )
        }
    }
}

/// Skill security auditor using oxidized-skills.
pub struct SkillAuditor {
    config: Config,
    /// Minimum score required to pass (0-100)
    pub min_score: u8,
    /// Whether to block loading of skills that fail audit
    pub block_on_failure: bool,
}

impl Default for SkillAuditor {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillAuditor {
    /// Create a new auditor with default configuration.
    pub fn new() -> Self {
        let config = Config::load(None).unwrap_or_default();
        Self {
            config,
            min_score: 70, // Default: require at least a C grade
            block_on_failure: false,
        }
    }

    /// Create an auditor with custom minimum score.
    pub fn with_min_score(mut self, score: u8) -> Self {
        self.min_score = score.min(100);
        self
    }

    /// Set whether to block loading of failing skills.
    pub fn with_blocking(mut self, block: bool) -> Self {
        self.block_on_failure = block;
        self
    }

    /// Load configuration from a custom path.
    pub fn with_config_path(mut self, path: &Path) -> Self {
        if let Ok(config) = Config::load(Some(path)) {
            self.config = config;
        }
        self
    }

    /// Audit a single skill directory.
    pub fn audit_skill(&self, skill_path: &Path, skill_name: &str) -> SkillAuditResult {
        debug!("Auditing skill: {} at {}", skill_name, skill_path.display());

        let report = audit::run_audit(skill_path, &self.config);

        let score = report.security_score;
        let grade = grade_to_char(&report.security_grade);
        let error_count = report
            .findings
            .iter()
            .filter(|f| f.severity == oxidized_skills::finding::Severity::Error)
            .count();
        let warning_count = report
            .findings
            .iter()
            .filter(|f| f.severity == oxidized_skills::finding::Severity::Warning)
            .count();

        let summary = if report.passed {
            format!("Audit passed with score {}/100", score)
        } else {
            let findings: Vec<String> = report
                .findings
                .iter()
                .take(5)
                .map(|f| format!("  - [{}] {}: {}", f.rule_id, f.scanner, f.message))
                .collect();
            format!(
                "Audit failed with {} errors, {} warnings:\n{}{}",
                error_count,
                warning_count,
                findings.join("\n"),
                if report.findings.len() > 5 {
                    format!("\n  ... and {} more", report.findings.len() - 5)
                } else {
                    String::new()
                }
            )
        };

        if !report.passed {
            warn!(
                "Skill {} failed security audit: {} errors, {} warnings (score: {})",
                skill_name, error_count, warning_count, score
            );
        }

        SkillAuditResult {
            skill_name: skill_name.to_string(),
            skill_path: skill_path.display().to_string(),
            passed: report.passed,
            score,
            grade,
            error_count,
            warning_count,
            summary,
            report: Some(report),
        }
    }

    /// Audit multiple skill directories.
    pub fn audit_skills(&self, skills: &[(impl AsRef<Path>, &str)]) -> Vec<SkillAuditResult> {
        skills
            .iter()
            .map(|(path, name)| self.audit_skill(path.as_ref(), name))
            .collect()
    }

    /// Get a formatted report for a skill audit.
    pub fn format_report(&self, result: &SkillAuditResult) -> String {
        if let Some(ref report) = result.report {
            output::format_report(report, &output::OutputFormat::Pretty)
        } else {
            result.summary.clone()
        }
    }

    /// Get JSON output for a skill audit.
    pub fn format_report_json(&self, result: &SkillAuditResult) -> String {
        if let Some(ref report) = result.report {
            output::format_report(report, &output::OutputFormat::Json)
        } else {
            serde_json::json!({
                "skill": result.skill_name,
                "passed": result.passed,
                "score": result.score,
                "grade": result.grade.to_string(),
            })
            .to_string()
        }
    }
}

/// Convert SecurityGrade enum to a char.
fn grade_to_char(grade: &oxidized_skills::finding::SecurityGrade) -> char {
    use oxidized_skills::finding::SecurityGrade;
    match grade {
        SecurityGrade::A => 'A',
        SecurityGrade::B => 'B',
        SecurityGrade::C => 'C',
        SecurityGrade::D => 'D',
        SecurityGrade::F => 'F',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auditor_default() {
        let auditor = SkillAuditor::new();
        assert_eq!(auditor.min_score, 70);
        assert!(!auditor.block_on_failure);
    }

    #[test]
    fn test_should_block() {
        let result = SkillAuditResult {
            skill_name: "test".to_string(),
            skill_path: "/tmp/test".to_string(),
            passed: false,
            score: 50,
            grade: 'F',
            error_count: 2,
            warning_count: 1,
            summary: "Failed".to_string(),
            report: None,
        };

        assert!(result.should_block(70));
        assert!(result.should_block(51));
        assert!(!result.should_block(50)); // Still blocked due to error_count > 0
    }
}
