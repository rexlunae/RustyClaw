//! Freshness Protocol — Data Volatility Awareness
//!
//! LLMs confidently produce outdated information: model IDs, pricing, deprecated APIs,
//! old package versions. This protocol categorizes data by volatility and indicates
//! when verification is needed.
//!
//! # Volatility Tiers
//!
//! - **Critical**: Changes in days/weeks (model IDs, pricing, CVEs) — MUST verify
//! - **High**: Changes in weeks/months (package versions, framework APIs) — verify when writing config
//! - **Medium**: Changes in months/quarters (browser APIs, compliance) — verify if uncertain
//! - **Stable**: Changes over years (language syntax, protocols) — trust training data

use serde::{Deserialize, Serialize};

/// Volatility tier indicating how frequently data changes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VolatilityTier {
    /// Changes in days to weeks — MUST verify before using
    /// Examples: LLM model IDs, API pricing, CVEs, SDK breaking changes
    Critical,

    /// Changes in weeks to months — verify when writing config/deps
    /// Examples: package versions, framework APIs, Docker tags, cloud services
    High,

    /// Changes in months to quarters — verify if uncertain
    /// Examples: browser APIs, crypto recommendations, compliance frameworks
    Medium,

    /// Changes over years — trust training data
    /// Examples: language syntax, protocols, algorithms, design patterns
    Stable,
}

impl VolatilityTier {
    /// Whether this tier requires verification before use
    pub fn requires_verification(&self) -> bool {
        matches!(self, VolatilityTier::Critical | VolatilityTier::High)
    }

    /// Human-readable recommendation for this tier
    pub fn recommendation(&self) -> &'static str {
        match self {
            VolatilityTier::Critical => "MUST verify via web search before implementing",
            VolatilityTier::High => "Verify when writing config, versions, or integration code",
            VolatilityTier::Medium => "Verify if uncertain or version-specific",
            VolatilityTier::Stable => "Trust training data — rarely changes",
        }
    }
}

/// Categories of data with their volatility tiers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataCategory {
    // Critical tier
    LlmModelIds,
    ApiPricing,
    SecurityAdvisories,
    SdkBreakingChanges,
    DeprecatedFeatures,

    // High tier
    PackageVersions,
    FrameworkApis,
    DockerBaseTags,
    CloudServices,
    TerraformSchemas,
    CiCdSyntax,
    OAuthFlows,
    CliFlags,

    // Medium tier
    BrowserApis,
    CryptoAlgorithms,
    ComplianceFrameworks,
    InfraBestPractices,

    // Stable tier
    LanguageFundamentals,
    Protocols,
    SqlFundamentals,
    Algorithms,
    DesignPatterns,
    GitOperations,
}

impl DataCategory {
    /// Get the volatility tier for this category
    pub fn tier(&self) -> VolatilityTier {
        match self {
            // Critical
            DataCategory::LlmModelIds
            | DataCategory::ApiPricing
            | DataCategory::SecurityAdvisories
            | DataCategory::SdkBreakingChanges
            | DataCategory::DeprecatedFeatures => VolatilityTier::Critical,

            // High
            DataCategory::PackageVersions
            | DataCategory::FrameworkApis
            | DataCategory::DockerBaseTags
            | DataCategory::CloudServices
            | DataCategory::TerraformSchemas
            | DataCategory::CiCdSyntax
            | DataCategory::OAuthFlows
            | DataCategory::CliFlags => VolatilityTier::High,

            // Medium
            DataCategory::BrowserApis
            | DataCategory::CryptoAlgorithms
            | DataCategory::ComplianceFrameworks
            | DataCategory::InfraBestPractices => VolatilityTier::Medium,

            // Stable
            DataCategory::LanguageFundamentals
            | DataCategory::Protocols
            | DataCategory::SqlFundamentals
            | DataCategory::Algorithms
            | DataCategory::DesignPatterns
            | DataCategory::GitOperations => VolatilityTier::Stable,
        }
    }

