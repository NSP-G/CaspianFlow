//! API Key secure storage — environment variable resolution + OS keychain.

use std::env;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::types::{ConfigError, ConfigResult};

/// Pattern for `${VAR_NAME}` environment variable references.
static ENV_VAR_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\$\{([A-Z_][A-Z0-9_]*)\}$").unwrap());

/// Service name used in OS keychain entries.
const KEYRING_SERVICE: &str = "caspianflow";

/// How an API key is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    /// Resolved from an environment variable like `${OPENAI_API_KEY}`.
    EnvVar(String),
    /// Retrieved from the OS keychain (macOS Keychain / Windows Credential Manager / Linux Secret Service).
    Keychain(String),
    /// Stored as plaintext in the config file (insecure — local use only).
    Plaintext(String),
    /// No API key configured.
    None,
}

/// Resolve an API key value from the `api_key` field of a model config.
///
/// Resolution order:
/// 1. If the value matches `${VAR_NAME}`, resolve from environment variable.
/// 2. If the value is `keyring:model_id`, fetch from OS keychain.
/// 3. If the value is a non-empty string, treat as plaintext (with warning).
/// 4. If `None` or empty, return `None`.
pub fn resolve_api_key(api_key_field: Option<&str>, model_id: &str) -> ConfigResult<KeySource> {
    let raw = match api_key_field {
        Some(s) if !s.is_empty() => s,
        _ => return Ok(KeySource::None),
    };

    // 1. Environment variable reference: ${VAR_NAME}
    if let Some(caps) = ENV_VAR_RE.captures(raw) {
        let var_name = &caps[1];
        match env::var(var_name) {
            Ok(val) if !val.is_empty() => Ok(KeySource::EnvVar(val)),
            Ok(_) => Err(ConfigError::EnvVarMissing {
                var: format!("{var_name} (empty)"),
            }),
            Err(_) => {
                tracing::warn!(
                    var = var_name,
                    model = model_id,
                    "environment variable not set, falling back to keychain"
                );
                // Fall back to keychain
                fetch_from_keychain(model_id)
            }
        }
    } else if raw == "keyring:default" || raw.starts_with("keyring:") {
        // 2. Keychain reference
        fetch_from_keychain(model_id)
    } else {
        // 3. Plaintext
        tracing::warn!(
            model = model_id,
            "API key is stored as plaintext in config file — consider using ${{ENV_VAR}} or keyring"
        );
        Ok(KeySource::Plaintext(raw.to_string()))
    }
}

/// Fetch an API key from the OS keychain.
///
/// On headless Linux without D-Bus/Secret Service, this gracefully returns `None`.
pub fn fetch_from_keychain(model_id: &str) -> ConfigResult<KeySource> {
    match keyring::Entry::new(KEYRING_SERVICE, model_id) {
        Ok(entry) => match entry.get_password() {
            Ok(password) if !password.is_empty() => Ok(KeySource::Keychain(password)),
            Ok(_) => Ok(KeySource::None),
            Err(keyring::Error::NoEntry) => Ok(KeySource::None),
            Err(e) => {
                tracing::warn!(
                    model = model_id,
                    error = %e,
                    "failed to read from keychain, keychain may be unavailable"
                );
                Ok(KeySource::None)
            }
        },
        Err(e) => {
            tracing::warn!(
                model = model_id,
                error = %e,
                "keychain entry creation failed"
            );
            Ok(KeySource::None)
        }
    }
}

/// Store an API key in the OS keychain.
///
/// Returns `Ok(())` if stored successfully, or an error describing what went wrong.
pub fn store_in_keychain(model_id: &str, api_key: &str) -> ConfigResult<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, model_id)
        .map_err(|e| ConfigError::Parse(format!("keychain entry creation failed: {e}")))?;

    entry
        .set_password(api_key)
        .map_err(|e| ConfigError::Parse(format!("keychain store failed: {e}")))?;

    tracing::info!(model = model_id, "API key stored in OS keychain");
    Ok(())
}

/// Delete an API key from the OS keychain.
pub fn delete_from_keychain(model_id: &str) -> ConfigResult<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, model_id)
        .map_err(|e| ConfigError::Parse(format!("keychain entry creation failed: {e}")))?;

    match entry.delete_credential() {
        Ok(()) => {
            tracing::info!(model = model_id, "API key deleted from OS keychain");
            Ok(())
        }
        Err(keyring::Error::NoEntry) => Ok(()), // Already gone — fine
        Err(e) => Err(ConfigError::Parse(format!("keychain delete failed: {e}"))),
    }
}

/// Get the resolved API key as a plain string (or `None` if not available).
pub fn resolve_api_key_string(
    api_key_field: Option<&str>,
    model_id: &str,
) -> ConfigResult<Option<String>> {
    match resolve_api_key(api_key_field, model_id)? {
        KeySource::EnvVar(s) | KeySource::Keychain(s) | KeySource::Plaintext(s) => Ok(Some(s)),
        KeySource::None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_resolve_env_var() {
        env::set_var("TEST_CF_API_KEY", "test-secret-12345");
        let result = resolve_api_key(Some("${TEST_CF_API_KEY}"), "test-model").unwrap();
        assert_eq!(result, KeySource::EnvVar("test-secret-12345".to_string()));
        env::remove_var("TEST_CF_API_KEY");
    }

    #[test]
    fn test_resolve_env_var_missing_falls_back_to_keychain() {
        // ENV_VAR_DOES_NOT_EXIST should not be set
        let result = resolve_api_key(
            Some("${ENV_VAR_DOES_NOT_EXIST_12345}"),
            "test-model-fallback",
        )
        .unwrap();
        // Keychain will return None in headless environment
        assert_eq!(result, KeySource::None);
    }

    #[test]
    fn test_resolve_plaintext() {
        let result = resolve_api_key(Some("sk-abc123"), "test-model").unwrap();
        assert_eq!(result, KeySource::Plaintext("sk-abc123".to_string()));
    }

    #[test]
    fn test_resolve_none() {
        let result = resolve_api_key(None, "test-model").unwrap();
        assert_eq!(result, KeySource::None);
    }

    #[test]
    fn test_resolve_empty_string() {
        let result = resolve_api_key(Some(""), "test-model").unwrap();
        assert_eq!(result, KeySource::None);
    }

    #[test]
    fn test_resolve_env_var_string() {
        env::set_var("TEST_CF_KEY2", "secret-value");
        let result = resolve_api_key_string(Some("${TEST_CF_KEY2}"), "test-model").unwrap();
        assert_eq!(result, Some("secret-value".to_string()));
        env::remove_var("TEST_CF_KEY2");
    }

    #[test]
    fn test_env_var_regex() {
        assert!(ENV_VAR_RE.is_match("${OPENAI_API_KEY}"));
        assert!(ENV_VAR_RE.is_match("${DEEPSEEK_API_KEY}"));
        assert!(ENV_VAR_RE.is_match("${MY_KEY_123}"));
        assert!(!ENV_VAR_RE.is_match("${lowercase}"));
        assert!(!ENV_VAR_RE.is_match("plain-key"));
        assert!(!ENV_VAR_RE.is_match("${MISSING_CLOSE"));
        assert!(!ENV_VAR_RE.is_match("${}"));
    }

    #[test]
    fn test_resolve_keyring_prefix() {
        // keyring: prefix should try keychain (returns None in headless)
        let result = resolve_api_key(Some("keyring:default"), "test-keyring-model").unwrap();
        assert_eq!(result, KeySource::None);
    }
}
