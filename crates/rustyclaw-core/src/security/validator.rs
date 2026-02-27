//! Input validation for the safety layer (inspired by IronClaw)
//!
//! Validates input text and tool parameters for security issues:
//! - Length limits (prevent DoS via huge inputs)
//! - Forbidden patterns
//! - Excessive whitespace/repetition (padding attacks)
//! - Null bytes and encoding issues
//!
//! # Attribution
//!
//! Input validation patterns inspired by [IronClaw](https://github.com/nearai/ironclaw) (Apache-2.0).

use std::collections::HashSet;

/// Result of validating input.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Whether the input is valid.
    pub is_valid: bool,
    /// Validation errors if any.
    pub errors: Vec<ValidationError>,
    /// Warnings that don't block processing.
    pub warnings: Vec<String>,
}

impl ValidationResult {
    /// Create a successful validation result.
    pub fn ok() -> Self {
        Self {
            is_valid: true,
            errors: vec![],
            warnings: vec![],
        }
    }

    /// Create a validation result with an error.
    pub fn error(error: ValidationError) -> Self {
        Self {
            is_valid: false,
            errors: vec![error],
            warnings: vec![],
        }
    }

    /// Add a warning to the result.
    pub fn with_warning(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }

    /// Merge another validation result into this one.
    pub fn merge(mut self, other: Self) -> Self {
        self.is_valid = self.is_valid && other.is_valid;
        self.errors.extend(other.errors);
        self.warnings.extend(other.warnings);
        self
    }
}

impl Default for ValidationResult {
    fn default() -> Self {
        Self::ok()
    }
}

/// A validation error.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// Field or aspect that failed validation.
    pub field: String,
    /// Error message.
    pub message: String,
    /// Error code for programmatic handling.
    pub code: ValidationErrorCode,
}

/// Error codes for validation errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValidationErrorCode {
    Empty,
    TooLong,
    TooShort,
    InvalidFormat,
    ForbiddenContent,
    InvalidEncoding,
    SuspiciousPattern,
}

/// Input validator with configurable rules.
pub struct InputValidator {
    /// Maximum input length.
    max_length: usize,
    /// Minimum input length.
    min_length: usize,
    /// Forbidden substrings (case-insensitive).
    forbidden_patterns: HashSet<String>,
}

impl InputValidator {
    /// Create a new validator with default settings.
    pub fn new() -> Self {
        Self {
            max_length: 100_000,
            min_length: 1,
            forbidden_patterns: HashSet::new(),
        }
    }

    /// Set maximum input length.
    pub fn with_max_length(mut self, max: usize) -> Self {
        self.max_length = max;
        self
    }

    /// Set minimum input length.
    pub fn with_min_length(mut self, min: usize) -> Self {
        self.min_length = min;
        self
    }

    /// Add a forbidden pattern (case-insensitive).
    pub fn forbid_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.forbidden_patterns
            .insert(pattern.into().to_lowercase());
        self
    }

    /// Validate input text.
    pub fn validate(&self, input: &str) -> ValidationResult {
        let mut result = ValidationResult::ok();

        // Check empty
        if input.is_empty() {
            return ValidationResult::error(ValidationError {
                field: "input".to_string(),
                message: "Input cannot be empty".to_string(),
                code: ValidationErrorCode::Empty,
            });
        }

        // Check length
        if input.len() > self.max_length {
            result = result.merge(ValidationResult::error(ValidationError {
                field: "input".to_string(),
                message: format!(
                    "Input too long: {} bytes (max {})",
                    input.len(),
                    self.max_length
                ),
                code: ValidationErrorCode::TooLong,
            }));
        }

        if input.len() < self.min_length {
            result = result.merge(ValidationResult::error(ValidationError {
                field: "input".to_string(),
                message: format!(
                    "Input too short: {} bytes (min {})",
                    input.len(),
                    self.min_length
                ),
                code: ValidationErrorCode::TooShort,
            }));
        }

        // Check for null bytes (invalid in most contexts)
        if input.chars().any(|c| c == '\x00') {
            result = result.merge(ValidationResult::error(ValidationError {
                field: "input".to_string(),
                message: "Input contains null bytes".to_string(),
                code: ValidationErrorCode::InvalidEncoding,
            }));
        }

        // Check forbidden patterns
        let lower_input = input.to_lowercase();
        for pattern in &self.forbidden_patterns {
            if lower_input.contains(pattern) {
                result = result.merge(ValidationResult::error(ValidationError {
                    field: "input".to_string(),
                    message: format!("Input contains forbidden pattern: {}", pattern),
                    code: ValidationErrorCode::ForbiddenContent,
                }));
            }
        }

        // Check for excessive whitespace (might indicate padding attacks)
        let whitespace_ratio =
            input.chars().filter(|c| c.is_whitespace()).count() as f64 / input.len() as f64;
        if whitespace_ratio > 0.9 && input.len() > 100 {
            result = result.with_warning("Input has unusually high whitespace ratio");
        }

        // Check for repeated characters (might indicate padding)
        if has_excessive_repetition(input) {
            result = result.with_warning("Input has excessive character repetition");
        }

        result
    }

    /// Validate tool parameters (recursively checks all string values in JSON).
    pub fn validate_tool_params(&self, params: &serde_json::Value) -> ValidationResult {
        let mut result = ValidationResult::ok();

        fn check_strings(
            value: &serde_json::Value,
            validator: &InputValidator,
            result: &mut ValidationResult,
        ) {
            match value {
                serde_json::Value::String(s) => {
                    let string_result = validator.validate(s);
                    *result = std::mem::take(result).merge(string_result);
                }
                serde_json::Value::Array(arr) => {
                    for item in arr {
                        check_strings(item, validator, result);
                    }
                }
                serde_json::Value::Object(obj) => {
                    for (_, v) in obj {
                        check_strings(v, validator, result);
                    }
                }
                _ => {}
            }
        }

        check_strings(params, self, &mut result);
        result
    }
}

