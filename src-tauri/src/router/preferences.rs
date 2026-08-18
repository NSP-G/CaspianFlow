//! Skill preference weights and routing sensitivity management.
//!
//! `SkillPreferences` stores per-skill weight multipliers that adjust
//! routing scores. The `Sensitivity` setting controls the global
//! high/low thresholds for route decisions.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::router::types::Sensitivity;

/// Minimum and maximum allowed weight values.
pub const MIN_WEIGHT: f64 = 0.1;
pub const MAX_WEIGHT: f64 = 5.0;
/// Default weight for skills without an explicit preference.
pub const DEFAULT_WEIGHT: f64 = 1.0;

/// Per-skill preference weights.
///
/// Weights are applied multiplicatively to the raw cosine similarity score:
/// `weighted_score = raw_similarity * weight`.
///
/// A weight > 1.0 makes a skill easier to match; < 1.0 makes it harder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPreferences {
    /// Map of skill name to weight.
    weights: HashMap<String, f64>,
    /// Current sensitivity preset (controls thresholds).
    #[serde(default)]
    sensitivity: Sensitivity,
}

impl Default for SkillPreferences {
    fn default() -> Self {
        Self {
            weights: HashMap::new(),
            sensitivity: Sensitivity::Balanced,
        }
    }
}

impl SkillPreferences {
    /// Create a new empty preference set with balanced sensitivity.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the weight for a skill (defaults to 1.0).
    pub fn get_weight(&self, skill_name: &str) -> f64 {
        self.weights
            .get(skill_name)
            .copied()
            .unwrap_or(DEFAULT_WEIGHT)
    }

    /// Set the weight for a skill, clamped to `[MIN_WEIGHT, MAX_WEIGHT]`.
    pub fn set_weight(&mut self, skill_name: &str, weight: f64) {
        let clamped = weight.clamp(MIN_WEIGHT, MAX_WEIGHT);
        self.weights.insert(skill_name.to_string(), clamped);
    }

    /// Adjust a skill's weight by a delta, clamped to valid range.
    pub fn adjust_weight(&mut self, skill_name: &str, delta: f64) {
        let current = self.get_weight(skill_name);
        self.set_weight(skill_name, current + delta);
    }

    /// Remove a skill's custom weight (reverts to default 1.0).
    pub fn remove_weight(&mut self, skill_name: &str) {
        self.weights.remove(skill_name);
    }

    /// Get all custom weights.
    pub fn all_weights(&self) -> &HashMap<String, f64> {
        &self.weights
    }

    /// Get the current sensitivity preset.
    pub fn sensitivity(&self) -> Sensitivity {
        self.sensitivity
    }

    /// Set the sensitivity preset.
    pub fn set_sensitivity(&mut self, sensitivity: Sensitivity) {
        self.sensitivity = sensitivity;
    }

    /// Get the `(high_threshold, low_threshold)` for the current sensitivity.
    pub fn thresholds(&self) -> (f64, f64) {
        self.sensitivity.thresholds()
    }

    /// Apply weight to a raw similarity score.
    pub fn apply_weight(&self, skill_name: &str, raw_score: f32) -> f32 {
        raw_score * self.get_weight(skill_name) as f32
    }

    /// Number of skills with custom weights.
    pub fn len(&self) -> usize {
        self.weights.len()
    }

    /// Whether there are no custom weights.
    pub fn is_empty(&self) -> bool {
        self.weights.is_empty()
    }

    /// Clear all custom weights.
    pub fn clear(&mut self) {
        self.weights.clear();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_weight() {
        let prefs = SkillPreferences::new();
        assert_eq!(prefs.get_weight("any_skill"), DEFAULT_WEIGHT);
        assert!(prefs.is_empty());
    }

    #[test]
    fn test_set_and_get_weight() {
        let mut prefs = SkillPreferences::new();
        prefs.set_weight("read_file", 1.5);
        assert_eq!(prefs.get_weight("read_file"), 1.5);
        assert_eq!(prefs.len(), 1);
    }

    #[test]
    fn test_weight_clamping() {
        let mut prefs = SkillPreferences::new();

        // Above max
        prefs.set_weight("skill_a", 10.0);
        assert_eq!(prefs.get_weight("skill_a"), MAX_WEIGHT);

        // Below min
        prefs.set_weight("skill_b", 0.01);
        assert_eq!(prefs.get_weight("skill_b"), MIN_WEIGHT);
    }

    #[test]
    fn test_adjust_weight() {
        let mut prefs = SkillPreferences::new();
        prefs.set_weight("skill_a", 1.0);

        prefs.adjust_weight("skill_a", 0.05);
        assert!((prefs.get_weight("skill_a") - 1.05).abs() < 1e-10);

        prefs.adjust_weight("skill_a", -0.10);
        assert!((prefs.get_weight("skill_a") - 0.95).abs() < 1e-10);
    }

    #[test]
    fn test_adjust_weight_clamps() {
        let mut prefs = SkillPreferences::new();
        prefs.adjust_weight("skill_a", -100.0);
        assert_eq!(prefs.get_weight("skill_a"), MIN_WEIGHT);

        prefs.adjust_weight("skill_b", 100.0);
        assert_eq!(prefs.get_weight("skill_b"), MAX_WEIGHT);
    }

    #[test]
    fn test_remove_weight() {
        let mut prefs = SkillPreferences::new();
        prefs.set_weight("skill_a", 2.0);
        assert_eq!(prefs.get_weight("skill_a"), 2.0);

        prefs.remove_weight("skill_a");
        assert_eq!(prefs.get_weight("skill_a"), DEFAULT_WEIGHT);
        assert!(prefs.is_empty());
    }

    #[test]
    fn test_apply_weight() {
        let mut prefs = SkillPreferences::new();
        prefs.set_weight("skill_a", 1.5);

        let raw = 0.80_f32;
        let weighted = prefs.apply_weight("skill_a", raw);
        assert!((weighted - 1.20).abs() < 1e-5);
    }

    #[test]
    fn test_sensitivity() {
        let mut prefs = SkillPreferences::new();
        assert_eq!(prefs.sensitivity(), Sensitivity::Balanced);

        let (h, l) = prefs.thresholds();
        assert_eq!(h, 0.82);
        assert_eq!(l, 0.65);

        prefs.set_sensitivity(Sensitivity::Conservative);
        let (h, l) = prefs.thresholds();
        assert_eq!(h, 0.85);
        assert_eq!(l, 0.70);

        prefs.set_sensitivity(Sensitivity::Aggressive);
        let (h, l) = prefs.thresholds();
        assert_eq!(h, 0.75);
        assert_eq!(l, 0.55);
    }

    #[test]
    fn test_clear() {
        let mut prefs = SkillPreferences::new();
        prefs.set_weight("a", 1.5);
        prefs.set_weight("b", 2.0);
        assert_eq!(prefs.len(), 2);

        prefs.clear();
        assert!(prefs.is_empty());
    }

    #[test]
    fn test_all_weights() {
        let mut prefs = SkillPreferences::new();
        prefs.set_weight("a", 1.5);
        prefs.set_weight("b", 2.0);

        let weights = prefs.all_weights();
        assert_eq!(weights.len(), 2);
        assert_eq!(weights["a"], 1.5);
        assert_eq!(weights["b"], 2.0);
    }
}
