//! Feedback store and cold-start tracker.
//!
//! ## FeedbackStore
//!
//! Records user feedback (implicit and explicit) after a skill match.
//! Feedback data is used to adjust skill preference weights over time.
//!
//! ## ColdStartTracker
//!
//! During the first `COLD_START_THRESHOLD` (10) routing calls, if the user
//! selects a candidate that is NOT the router's top-1, a correction sample
//! is recorded. After `CORRECTION_PROMOTION_THRESHOLD` (5) corrections for
//! the same skill, that skill's weight is automatically boosted by 0.1.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::RwLock;

use crate::router::preferences::SkillPreferences;
use crate::router::types::{CorrectionSample, FeedbackRecord, FeedbackType};

/// Number of routing calls considered "cold start" (first N calls).
pub const COLD_START_THRESHOLD: usize = 10;

/// Number of corrections needed before auto-promoting a skill's weight.
pub const CORRECTION_PROMOTION_THRESHOLD: usize = 5;

/// Weight boost applied when a skill is auto-promoted via corrections.
pub const CORRECTION_WEIGHT_BOOST: f64 = 0.1;

// ---------------------------------------------------------------------------
// FeedbackStore
// ---------------------------------------------------------------------------

/// Stores feedback records and applies weight adjustments to `SkillPreferences`.
pub struct FeedbackStore {
    records: RwLock<Vec<FeedbackRecord>>,
}

impl FeedbackStore {
    /// Create an empty feedback store.
    pub fn new() -> Self {
        Self {
            records: RwLock::new(Vec::new()),
        }
    }

    /// Record a feedback event and apply the weight adjustment.
    ///
    /// Returns the weight delta that was applied.
    pub fn record(
        &self,
        user_input: &str,
        matched_skill: &str,
        score: f32,
        feedback_type: FeedbackType,
        preferences: &mut SkillPreferences,
    ) -> f64 {
        let delta = feedback_type.weight_delta();
        let timestamp = current_timestamp();

        let record = FeedbackRecord {
            user_input: user_input.to_string(),
            matched_skill: matched_skill.to_string(),
            score,
            feedback_type: feedback_type.to_string(),
            timestamp,
        };

        self.records.write().push(record);

        // Apply weight adjustment
        preferences.adjust_weight(matched_skill, delta);

        tracing::info!(
            skill = matched_skill,
            feedback = %feedback_type,
            delta,
            "feedback recorded and weight adjusted"
        );

        delta
    }

    /// Get all feedback records (clone).
    pub fn records(&self) -> Vec<FeedbackRecord> {
        self.records.read().clone()
    }

    /// Number of feedback records.
    pub fn len(&self) -> usize {
        self.records.read().len()
    }

    /// Whether there are no feedback records.
    pub fn is_empty(&self) -> bool {
        self.records.read().is_empty()
    }

    /// Get feedback records for a specific skill.
    pub fn records_for_skill(&self, skill_name: &str) -> Vec<FeedbackRecord> {
        self.records
            .read()
            .iter()
            .filter(|r| r.matched_skill == skill_name)
            .cloned()
            .collect()
    }

    /// Clear all feedback records.
    pub fn clear(&self) {
        self.records.write().clear();
    }
}

impl Default for FeedbackStore {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ColdStartTracker
// ---------------------------------------------------------------------------

/// Tracks routing usage during the cold-start period and records corrections.
pub struct ColdStartTracker {
    /// Total number of routing calls so far.
    usage_count: RwLock<usize>,
    /// Correction samples collected during cold start.
    corrections: RwLock<Vec<CorrectionSample>>,
    /// Per-skill correction count (for auto-promotion).
    correction_counts: RwLock<HashMap<String, usize>>,
}

impl ColdStartTracker {
    /// Create a new cold-start tracker.
    pub fn new() -> Self {
        Self {
            usage_count: RwLock::new(0),
            corrections: RwLock::new(Vec::new()),
            correction_counts: RwLock::new(HashMap::new()),
        }
    }

    /// Increment the usage counter. Called on every routing call.
    pub fn increment_usage(&self) {
        *self.usage_count.write() += 1;
    }

    /// Whether we are still in the cold-start period.
    pub fn is_cold_start(&self) -> bool {
        *self.usage_count.read() < COLD_START_THRESHOLD
    }