impl Default for InputValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if string has excessive repetition of characters.
fn has_excessive_repetition(s: &str) -> bool {
    if s.len() < 50 {
        return false;
    }

    let chars: Vec<char> = s.chars().collect();
    let mut max_repeat = 1;
    let mut current_repeat = 1;

    for i in 1..chars.len() {
        if chars[i] == chars[i - 1] {
            current_repeat += 1;
            max_repeat = max_repeat.max(current_repeat);
        } else {
            current_repeat = 1;
        }
    }

    // More than 20 repeated characters is suspicious
    max_repeat > 20
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_input() {
        let validator = InputValidator::new();
        let result = validator.validate("Hello, this is a normal message.");
        assert!(result.is_valid);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_empty_input() {
        let validator = InputValidator::new();
        let result = validator.validate("");
        assert!(!result.is_valid);
        assert!(result.errors.iter().any(|e| e.code == ValidationErrorCode::Empty));
    }

    #[test]
    fn test_too_long_input() {
        let validator = InputValidator::new().with_max_length(10);
        let result = validator.validate("This is way too long for the limit");
        assert!(!result.is_valid);
        assert!(result.errors.iter().any(|e| e.code == ValidationErrorCode::TooLong));
    }

    #[test]
    fn test_forbidden_pattern() {
        let validator = InputValidator::new().forbid_pattern("forbidden");
        let result = validator.validate("This contains FORBIDDEN content");
        assert!(!result.is_valid);
        assert!(result.errors.iter().any(|e| e.code == ValidationErrorCode::ForbiddenContent));
    }

    #[test]
    fn test_excessive_repetition_warning() {
        let validator = InputValidator::new();
        // String needs to be >= 50 chars for repetition check
        let result = validator.validate(&format!(
            "Start of message{}End of message",
            "a".repeat(30)
        ));
        assert!(result.is_valid); // Still valid, just a warning
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn test_null_bytes_rejected() {
        let validator = InputValidator::new();
        let result = validator.validate("Hello\x00World");
        assert!(!result.is_valid);
        assert!(result.errors.iter().any(|e| e.code == ValidationErrorCode::InvalidEncoding));
    }

    #[test]
    fn test_validate_tool_params() {
        let validator = InputValidator::new().forbid_pattern("secret_word");
        let params = serde_json::json!({
            "name": "test",
            "nested": {
                "value": "contains secret_word here"
            }
        });
        let result = validator.validate_tool_params(&params);
        assert!(!result.is_valid);
    }

    #[test]
    fn test_high_whitespace_warning() {
        let validator = InputValidator::new();
        // Create a string that's mostly whitespace
        let whitespace_heavy = format!("a{}", " ".repeat(150));
        let result = validator.validate(&whitespace_heavy);
        assert!(result.is_valid); // Valid, but has warning
        assert!(result.warnings.iter().any(|w| w.contains("whitespace")));
    }
}
