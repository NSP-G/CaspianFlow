//! Model routing and fallback for P24.
//!
//! `ModelRouter` sits *on top of* the existing configuration system
//! (`ConfigManager` → `Settings::get_model` + `ConfigManager::resolve_api_key`)
//! and adds:
//!
//! - a **priority chain** — `Task-specified > Skill preference > Agent default >
//!   Global default`, then
//! - a **fallback chain** — each model's configured `fallback` list, and
//! - **health gating** — unhealthy (or key-less) models are skipped.
//!
//! It does *not* own model configuration or API-key storage; those remain the
//! responsibility of `ConfigManager` (B2 — reuse, don't reimplement).

use std::sync::Arc;

use thiserror::Error;

use crate::config::ConfigManager;
use crate::router::health::HealthChecker;
use crate::router::providers::provider_from_config;
use crate::router::slot_filler::LlmProvider;

/// How `resolve` should pick a model.
///
/// Only `Priority` is implemented in P24. `TurboAuto` and `LoadBalanced` are
/// reserved enum variants (per the design doc §8) — selecting them returns
/// [`ModelRouterError::UnsupportedStrategy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionStrategy {
    /// Resolve via the priority + fallback chain (P24 default).
    Priority,
    /// Reserved for the future TurboAuto routing engine.
    TurboAuto,
    /// Reserved for future load-aware distribution.
    LoadBalanced,
}

/// Errors raised while resolving a model.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModelRouterError {
    #[error("no available model after exhausting priority + fallback chain")]
    NoAvailableModel,
    #[error("selection strategy not supported in P24: {0:?}")]
    UnsupportedStrategy(SelectionStrategy),
}

/// The model-routing layer.
pub struct ModelRouter {
    config: Arc<ConfigManager>,
    health: Arc<dyn HealthChecker>,
}

impl ModelRouter {
    /// Build a router over the live configuration manager.
    pub fn new(config: Arc<ConfigManager>, health: Arc<dyn HealthChecker>) -> Self {
        Self { config, health }
    }

    /// Resolve a provider for the given preference chain.
    ///
    /// `model` is the Task-specified model (a bare model id *or* a
    /// `provider/model` string). `skill_preferred` / `agent_default` are
    /// caller-supplied model ids (from skill.yaml / AGENT.yaml). All three are
    /// optional; when absent the corresponding priority tier is skipped.
    pub fn resolve(
        &self,
        model: Option<&str>,
        skill_preferred: Option<&str>,
        agent_default: Option<&str>,
        strategy: SelectionStrategy,
    ) -> Result<Arc<dyn LlmProvider>, ModelRouterError> {
        match strategy {
            SelectionStrategy::Priority => {}
            other => return Err(ModelRouterError::UnsupportedStrategy(other)),
        }

        // Priority chain. `None` candidates are skipped.
        let primary_chain = [model, skill_preferred, agent_default];

        // 1. Walk the explicit priority chain first.
        for spec in primary_chain.into_iter().flatten() {
            if let Some(id) = self.lookup_id(spec) {
                if let Some(provider) = self.try_build(&id, "priority-chain") {
                    return Ok(provider);
                }
            }
        }

        // 2. Fall back to the global default model.
        if let Some(def) = self.config.settings().default_model() {
            if let Some(provider) = self.try_build(def.id.as_str(), "global-default") {
                return Ok(provider);
            }
        }

        // 3. Follow the fallback chain of any configured model reachable from
        //    the priority chain, then the global default's fallback.
        let mut visited = std::collections::HashSet::new();
        for spec in primary_chain.into_iter().flatten().chain(
            self.config
                .settings()
                .default_model()
                .map(|d| d.id.as_str()),
        ) {
            if let Some(id) = self.lookup_id(spec) {
                if let Some(provider) = self.follow_fallback(&id, &mut visited) {
                    return Ok(provider);
                }
            }
        }

        Err(ModelRouterError::NoAvailableModel)
    }