    /// Current usage count.
    pub fn usage_count(&self) -> usize {
        *self.usage_count.read()
    }

    /// Record a correction: the user selected a non-top-1 candidate.
    ///
    /// If the correction count for the selected skill reaches
    /// `CORRECTION_PROMOTION_THRESHOLD`, the skill's weight is auto-boosted
    /// and the correction count is reset.
    ///
    /// Returns `Some(weight_boost)` if a promotion was applied, `None` otherwise.
    pub fn record_correction(
        &self,
        user_input: &str,
        top_1_skill: &str,
        selected_skill: &str,
        preferences: &mut SkillPreferences,
    ) -> Option<f64> {
        // Only record during cold start
        if !self.is_cold_start() {
            return None;
        }

        // Only record if the user selected a different skill than top-1
        if top_1_skill == selected_skill {
            return None;
        }

        let sample = CorrectionSample {
            user_input: user_input.to_string(),
            top_1_skill: top_1_skill.to_string(),
            selected_skill: selected_skill.to_string(),
            timestamp: current_timestamp(),
        };

        self.corrections.write().push(sample);

        // Increment per-skill correction count
        let mut counts = self.correction_counts.write();
        let count = counts.entry(selected_skill.to_string()).or_insert(0);
        *count += 1;

        tracing::info!(
            selected_skill = selected_skill,
            top_1_skill = top_1_skill,
            correction_count = *count,
            "correction recorded during cold start"
        );

        // Check for auto-promotion
        if *count >= CORRECTION_PROMOTION_THRESHOLD {
            preferences.adjust_weight(selected_skill, CORRECTION_WEIGHT_BOOST);
            *count = 0; // reset after promotion
            tracing::info!(
                skill = selected_skill,
                boost = CORRECTION_WEIGHT_BOOST,
                "skill auto-promoted due to correction accumulation"
            );
            return Some(CORRECTION_WEIGHT_BOOST);
        }

        None
    }

    /// Get all correction samples.
    pub fn corrections(&self) -> Vec<CorrectionSample> {
        self.corrections.read().clone()
    }

    /// Number of corrections recorded.
    pub fn correction_count(&self) -> usize {
        self.corrections.read().len()
    }

    /// Clear all correction data.
    pub fn clear(&self) {
        *self.usage_count.write() = 0;
        self.corrections.write().clear();
        self.correction_counts.write().clear();
    }
}

impl Default for ColdStartTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the current Unix timestamp in seconds.
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feedback_store_record() {
        let store = FeedbackStore::new();
        let mut prefs = SkillPreferences::new();

        let delta = store.record(
            "read file",
            "read_file",
            0.85,
            FeedbackType::ExplicitPositive,
            &mut prefs,
        );

        assert!((delta - 0.05).abs() < 1e-10);
        assert_eq!(store.len(), 1);
        assert!((prefs.get_weight("read_file") - 1.05).abs() < 1e-10);
    }

    #[test]
    fn test_feedback_store_negative() {
        let store = FeedbackStore::new();
        let mut prefs = SkillPreferences::new();

        store.record(
            "read file",
            "read_file",
            0.85,
            FeedbackType::ExplicitNegative,
            &mut prefs,
        );

        assert!((prefs.get_weight("read_file") - 0.90).abs() < 1e-10);
    }

    #[test]
    fn test_feedback_store_multiple() {
        let store = FeedbackStore::new();
        let mut prefs = SkillPreferences::new();

        store.record(
            "a",
            "skill_a",
            0.8,
            FeedbackType::ImplicitSuccess,
            &mut prefs,
        );
        store.record(
            "b",
            "skill_b",
            0.7,
            FeedbackType::ImplicitFailure,
            &mut prefs,
        );
        store.record(
            "c",
            "skill_a",
            0.9,
            FeedbackType::ExplicitPositive,
            &mut prefs,
        );

        assert_eq!(store.len(), 3);

        let skill_a_records = store.records_for_skill("skill_a");
        assert_eq!(skill_a_records.len(), 2);

        let skill_b_records = store.records_for_skill("skill_b");
        assert_eq!(skill_b_records.len(), 1);
    }

