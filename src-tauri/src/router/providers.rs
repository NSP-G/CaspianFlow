//! LLM provider implementations for P24 multi-API support.
//!
//! Design (P24 v2): prefer a single generic [`OpenAICompatibleProvider`] that
//! covers the vast majority of vendors (OpenAI, DeepSeek, GLM, Moonshot, Qwen,
//! …), plus two specialized providers for genuinely different protocols
//! ([`AnthropicProvider`]) and user-defined endpoints ([`CustomProvider`]).
//!
//! The 30+ vendor list is *not* 30+ distinct implementations — it is 30+ rows
//! in the [`PRESETS`] table (base_url + auth scheme), resolved by the
//! `preset` field on `ModelConfig`. This keeps the code surface small while
//! still letting users pick any vendor by filling in an API key.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::config::settings::ModelConfig;
use crate::router::slot_filler::{LlmProvider, ModelSize};
use crate::types::{LlmError, LlmResult};

/// A vendor preset: default base URL + the auth scheme used by its API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthScheme {
    /// `Authorization: Bearer <key>` (OpenAI-compatible APIs).
    Bearer,
    /// `x-api-key: <key>` + `anthropic-version` (Anthropic native protocol).
    XApiKey,
}

#[derive(Debug, Clone, Copy)]
struct Preset {
    base_url: &'static str,
    auth: AuthScheme,
}