    /// Map a spec (bare id or `provider/model`) to a configured model id.
    fn lookup_id(&self, spec: &str) -> Option<String> {
        let settings = self.config.settings();
        if let Some(m) = settings.get_model(spec) {
            return Some(m.id.clone());
        }
        // Try `provider/model` → first model whose `provider` matches.
        if let Some((prov, _)) = spec.split_once('/') {
            return settings
                .models
                .iter()
                .find(|m| m.provider == prov)
                .map(|m| m.id.clone());
        }
        None
    }

    /// Attempt to build a provider, gated on health + resolvable key.
    /// Returns `None` if the model is unhealthy or its key cannot be resolved.
    fn try_build(&self, id: &str, reason: &str) -> Option<Arc<dyn LlmProvider>> {
        let settings = self.config.settings();
        let cfg = settings.get_model(id)?;

        if !self.health.is_healthy(&cfg.id) {
            tracing::warn!(model = %cfg.id, reason = "unhealthy", "skipping model in resolve");
            return None;
        }

        let key = match self.config.resolve_api_key(&cfg.id) {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(model = %cfg.id, error = %e, "failed to resolve api key");
                return None;
            }
        };

        let key = key.unwrap_or_default();
        let provider: Arc<dyn LlmProvider> = Arc::from(provider_from_config(cfg, key));
        tracing::info!(model = %cfg.id, selection = reason, "resolved model");
        Some(provider)
    }

    /// Walk a model's `fallback` list, returning the first healthy+keyed one.
    fn follow_fallback(
        &self,
        id: &str,
        visited: &mut std::collections::HashSet<String>,
    ) -> Option<Arc<dyn LlmProvider>> {
        let settings = self.config.settings();
        let start = settings.get_model(id)?;
        for fb in &start.fallback {
            if !visited.insert(fb.clone()) {
                continue; // cycle guard
            }
            if let Some(provider) = self.try_build(fb, "fallback") {
                return Some(provider);
            }
            // Recurse into the fallback's own fallback list.
            if let Some(p) = self.follow_fallback(fb, visited) {
                return Some(p);
            }
        }
        None
    }
}

/// Lightweight provider wrapper carrying the resolved model id + selection
/// reason, so callers (e.g. `ask`) can populate `QAResponse.meta`.
pub struct ResolvedModel {
    pub provider: Arc<dyn LlmProvider>,
    pub model_id: String,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings::{ModelConfig, Settings};
    use crate::config::CaspianPaths;
    use crate::router::health::InMemoryHealthChecker;

    fn model(id: &str, provider: &str, fallback: &[&str], default: bool) -> ModelConfig {
        ModelConfig {
            id: id.to_string(),
            provider: provider.to_string(),
            name: id.to_string(),
            api_key: Some(format!("${{{}_KEY}}", id.to_uppercase())),
            base_url: None,
            model_name: None,
            max_tokens: 4096,
            priority: 1,
            preset: Some(provider.to_string()),
            health: true,
            fallback: fallback.iter().map(|s| s.to_string()).collect(),
            default,
            auth_type: None,
        }
    }

    /// Build a ConfigManager over an in-memory settings file (no disk writes
    /// beyond the temp dir) for router tests.
    async fn test_config(models: Vec<ModelConfig>) -> Arc<ConfigManager> {
        let tmp = tempfile::tempdir().unwrap();
        let paths = CaspianPaths::resolve(Some(tmp.path()));
        // Initialize the file (writes the default config), then overwrite it
        // with our test models so ConfigManager loads exactly what we want.
        let _ = Settings::init(&paths);
        let mut settings = Settings::default();
        settings.models = models;
        settings.save(&paths.settings_file).unwrap();
        Arc::new(
            ConfigManager::init_with_paths(Some(tmp.path()))
                .await
                .unwrap(),
        )
    }

    async fn router(models: Vec<ModelConfig>, health: Arc<dyn HealthChecker>) -> ModelRouter {
        ModelRouter::new(test_config(models).await, health)
    }

    #[tokio::test]
    async fn priority_chain_task_beats_default() {
        let models = vec![
            model("deepseek", "deepseek", &[], true), // global default
            model("gpt", "openai", &[], false),
        ];
        let r = router(models, Arc::new(InMemoryHealthChecker::new())).await;
        let p = r
            .resolve(Some("gpt"), None, None, SelectionStrategy::Priority)
            .unwrap();
        assert_eq!(p.model_name(), "gpt");
    }