    #[test]
    fn test_feedback_store_clear() {
        let store = FeedbackStore::new();
        let mut prefs = SkillPreferences::new();

        store.record(
            "a",
            "skill_a",
            0.8,
            FeedbackType::ImplicitSuccess,
            &mut prefs,
        );
        assert!(!store.is_empty());

        store.clear();
        assert!(store.is_empty());
    }

    #[test]
    fn test_cold_start_initial() {
        let tracker = ColdStartTracker::new();
        assert!(tracker.is_cold_start());
        assert_eq!(tracker.usage_count(), 0);
        assert_eq!(tracker.correction_count(), 0);
    }

    #[test]
    fn test_cold_start_increment() {
        let tracker = ColdStartTracker::new();

        for _ in 0..5 {
            tracker.increment_usage();
        }
        assert!(tracker.is_cold_start());
        assert_eq!(tracker.usage_count(), 5);

        for _ in 0..5 {
            tracker.increment_usage();
        }
        assert!(!tracker.is_cold_start());
        assert_eq!(tracker.usage_count(), 10);
    }

    #[test]
    fn test_correction_recorded() {
        let tracker = ColdStartTracker::new();
        let mut prefs = SkillPreferences::new();

        tracker.increment_usage();

        let boost = tracker.record_correction("help me", "read_file", "write_file", &mut prefs);

        assert!(boost.is_none()); // Not enough corrections yet
        assert_eq!(tracker.correction_count(), 1);
    }

    #[test]
    fn test_correction_not_recorded_for_top1() {
        let tracker = ColdStartTracker::new();
        let mut prefs = SkillPreferences::new();

        tracker.increment_usage();

        let boost = tracker.record_correction(
            "help me",
            "read_file",
            "read_file", // same as top-1
            &mut prefs,
        );

        assert!(boost.is_none());
        assert_eq!(tracker.correction_count(), 0);
    }

    #[test]
    fn test_correction_not_recorded_after_cold_start() {
        let tracker = ColdStartTracker::new();
        let mut prefs = SkillPreferences::new();

        // Exhaust cold start period
        for _ in 0..COLD_START_THRESHOLD {
            tracker.increment_usage();
        }
        assert!(!tracker.is_cold_start());

        let boost = tracker.record_correction("help me", "read_file", "write_file", &mut prefs);

        assert!(boost.is_none());
        assert_eq!(tracker.correction_count(), 0);
    }

    #[test]
    fn test_correction_auto_promotion() {
        let tracker = ColdStartTracker::new();
        let mut prefs = SkillPreferences::new();

        // Record CORRECTION_PROMOTION_THRESHOLD corrections for the same skill
        for i in 0..CORRECTION_PROMOTION_THRESHOLD {
            tracker.increment_usage();
            let boost = tracker.record_correction(
                &format!("help {i}"),
                "read_file",
                "write_file",
                &mut prefs,
            );

            if i == CORRECTION_PROMOTION_THRESHOLD - 1 {
                // Last correction should trigger promotion
                assert!(boost.is_some());
                assert!((boost.unwrap() - CORRECTION_WEIGHT_BOOST).abs() < 1e-10);
            } else {
                assert!(boost.is_none());
            }
        }

        // Weight should have been boosted
        assert!((prefs.get_weight("write_file") - (1.0 + CORRECTION_WEIGHT_BOOST)).abs() < 1e-10);
    }

    #[test]
    fn test_cold_start_clear() {
        let tracker = ColdStartTracker::new();
        let mut prefs = SkillPreferences::new();

        tracker.increment_usage();
        tracker.record_correction("a", "x", "y", &mut prefs);

        assert_eq!(tracker.usage_count(), 1);
        assert_eq!(tracker.correction_count(), 1);

        tracker.clear();

        assert_eq!(tracker.usage_count(), 0);
        assert_eq!(tracker.correction_count(), 0);
        assert!(tracker.is_cold_start());
    }

    #[test]
    fn test_feedback_weight_clamping() {
        let store = FeedbackStore::new();
        let mut prefs = SkillPreferences::new();

        // Apply many negative feedbacks — should clamp at MIN_WEIGHT
        for _ in 0..100 {
            store.record(
                "test",
                "skill_a",
                0.5,
                FeedbackType::ExplicitNegative,
                &mut prefs,
            );
        }

        assert_eq!(
            prefs.get_weight("skill_a"),
            crate::router::preferences::MIN_WEIGHT
        );
    }
}
