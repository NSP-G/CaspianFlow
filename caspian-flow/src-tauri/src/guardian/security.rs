//! Security checker (L4) — detects dangerous content in Skill output.
//!
//! This module implements rule-based security checks using regex patterns.
//! It detects:
//! - API keys and tokens (OpenAI, AWS, GitHub, etc.)
//! - Private keys (RSA, EC)
//! - Passwords in output
//! - Malicious code patterns (rm -rf, mkfs, fork bombs, etc.)
//! - Output size limits (truncation + warning)
//!
//! Design note: We detect *values* not *key names* — a JSON field named
//! "password" with an empty value does not trigger, but "password: secret123"
//! in free text does. This reduces false positives from code examples.

use regex::Regex;
use serde::{Deserialize, Serialize};

/// A security violation found in Skill output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecurityViolation {
    /// The category of violation (e.g. "api_key", "private_key", "malicious_code").
    pub category: String,
    /// Human-readable description.
    pub message: String,
    /// The matched pattern (redacted for safety).
    pub matched_pattern: String,
}

impl std::fmt::Display for SecurityViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.category, self.message)
    }
}

/// A compiled security rule.
struct SecurityRule {
    category: &'static str,
    pattern: Regex,
    message: &'static str,
}

/// The security checker — holds compiled regex rules.
///
/// Rules are compiled once at construction time for performance.
/// All checks are pure in-memory operations (no network calls).
pub struct SecurityChecker {
    rules: Vec<SecurityRule>,
    max_output_size: usize,
    enable_truncation: bool,
}

impl SecurityChecker {
    /// Create a new security checker with default rules and settings.
    pub fn new() -> Self {
        Self {
            rules: default_rules(),
            max_output_size: DEFAULT_MAX_OUTPUT_SIZE,
            enable_truncation: true,
        }
    }

    /// Create a checker with custom output size limit.
    pub fn with_max_output_size(mut self, size: usize) -> Self {
        self.max_output_size = size;
        self
    }

    /// Enable or disable output truncation.
    pub fn with_truncation(mut self, enable: bool) -> Self {
        self.enable_truncation = enable;
        self
    }

    /// Check raw output for security violations.
    ///
    /// Returns a list of violations found. Empty list = clean.
    pub fn check(&self, output: &str) -> Vec<SecurityViolation> {
        let mut violations = Vec::new();

        for rule in &self.rules {
            if let Some(m) = rule.pattern.find(output) {
                // Redact the matched text — don't include the actual secret
                let redacted = redact_match(m.as_str());
                violations.push(SecurityViolation {
                    category: rule.category.to_string(),
                    message: rule.message.to_string(),
                    matched_pattern: redacted,
                });
            }
        }

        violations
    }

    /// Check output size and truncate if necessary.
    ///
    /// Returns `(possibly_truncated_output, warnings)`.
    /// If the output is within limits, returns it unchanged with no warnings.
    /// If truncation is disabled and output exceeds limit, returns a warning
    /// but does not truncate.
    pub fn check_size(&self, output: &str) -> (String, Vec<String>) {
        let byte_len = output.len();

        if byte_len <= self.max_output_size {
            return (output.to_string(), Vec::new());
        }

        let mut warnings = Vec::new();

        if self.enable_truncation {
            // Truncate at byte boundary (careful with UTF-8)
            let truncated = truncate_at_char_boundary(output, self.max_output_size);
            let truncation_notice = format!(
                "\n\n[OUTPUT TRUNCATED: original size {byte_len} bytes, limit {} bytes]",
                self.max_output_size
            );
            warnings.push(format!(
                "output truncated from {byte_len} to {} bytes",
                truncated.len() + truncation_notice.len()
            ));
            (format!("{truncated}{truncation_notice}"), warnings)
        } else {
            warnings.push(format!(
                "output size {byte_len} exceeds limit {} bytes (truncation disabled)",
                self.max_output_size
            ));
            (output.to_string(), warnings)
        }
    }

    /// Check if a skill has the `security-exempt` tag (reduces strictness).
    pub fn is_exempted(tags: &[String]) -> bool {
        tags.iter().any(|t| t == "security-exempt")
    }
}

impl Default for SecurityChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SecurityChecker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecurityChecker")
            .field("rules_count", &self.rules.len())
            .field("max_output_size", &self.max_output_size)
            .field("enable_truncation", &self.enable_truncation)
            .finish()
    }
}

/// Default maximum output size: 1 MB.
pub const DEFAULT_MAX_OUTPUT_SIZE: usize = 1024 * 1024;