    /// Description of what this category covers
    pub fn description(&self) -> &'static str {
        match self {
            DataCategory::LlmModelIds => "LLM/AI model names, context windows, capabilities",
            DataCategory::ApiPricing => "Cloud service, SaaS, or API pricing",
            DataCategory::SecurityAdvisories => "Active CVEs, vulnerability disclosures",
            DataCategory::SdkBreakingChanges => "Major SDK/library version breaking changes",
            DataCategory::DeprecatedFeatures => "APIs, services, or flags marked for removal",
            DataCategory::PackageVersions => "Latest stable versions of libraries/frameworks",
            DataCategory::FrameworkApis => "Next.js, React, FastAPI, etc. API surfaces",
            DataCategory::DockerBaseTags => "Current LTS/stable Docker image tags",
            DataCategory::CloudServices => "AWS/GCP/Azure service names and features",
            DataCategory::TerraformSchemas => "Terraform/Pulumi resource schemas",
            DataCategory::CiCdSyntax => "GitHub Actions, GitLab CI workflow syntax",
            DataCategory::OAuthFlows => "Provider-specific OAuth endpoints and scopes",
            DataCategory::CliFlags => "CLI tool flags and subcommands",
            DataCategory::BrowserApis => "Web APIs, CSS features, browser support",
            DataCategory::CryptoAlgorithms => "Current best practices for hashing/encryption",
            DataCategory::ComplianceFrameworks => "SOC2, HIPAA, GDPR requirements",
            DataCategory::InfraBestPractices => "Instance types, scaling patterns",
            DataCategory::LanguageFundamentals => "Language syntax, type systems, stdlib",
            DataCategory::Protocols => "HTTP, TCP/IP, WebSocket, gRPC",
            DataCategory::SqlFundamentals => "SQL and database fundamentals",
            DataCategory::Algorithms => "Algorithms and data structures",
            DataCategory::DesignPatterns => "Design patterns and architecture principles",
            DataCategory::GitOperations => "Git operations and workflows",
        }
    }
}

/// Freshness protocol for tracking data volatility
pub struct FreshnessProtocol;

impl FreshnessProtocol {
    /// Check if a piece of data needs verification
    pub fn needs_verification(category: DataCategory) -> bool {
        category.tier().requires_verification()
    }

    /// Get verification guidance for a category
    pub fn guidance(category: DataCategory) -> String {
        format!(
            "{}: {} — {}",
            category.description(),
            format!("{:?}", category.tier()).to_uppercase(),
            category.tier().recommendation()
        )
    }

    /// Classify text to detect potential volatile data
    /// Returns categories that might need verification
    pub fn classify_text(text: &str) -> Vec<DataCategory> {
        let mut categories = Vec::new();
        let lower = text.to_lowercase();

        // Model IDs
        if lower.contains("gpt-4")
            || lower.contains("claude")
            || lower.contains("gemini")
            || lower.contains("model") && (lower.contains("id") || lower.contains("name"))
        {
            categories.push(DataCategory::LlmModelIds);
        }

        // Pricing
        if lower.contains("price")
            || lower.contains("pricing")
            || lower.contains("cost")
            || lower.contains("$/")
            || lower.contains("per token")
        {
            categories.push(DataCategory::ApiPricing);
        }

        // Security
        if lower.contains("cve-") || lower.contains("vulnerability") || lower.contains("advisory") {
            categories.push(DataCategory::SecurityAdvisories);
        }

        // Package versions
        if lower.contains("version")
            || lower.contains("@")
            || lower.contains("^")
            || lower.contains("~")
            || lower.contains("latest")
        {
            categories.push(DataCategory::PackageVersions);
        }

        // Docker
        if lower.contains("docker")
            || lower.contains("from ")
            || lower.contains(":alpine")
            || lower.contains(":slim")
        {
            categories.push(DataCategory::DockerBaseTags);
        }

        categories
    }

    /// Format a verification citation
    pub fn format_citation(what: &str, source: &str, date: &str) -> String {
        format!("✓ Verified: {} ({}, {})", what, source, date)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_volatility_tiers() {
        assert!(VolatilityTier::Critical.requires_verification());
        assert!(VolatilityTier::High.requires_verification());
        assert!(!VolatilityTier::Medium.requires_verification());
        assert!(!VolatilityTier::Stable.requires_verification());
    }

    #[test]
    fn test_category_tiers() {
        assert_eq!(DataCategory::LlmModelIds.tier(), VolatilityTier::Critical);
        assert_eq!(DataCategory::PackageVersions.tier(), VolatilityTier::High);
        assert_eq!(DataCategory::BrowserApis.tier(), VolatilityTier::Medium);
        assert_eq!(DataCategory::Algorithms.tier(), VolatilityTier::Stable);
    }

    #[test]
    fn test_text_classification() {
        let categories = FreshnessProtocol::classify_text("Use gpt-4o model");
        assert!(categories.contains(&DataCategory::LlmModelIds));

        let categories = FreshnessProtocol::classify_text("FROM node:22-alpine");
        assert!(categories.contains(&DataCategory::DockerBaseTags));

        let categories = FreshnessProtocol::classify_text("CVE-2026-1234 vulnerability");
        assert!(categories.contains(&DataCategory::SecurityAdvisories));
    }
}
