//! Type definitions for the semantic router.
//!
//! These types are the public API surface of the router module.

use serde::{Deserialize, Serialize};

use crate::skill::schema::Skill;

// ---------------------------------------------------------------------------
// Route result
// ---------------------------------------------------------------------------

/// The outcome of routing a user query against the skill index.
#[derive(Debug, Clone)]
pub enum RouteResult {
    /// High confidence: a single skill matched above `high_threshold`.
    DirectMatch {
        skill: Box<Skill>,
        score: f32,
        threshold: f64,
    },

    /// Medium confidence: top-K candidates between `low_threshold` and `high_threshold`.
    Candidates {
        skills: Vec<ScoredSkill>,
        top_score: f32,
    },

    /// Low confidence: no skill matched above `low_threshold`.
    NoMatch { top_score: f32, threshold: f64 },
}

/// A skill paired with its routing score.
#[derive(Debug, Clone)]
pub struct ScoredSkill {
    pub skill: Skill,
    /// Raw cosine similarity (before weight adjustment).
    pub raw_score: f32,
    /// Final score after applying skill preference weight.
    pub weighted_score: f32,
}

// ---------------------------------------------------------------------------
// Feedback types
// ---------------------------------------------------------------------------

/// The kind of feedback a user can provide after a skill match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackType {
    /// Implicit: the skill executed successfully.
    ImplicitSuccess,
    /// Implicit: the skill execution failed.
    ImplicitFailure,
    /// Explicit: the user gave a thumbs-up.
    ExplicitPositive,
    /// Explicit: the user gave a thumbs-down.
    ExplicitNegative,
}

impl FeedbackType {
    /// The weight delta applied to a skill's preference when this feedback is recorded.
    pub fn weight_delta(self) -> f64 {
        match self {
            Self::ImplicitSuccess => 0.02,
            Self::ImplicitFailure => -0.03,
            Self::ExplicitPositive => 0.05,
            Self::ExplicitNegative => -0.10,
        }
    }
}

impl std::fmt::Display for FeedbackType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ImplicitSuccess => write!(f, "implicit_success"),
            Self::ImplicitFailure => write!(f, "implicit_failure"),
            Self::ExplicitPositive => write!(f, "explicit_positive"),
            Self::ExplicitNegative => write!(f, "explicit_negative"),
        }
    }
}

/// A single feedback record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackRecord {
    pub user_input: String,
    pub matched_skill: String,
    pub score: f32,
    pub feedback_type: String,
    /// Unix timestamp (seconds).
    pub timestamp: u64,
}

/// A correction sample collected during cold-start.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrectionSample {
    pub user_input: String,
    /// The skill the router ranked first.
    pub top_1_skill: String,
    /// The skill the user actually selected.
    pub selected_skill: String,
    /// Unix timestamp (seconds).
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// Sensitivity
// ---------------------------------------------------------------------------

/// Sensitivity preset for routing thresholds.
///
/// Controls how aggressive the router is in matching skills.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    /// Conservative: requires higher confidence for a match.
    Conservative,
    /// Balanced: the default setting.
    #[default]
    Balanced,
    /// Aggressive: matches more readily.
    Aggressive,
}

impl Sensitivity {
    /// Returns `(high_threshold, low_threshold)` for this preset.
    pub fn thresholds(self) -> (f64, f64) {
        match self {
            Self::Conservative => (0.85, 0.70),
            Self::Balanced => (0.82, 0.65),
            Self::Aggressive => (0.75, 0.55),
        }
    }
}

impl std::fmt::Display for Sensitivity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conservative => write!(f, "conservative"),
            Self::Balanced => write!(f, "balanced"),
            Self::Aggressive => write!(f, "aggressive"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feedback_weight_deltas() {
        assert!((FeedbackType::ImplicitSuccess.weight_delta() - 0.02).abs() < 1e-10);
        assert!((FeedbackType::ImplicitFailure.weight_delta() - (-0.03)).abs() < 1e-10);
        assert!((FeedbackType::ExplicitPositive.weight_delta() - 0.05).abs() < 1e-10);
        assert!((FeedbackType::ExplicitNegative.weight_delta() - (-0.10)).abs() < 1e-10);
    }

    #[test]
    fn test_sensitivity_thresholds() {
        let (h, l) = Sensitivity::Conservative.thresholds();
        assert_eq!(h, 0.85);
        assert_eq!(l, 0.70);

        let (h, l) = Sensitivity::Balanced.thresholds();
        assert_eq!(h, 0.82);
        assert_eq!(l, 0.65);

        let (h, l) = Sensitivity::Aggressive.thresholds();
        assert_eq!(h, 0.75);
        assert_eq!(l, 0.55);
    }

    #[test]
    fn test_sensitivity_default() {
        assert_eq!(Sensitivity::default(), Sensitivity::Balanced);
    }

    #[test]
    fn test_feedback_type_display() {
        assert_eq!(
            FeedbackType::ImplicitSuccess.to_string(),
            "implicit_success"
        );
        assert_eq!(
            FeedbackType::ExplicitNegative.to_string(),
            "explicit_negative"
        );
    }

    #[test]
    fn test_sensitivity_serde() {
        let json = serde_json::to_string(&Sensitivity::Aggressive).unwrap();
        assert_eq!(json, "\"aggressive\"");

        let s: Sensitivity = serde_json::from_str("\"balanced\"").unwrap();
        assert_eq!(s, Sensitivity::Balanced);
    }
}