    #[tokio::test]
    async fn skill_preference_used_when_no_task_model() {
        let models = vec![
            model("deepseek", "deepseek", &[], true),
            model("gpt", "openai", &[], false),
        ];
        let r = router(models, Arc::new(InMemoryHealthChecker::new())).await;
        let p = r
            .resolve(None, Some("gpt"), None, SelectionStrategy::Priority)
            .unwrap();
        assert_eq!(p.model_name(), "gpt");
    }

    #[tokio::test]
    async fn agent_default_used_when_no_task_or_skill() {
        let models = vec![
            model("deepseek", "deepseek", &[], true),
            model("gpt", "openai", &[], false),
        ];
        let r = router(models, Arc::new(InMemoryHealthChecker::new())).await;
        let p = r
            .resolve(None, None, Some("gpt"), SelectionStrategy::Priority)
            .unwrap();
        assert_eq!(p.model_name(), "gpt");
    }

    #[tokio::test]
    async fn global_default_when_all_preferences_absent() {
        let models = vec![model("deepseek", "deepseek", &[], true)];
        let r = router(models, Arc::new(InMemoryHealthChecker::new())).await;
        let p = r
            .resolve(None, None, None, SelectionStrategy::Priority)
            .unwrap();
        assert_eq!(p.model_name(), "deepseek");
    }

    #[tokio::test]
    async fn fallback_on_unhealthy() {
        let models = vec![
            model("deepseek", "deepseek", &["gpt"], true),
            model("gpt", "openai", &[], false),
        ];
        let health = Arc::new(InMemoryHealthChecker::new());
        health.mark_unhealthy("deepseek");
        let r = router(models, health).await;
        // Task asks deepseek, but it's unhealthy → fallback to gpt.
        let p = r
            .resolve(Some("deepseek"), None, None, SelectionStrategy::Priority)
            .unwrap();
        assert_eq!(p.model_name(), "gpt");
    }

    #[tokio::test]
    async fn unhealthy_global_default_falls_back() {
        let models = vec![
            model("deepseek", "deepseek", &["gpt"], true),
            model("gpt", "openai", &[], false),
        ];
        let health = Arc::new(InMemoryHealthChecker::new());
        health.mark_unhealthy("deepseek");
        let r = router(models, health).await;
        // No explicit model → global default (deepseek) unhealthy → fallback gpt.
        let p = r
            .resolve(None, None, None, SelectionStrategy::Priority)
            .unwrap();
        assert_eq!(p.model_name(), "gpt");
    }

    #[tokio::test]
    async fn no_available_model_when_all_unhealthy() {
        let models = vec![
            model("deepseek", "deepseek", &["gpt"], true),
            model("gpt", "openai", &["deepseek"], false),
        ];
        let health = Arc::new(InMemoryHealthChecker::new());
        health.mark_unhealthy("deepseek");
        health.mark_unhealthy("gpt");
        let r = router(models, health).await;
        let err = r
            .resolve(Some("deepseek"), None, None, SelectionStrategy::Priority)
            .err()
            .expect("expected an error");
        assert_eq!(err, ModelRouterError::NoAvailableModel);
    }

    #[tokio::test]
    async fn turboauto_strategy_is_unsupported() {
        let models = vec![model("deepseek", "deepseek", &[], true)];
        let r = router(models, Arc::new(InMemoryHealthChecker::new())).await;
        let err = r
            .resolve(None, None, None, SelectionStrategy::TurboAuto)
            .err()
            .expect("expected an error");
        assert_eq!(
            err,
            ModelRouterError::UnsupportedStrategy(SelectionStrategy::TurboAuto)
        );
    }

    #[tokio::test]
    async fn provider_model_string_accepted() {
        let models = vec![model("deepseek", "deepseek", &[], true)];
        let r = router(models, Arc::new(InMemoryHealthChecker::new())).await;
        // "deepseek/deepseek-v4" maps to the model whose provider == "deepseek".
        let p = r
            .resolve(
                Some("deepseek/deepseek-v4"),
                None,
                None,
                SelectionStrategy::Priority,
            )
            .unwrap();
        assert_eq!(p.model_name(), "deepseek");
    }
}
