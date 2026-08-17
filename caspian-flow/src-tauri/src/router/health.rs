//! Model availability probing for P24 fallback.
//!
//! The [`HealthChecker`] trait decouples the router's fallback logic from the
//! actual probing mechanism. Two implementations ship:
//!
//! - [`InMemoryHealthChecker`]: a purely in-memory, dependency-free tracker
//!   that starts every model as healthy and can be flipped by
//!   `mark_unhealthy` / `mark_healthy`. This is what the CI test-suite and the
//!   runtime "mark on call failure, retry later" loop use.
//! - [`HttpHealthChecker`]: a real network probe (`GET {base_url}/models`,
//!   3-second timeout). Because it hits external endpoints, all of its live
//!   tests are gated behind `#[ignore]`.
//!
//! Design note (C3): the CI-visible path is fully mockable; real network
//! probing is opt-in only, matching the P22/P23 discipline of never depending
//! on external services in the default `cargo test` run.

use std::collections::HashMap;

use parking_lot::RwLock;

/// A pluggable model-availability checker.
pub trait HealthChecker: Send + Sync {
    /// Returns whether the given model id is currently considered healthy.
    ///
    /// Unknown model ids are treated as healthy by default (the router only
    /// records *un*healthy state; a never-probed model is assumed usable).
    fn is_healthy(&self, model_id: &str) -> bool;

    /// The set of model ids currently considered healthy (best-effort; may be
    /// empty for checkers that don't track a full registry).
    fn healthy_models(&self) -> Vec<String>;

    /// Trigger a (re)check of all tracked models. For the in-memory checker
    /// this simply clears transient unhealthy marks so they can be re-probed.
    fn refresh(&self);
}

/// In-memory health tracker — the default, dependency-free checker.
///
/// Every model id is healthy until explicitly marked unhealthy. This backs
/// both the CI tests and the runtime "call failed → mark unhealthy → retry
/// probe later" degradation loop.
#[derive(Default)]
pub struct InMemoryHealthChecker {
    /// model_id -> healthy?  (absent = healthy by default)
    state: RwLock<HashMap<String, bool>>,
}

impl InMemoryHealthChecker {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(HashMap::new()),
        }
    }

    /// Seed the tracker with an explicit set of (model_id, healthy) entries.
    pub fn with_states<I, S>(entries: I) -> Self
    where
        I: IntoIterator<Item = (S, bool)>,
        S: Into<String>,
    {
        let mut map = HashMap::new();
        for (id, healthy) in entries {
            map.insert(id.into(), healthy);
        }
        Self {
            state: RwLock::new(map),
        }
    }

    /// Mark a model unavailable (e.g. after a failed call). The router will
    /// skip it on the next `resolve`.
    pub fn mark_unhealthy(&self, model_id: &str) {
        self.state.write().insert(model_id.to_string(), false);
    }

    /// Mark a model available again (e.g. after a successful re-probe).
    pub fn mark_healthy(&self, model_id: &str) {
        self.state.write().insert(model_id.to_string(), true);
    }
}

impl HealthChecker for InMemoryHealthChecker {
    fn is_healthy(&self, model_id: &str) -> bool {
        // Absent = healthy by default.
        *self.state.read().get(model_id).unwrap_or(&true)
    }

    fn healthy_models(&self) -> Vec<String> {
        self.state
            .read()
            .iter()
            .filter(|(_, healthy)| **healthy)
            .map(|(id, _)| id.clone())
            .collect()
    }

    fn refresh(&self) {
        // Clear transient unhealthy marks so the next resolve re-probes them.
        self.state.write().retain(|_, healthy| *healthy);
    }
}

/// Real HTTP health checker: probes `GET {base_url}/models` with a short
/// timeout. Network-dependent, so its live tests are `#[ignore]`.
pub struct HttpHealthChecker {
    client: reqwest::Client,
    /// model_id -> base_url to probe.
    endpoints: HashMap<String, String>,
    /// Cached results from the last probe pass.
    cache: RwLock<HashMap<String, bool>>,
}

impl HttpHealthChecker {
    /// Build a checker over a map of `model_id -> base_url`.
    pub fn new(endpoints: HashMap<String, String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .expect("reqwest client");
        Self {
            client,
            endpoints,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Probe a single endpoint. Returns true on any 2xx/4xx (endpoint alive,
    /// even if auth is missing); false on connect/timeout errors.
    async fn probe(&self, base_url: &str) -> bool {
        let url = format!("{}/models", base_url.trim_end_matches('/'));
        match self.client.get(&url).send().await {
            // A response — even 401/403 — means the endpoint is reachable.
            Ok(resp) => resp.status().as_u16() < 500,
            Err(_) => false,
        }
    }

    /// Probe all configured endpoints and update the cache.
    pub async fn probe_all(&self) {
        let mut results = HashMap::new();
        for (id, url) in &self.endpoints {
            results.insert(id.clone(), self.probe(url).await);
        }
        *self.cache.write() = results;
    }
}

impl HealthChecker for HttpHealthChecker {
    fn is_healthy(&self, model_id: &str) -> bool {
        // Absent from cache = not yet probed = assume healthy.
        *self.cache.read().get(model_id).unwrap_or(&true)
    }

    fn healthy_models(&self) -> Vec<String> {
        self.cache
            .read()
            .iter()
            .filter(|(_, h)| **h)
            .map(|(id, _)| id.clone())
            .collect()
    }

    fn refresh(&self) {
        // Synchronous trait method: clear cache so the next async probe_all
        // re-populates. (Callers schedule probe_all on their runtime.)
        self.cache.write().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_model_is_healthy_by_default() {
        let hc = InMemoryHealthChecker::new();
        assert!(hc.is_healthy("never-seen"));
    }

    #[test]
    fn mark_unhealthy_then_healthy() {
        let hc = InMemoryHealthChecker::new();
        hc.mark_unhealthy("m1");
        assert!(!hc.is_healthy("m1"));
        hc.mark_healthy("m1");
        assert!(hc.is_healthy("m1"));
    }

    #[test]
    fn with_states_seeds_correctly() {
        let hc = InMemoryHealthChecker::with_states([("good", true), ("bad", false)]);
        assert!(hc.is_healthy("good"));
        assert!(!hc.is_healthy("bad"));
    }

    #[test]
    fn healthy_models_lists_only_healthy() {
        let hc = InMemoryHealthChecker::with_states([("a", true), ("b", false), ("c", true)]);
        let mut healthy = hc.healthy_models();
        healthy.sort();
        assert_eq!(healthy, vec!["a".to_string(), "c".to_string()]);
    }

    #[test]
    fn refresh_clears_unhealthy_marks() {
        let hc = InMemoryHealthChecker::new();
        hc.mark_unhealthy("m1");
        assert!(!hc.is_healthy("m1"));
        hc.refresh();
        // After refresh, the unhealthy mark is dropped → healthy by default.
        assert!(hc.is_healthy("m1"));
    }

    // Live network probe — gated.
    #[tokio::test]
    #[ignore]
    async fn live_http_probe() {
        let mut endpoints = HashMap::new();
        endpoints.insert(
            "openai".to_string(),
            "https://api.openai.com/v1".to_string(),
        );
        let hc = HttpHealthChecker::new(endpoints);
        hc.probe_all().await;
        // OpenAI's /models endpoint responds (401 without a key) → reachable.
        assert!(hc.is_healthy("openai"));
    }
}