/// Redact a matched secret — show first 4 chars + asterisks.
fn redact_match(matched: &str) -> String {
    if matched.len() <= 8 {
        "*".repeat(matched.len())
    } else {
        let prefix = &matched[..4];
        format!("{prefix}{}", "*".repeat(matched.len().saturating_sub(4)))
    }
}

/// Truncate a string at a character boundary near the target byte length.
fn truncate_at_char_boundary(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }

    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Compile the default set of security rules.
///
/// Each rule is a regex pattern that matches a specific type of sensitive
/// or dangerous content.
fn default_rules() -> Vec<SecurityRule> {
    vec![
        // --- API Keys ---
        SecurityRule {
            category: "api_key",
            pattern: Regex::new(r"(?i)sk-[a-zA-Z0-9]{20,}").unwrap(),
            message: "OpenAI API key detected in output",
        },
        SecurityRule {
            category: "api_key",
            pattern: Regex::new(r"AKIA[A-Z0-9]{16}").unwrap(),
            message: "AWS access key ID detected in output",
        },
        SecurityRule {
            category: "api_key",
            pattern: Regex::new(r"ghp_[a-zA-Z0-9]{36}").unwrap(),
            message: "GitHub personal access token detected in output",
        },
        SecurityRule {
            category: "api_key",
            pattern: Regex::new(r"gho_[a-zA-Z0-9]{36}").unwrap(),
            message: "GitHub OAuth token detected in output",
        },
        SecurityRule {
            category: "api_key",
            pattern: Regex::new(r"xox[baprs]-[a-zA-Z0-9-]{10,}").unwrap(),
            message: "Slack token detected in output",
        },
        // --- Bearer tokens ---
        SecurityRule {
            category: "token",
            pattern: Regex::new(r"(?i)Bearer\s+[a-zA-Z0-9\-._~+/]+=*").unwrap(),
            message: "Bearer authentication token detected in output",
        },
        // --- Private keys ---
        SecurityRule {
            category: "private_key",
            pattern: Regex::new(r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY-----")
                .unwrap(),
            message: "Private key block detected in output",
        },
        // --- Passwords in free text (not JSON key names) ---
        SecurityRule {
            category: "password",
            pattern: Regex::new(r"(?i)(?:password|passwd|pwd)\s*[=:]\s*\S+").unwrap(),
            message: "Password value detected in output",
        },
        // --- Malicious commands ---
        SecurityRule {
            category: "malicious_code",
            pattern: Regex::new(r"(?i)rm\s+-rf\s+/").unwrap(),
            message: "Destructive command 'rm -rf /' detected in output",
        },
        SecurityRule {
            category: "malicious_code",
            pattern: Regex::new(r"(?i)mkfs\.\w+\s+/dev/").unwrap(),
            message: "Filesystem format command detected in output",
        },
        SecurityRule {
            category: "malicious_code",
            pattern: Regex::new(r"(?i)dd\s+if=/dev/zero\s+of=/dev/sd").unwrap(),
            message: "Disk overwrite command detected in output",
        },
        SecurityRule {
            category: "malicious_code",
            pattern: Regex::new(r":\(\)\{\s*:\|:&\s*\};:").unwrap(),
            message: "Fork bomb pattern detected in output",
        },
        SecurityRule {
            category: "malicious_code",
            pattern: Regex::new(r"(?i)chmod\s+777\s+/").unwrap(),
            message: "Dangerous chmod 777 on root path detected in output",
        },
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_output() {
        let checker = SecurityChecker::new();
        let violations = checker.check(r#"{"content": "hello world"}"#);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_detect_openai_key() {
        let checker = SecurityChecker::new();
        let output = r#"{"result": "sk-abcdefghijklmnopqrstuvwxyz1234567890"}"#;
        let violations = checker.check(output);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].category, "api_key");
        assert!(violations[0].message.contains("OpenAI"));
    }

    #[test]
    fn test_detect_aws_key() {
        let checker = SecurityChecker::new();
        let output = "AKIAIOSFODNN7EXAMPLE";
        let violations = checker.check(output);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].category, "api_key");
        assert!(violations[0].message.contains("AWS"));
    }

    #[test]
    fn test_detect_github_token() {
        let checker = SecurityChecker::new();
        let output = "ghp_0123456789abcdefghijklmnopqrstuvwxyz";
        let violations = checker.check(output);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].category, "api_key");
        assert!(violations[0].message.contains("GitHub"));
    }

    #[test]
    fn test_detect_bearer_token() {
        let checker = SecurityChecker::new();
        let output = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        let violations = checker.check(output);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].category, "token");
    }

    #[test]
    fn test_detect_private_key() {
        let checker = SecurityChecker::new();
        let output = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...";
        let violations = checker.check(output);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].category, "private_key");
    }

    #[test]
    fn test_detect_password_value() {
        let checker = SecurityChecker::new();
        let output = "The password=secret123 was found";
        let violations = checker.check(output);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].category, "password");
    }

    #[test]
    fn test_password_key_name_no_false_positive() {
        // A JSON field named "password" with empty value should not trigger
        let checker = SecurityChecker::new();
        let output = r#"{"password": ""}"#;
        let violations = checker.check(output);
        // "password": "" — the regex requires \S+ after =, so empty won't match
        // But "password": "" has a colon — let's verify behavior
        // The pattern is (?i)(?:password|passwd|pwd)\s*[=:]\s*\S+
        // "password": "" → after the colon there's "" which is not \S+ (space then empty)
        assert!(
            violations.is_empty() || violations.iter().all(|v| v.category != "password"),
            "empty password value should not trigger"
        );
    }

    #[test]
    fn test_detect_rm_rf() {
        let checker = SecurityChecker::new();
        let violations = checker.check("run rm -rf / to clean up");
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].category, "malicious_code");
    }

    #[test]
    fn test_detect_fork_bomb() {
        let checker = SecurityChecker::new();
        let violations = checker.check(":(){ :|:& };:");
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].category, "malicious_code");
    }

    #[test]
    fn test_detect_mkfs() {
        let checker = SecurityChecker::new();
        let violations = checker.check("mkfs.ext4 /dev/sda1");
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].category, "malicious_code");
    }

    #[test]
    fn test_detect_chmod_777_root() {
        let checker = SecurityChecker::new();
        let violations = checker.check("chmod 777 /etc/passwd");
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].category, "malicious_code");
    }

    #[test]
    fn test_multiple_violations() {
        let checker = SecurityChecker::new();
        let output = "sk-abcdefghijklmnopqrstuvwxyz1234567890 and rm -rf /";
        let violations = checker.check(output);
        assert_eq!(violations.len(), 2);
    }

    #[test]
    fn test_size_within_limit() {
        let checker = SecurityChecker::new();
        let output = "small output";
        let (result, warnings) = checker.check_size(output);
        assert_eq!(result, output);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_size_truncation() {
        let checker = SecurityChecker::new().with_max_output_size(10);
        let output = "this is a very long output that exceeds the limit";
        let (result, warnings) = checker.check_size(output);
        assert!(result.contains("TRUNCATED"));
        assert!(!warnings.is_empty());
        assert!(result.len() < output.len() + 100); // truncation notice adds some bytes
    }

    #[test]
    fn test_size_no_truncation() {
        let checker = SecurityChecker::new()
            .with_max_output_size(10)
            .with_truncation(false);
        let output = "this is a very long output";
        let (result, warnings) = checker.check_size(output);
        assert_eq!(result, output); // not truncated
        assert!(!warnings.is_empty());
    }

    #[test]
    fn test_size_truncation_utf8() {
        let checker = SecurityChecker::new().with_max_output_size(10);
        // Chinese characters are 3 bytes each in UTF-8
        let output = "你好世界你好世界你好世界";
        let (result, warnings) = checker.check_size(output);
        assert!(result.contains("TRUNCATED"));
        assert!(!warnings.is_empty());
        // Should not have broken UTF-8
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
    }

    #[test]
    fn test_security_exempt_tag() {
        assert!(SecurityChecker::is_exempted(&[
            "security-exempt".to_string()
        ]));
        assert!(!SecurityChecker::is_exempted(&["file".to_string()]));
        assert!(!SecurityChecker::is_exempted(&[]));
    }

    #[test]
    fn test_redact_match() {
        let redacted = redact_match("sk-abcdefghijklmnopqrstuvwxyz1234567890");
        assert!(redacted.starts_with("sk-a"));
        assert!(redacted.contains("*"));
        assert!(!redacted.contains("bcdefgh"));
    }

    #[test]
    fn test_redact_short_match() {
        let redacted = redact_match("short");
        assert_eq!(redacted, "*****");
    }

    #[test]
    fn test_debug_format() {
        let checker = SecurityChecker::new();
        let debug = format!("{checker:?}");
        assert!(debug.contains("SecurityChecker"));
        assert!(debug.contains("rules_count"));
    }
}