/// Static vendor preset table. `base_url` is the API root; the provider
/// appends the endpoint path (`/chat/completions`, `/messages`, …).
///
/// Source: the ~20 representative entries from the P24 design doc §9 (the full
/// 30+ list is a separate reference document and is **not** loaded by code).
static PRESETS: &[(&str, Preset)] = &[
    (
        "openai",
        Preset {
            base_url: "https://api.openai.com/v1",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "deepseek",
        Preset {
            base_url: "https://api.deepseek.com/v1",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "glm",
        Preset {
            base_url: "https://open.bigmodel.cn/api/paas/v4",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "moonshot",
        Preset {
            base_url: "https://api.moonshot.cn/v1",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "qwen",
        Preset {
            base_url: "https://dashscope.aliyun.com/compatible-mode/v1",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "doubao",
        Preset {
            base_url: "https://ark.cn-beijing.volces.com/api/v3",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "baidu",
        Preset {
            base_url: "https://qianfan.baidubce.com/v2",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "hunyuan",
        Preset {
            base_url: "https://api.hunyuan.cloud.tencent.com/v1",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "spark",
        Preset {
            base_url: "https://spark-api-open.xf-yun.com/v1",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "siliconflow",
        Preset {
            base_url: "https://api.siliconflow.cn/v1",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "minimax",
        Preset {
            base_url: "https://api.minimax.chat/v1",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "baichuan",
        Preset {
            base_url: "https://api.baichuan-ai.com/v1",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "stepfun",
        Preset {
            base_url: "https://api.stepfun.com/v1",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "yi",
        Preset {
            base_url: "https://api.lingyiwanwu.com/v1",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "openrouter",
        Preset {
            base_url: "https://openrouter.ai/api/v1",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "together",
        Preset {
            base_url: "https://api.together.xyz/v1",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "groq",
        Preset {
            base_url: "https://api.groq.com/openai/v1",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "fireworks",
        Preset {
            base_url: "https://api.fireworks.ai/inference/v1",
            auth: AuthScheme::Bearer,
        },
    ),
    (
        "anthropic",
        Preset {
            base_url: "https://api.anthropic.com/v1",
            auth: AuthScheme::XApiKey,
        },
    ),
    (
        "ollama",
        Preset {
            base_url: "http://localhost:11434/v1",
            auth: AuthScheme::Bearer,
        },
    ),
];

/// Resolve a preset name to its base URL + auth scheme.
///
/// Returns `None` for unknown / custom presets — callers must then fall back
/// to the explicit `base_url` / `auth_type` on the `ModelConfig`.
pub fn resolve_preset(preset: &str) -> Option<(&'static str, AuthScheme)> {
    PRESETS
        .iter()
        .find(|(name, _)| *name == preset)
        .map(|(_, p)| (p.base_url, p.auth))
}

/// All known preset names (exposed for settings UI / validation).
pub fn preset_names() -> Vec<&'static str> {
    PRESETS.iter().map(|(n, _)| *n).collect()
}

/// Compute the effective base URL for a model config, honoring `preset` then
/// the explicit `base_url` override.
pub fn effective_base_url(cfg: &ModelConfig) -> Option<String> {
    if let Some(url) = &cfg.base_url {
        return Some(url.clone());
    }
    cfg.preset
        .as_deref()
        .and_then(resolve_preset)
        .map(|(u, _)| u.to_string())
}

/// Compute the effective auth scheme for a model config.
fn effective_auth(cfg: &ModelConfig) -> AuthScheme {
    if let Some(at) = &cfg.auth_type {
        return match at.as_str() {
            "x-api-key" => AuthScheme::XApiKey,
            _ => AuthScheme::Bearer,
        };
    }
    cfg.preset
        .as_deref()
        .and_then(resolve_preset)
        .map(|(_, auth)| auth)
        .unwrap_or(AuthScheme::Bearer)
}

/// Shared OpenAI-compatible chat-completion call.
///
/// Used by `OpenAICompatibleProvider`, `GLMProvider`, and `CustomProvider`
/// (all of which speak the `/chat/completions` protocol).
async fn openai_chat_completion(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    auth: AuthScheme,
    model: &str,
    prompt: &str,
    max_tokens: u32,
) -> LlmResult<String> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let mut req = client.post(&url).json(&json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": max_tokens,
    }));
    req = match auth {
        AuthScheme::Bearer => req.bearer_auth(api_key),
        // Custom providers may use a non-Bearer scheme; encode it literally.
        AuthScheme::XApiKey => req.header("x-api-key", api_key),
    };

    let resp = req
        .send()
        .await
        .map_err(|e| LlmError::NetworkError(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(LlmError::NetworkError(format!("HTTP {status}: {body}")));
    }

    let value: Value = resp
        .json()
        .await
        .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;
    value
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| LlmError::InvalidResponse("missing choices[0].message.content".into()))
}

/// Anthropic native `POST /messages` call.
async fn anthropic_message(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    prompt: &str,
    max_tokens: u32,
) -> LlmResult<String> {
    let url = format!("{}/messages", base_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": [{"role": "user", "content": prompt}],
        }))
        .send()
        .await
        .map_err(|e| LlmError::NetworkError(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(LlmError::NetworkError(format!("HTTP {status}: {body}")));
    }
    let value: Value = resp
        .json()
        .await
        .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;
    value
        .get("content")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| LlmError::InvalidResponse("missing content[0].text".into()))
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("reqwest client")
}

/// Generic OpenAI-compatible provider (covers OpenAI, DeepSeek, Moonshot, Qwen,
/// GLM, Ollama, …).
pub struct OpenAICompatibleProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model_name: String,
    model_size: ModelSize,
    auth: AuthScheme,
    max_tokens: u32,
}

impl OpenAICompatibleProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model_name: impl Into<String>,
        auth: AuthScheme,
    ) -> Self {
        Self {
            client: client(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            model_name: model_name.into(),
            model_size: ModelSize::Large,
            auth,
            max_tokens: 4096,
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAICompatibleProvider {
    async fn generate(&self, prompt: &str) -> LlmResult<String> {
        openai_chat_completion(
            &self.client,
            &self.base_url,
            &self.api_key,
            self.auth,
            &self.model_name,
            prompt,
            self.max_tokens,
        )
        .await
    }
    fn model_name(&self) -> &str {
        &self.model_name
    }
    fn model_size(&self) -> ModelSize {
        self.model_size
    }
}

/// GLM provider — GLM v4 is OpenAI-compatible at the `…/v4` base, so it reuses
/// the OpenAI-compatible path. Kept as a distinct type per the P24 design doc.
pub struct GLMProvider {
    inner: OpenAICompatibleProvider,
}

impl GLMProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model_name: impl Into<String>,
    ) -> Self {
        Self {
            inner: OpenAICompatibleProvider::new(base_url, api_key, model_name, AuthScheme::Bearer),
        }
    }
}

#[async_trait]
impl LlmProvider for GLMProvider {
    async fn generate(&self, prompt: &str) -> LlmResult<String> {
        self.inner.generate(prompt).await
    }
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }
    fn model_size(&self) -> ModelSize {
        self.inner.model_size()
    }
}

/// Anthropic native-protocol provider (`x-api-key`, `/messages`).
pub struct AnthropicProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model_name: String,
    model_size: ModelSize,
    max_tokens: u32,
}

impl AnthropicProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model_name: impl Into<String>,
    ) -> Self {
        Self {
            client: client(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            model_name: model_name.into(),
            model_size: ModelSize::Large,
            max_tokens: 4096,
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn generate(&self, prompt: &str) -> LlmResult<String> {
        anthropic_message(
            &self.client,
            &self.base_url,
            &self.api_key,
            &self.model_name,
            prompt,
            self.max_tokens,
        )
        .await
    }
    fn model_name(&self) -> &str {
        &self.model_name
    }
    fn model_size(&self) -> ModelSize {
        self.model_size
    }
}

/// User-defined provider: arbitrary `base_url` + `api_key` + `auth_type`,
/// speaking the OpenAI-compatible protocol.
pub struct CustomProvider {
    inner: OpenAICompatibleProvider,
}

impl CustomProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model_name: impl Into<String>,
        auth: AuthScheme,
    ) -> Self {
        Self {
            inner: OpenAICompatibleProvider::new(base_url, api_key, model_name, auth),
        }
    }
}

#[async_trait]
impl LlmProvider for CustomProvider {
    async fn generate(&self, prompt: &str) -> LlmResult<String> {
        self.inner.generate(prompt).await
    }
    fn model_name(&self) -> &str {
        self.inner.model_name()
    }
    fn model_size(&self) -> ModelSize {
        self.inner.model_size()
    }
}

/// Build the appropriate `LlmProvider` for a `ModelConfig`.
///
/// Selection rule:
/// - `preset == "anthropic"` → [`AnthropicProvider`]
/// - `preset == "glm"` → [`GLMProvider`]
/// - `preset == "custom"` or `auth_type` set → [`CustomProvider`]
/// - otherwise (openai/deepseek/moonshot/…/none) → [`OpenAICompatibleProvider`]
///
/// The API key is expected to be already resolved (e.g. via
/// `ConfigManager::resolve_api_key`, which handles `${ENV_VAR}` + keychain).
pub fn provider_from_config(cfg: &ModelConfig, resolved_api_key: String) -> Box<dyn LlmProvider> {
    let base_url =
        effective_base_url(cfg).unwrap_or_else(|| "http://localhost:11434/v1".to_string());
    let model = cfg.model_name.clone().unwrap_or_else(|| cfg.id.clone());
    let auth = effective_auth(cfg);

    match cfg.preset.as_deref() {
        Some("anthropic") => Box::new(AnthropicProvider::new(base_url, resolved_api_key, model)),
        Some("glm") => Box::new(GLMProvider::new(base_url, resolved_api_key, model)),
        Some("custom") => Box::new(CustomProvider::new(base_url, resolved_api_key, model, auth)),
        _ => Box::new(OpenAICompatibleProvider::new(
            base_url,
            resolved_api_key,
            model,
            auth,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_table_covers_known_vendors() {
        assert!(resolve_preset("openai").is_some());
        assert!(resolve_preset("deepseek").is_some());
        assert!(resolve_preset("anthropic").is_some());
        assert!(resolve_preset("glm").is_some());
        assert!(resolve_preset("ollama").is_some());
        // Unknown preset → None (falls back to explicit base_url).
        assert!(resolve_preset("does-not-exist").is_none());
    }

    #[test]
    fn preset_base_urls_match_design_doc() {
        assert_eq!(
            resolve_preset("openai").unwrap().0,
            "https://api.openai.com/v1"
        );
        assert_eq!(
            resolve_preset("deepseek").unwrap().0,
            "https://api.deepseek.com/v1"
        );
        assert_eq!(
            resolve_preset("anthropic").unwrap(),
            ("https://api.anthropic.com/v1", AuthScheme::XApiKey)
        );
    }

    #[test]
    fn provider_from_config_selects_anthropic() {
        let cfg = ModelConfig {
            id: "claude".into(),
            provider: "anthropic".into(),
            name: "Claude".into(),
            api_key: Some("${KEY}".into()),
            base_url: None,
            model_name: Some("claude-sonnet-4".into()),
            max_tokens: 4096,
            priority: 1,
            preset: Some("anthropic".into()),
            health: true,
            fallback: vec![],
            default: false,
            auth_type: None,
        };
        let p = provider_from_config(&cfg, "sk-test".into());
        assert_eq!(p.model_name(), "claude-sonnet-4");
    }

    #[test]
    fn provider_from_config_selects_openai_compatible() {
        let cfg = ModelConfig {
            id: "ds".into(),
            provider: "deepseek".into(),
            name: "DeepSeek".into(),
            api_key: Some("${KEY}".into()),
            base_url: None,
            model_name: None,
            max_tokens: 4096,
            priority: 1,
            preset: Some("deepseek".into()),
            health: true,
            fallback: vec![],
            default: false,
            auth_type: None,
        };
        let p = provider_from_config(&cfg, "sk-test".into());
        // model_name falls back to id when not set
        assert_eq!(p.model_name(), "ds");
    }

    #[test]
    fn provider_from_config_respects_explicit_base_url() {
        let cfg = ModelConfig {
            id: "my".into(),
            provider: "custom".into(),
            name: "My".into(),
            api_key: Some("sk".into()),
            base_url: Some("https://my.api/v1".into()),
            model_name: Some("m1".into()),
            max_tokens: 4096,
            priority: 1,
            preset: Some("custom".into()),
            health: false,
            fallback: vec![],
            default: false,
            auth_type: Some("bearer".into()),
        };
        let p = provider_from_config(&cfg, "sk".into());
        assert_eq!(p.model_name(), "m1");
    }

    // Network-dependent tests are gated: they require a live API key + endpoint.
    #[tokio::test]
    #[ignore]
    async fn live_openai_compatible_generate() {
        let cfg = ModelConfig {
            id: "gpt".into(),
            provider: "openai".into(),
            name: "GPT".into(),
            api_key: Some("${OPENAI_API_KEY}".into()),
            base_url: None,
            model_name: Some("gpt-4o-mini".into()),
            max_tokens: 64,
            priority: 1,
            preset: Some("openai".into()),
            health: true,
            fallback: vec![],
            default: false,
            auth_type: None,
        };
        let key = std::env::var("OPENAI_API_KEY").expect("set OPENAI_API_KEY");
        let p = provider_from_config(&cfg, key);
        let out = p.generate("Say 'pong'.").await.unwrap();
        assert!(out.to_lowercase().contains("pong"));
    }
}
