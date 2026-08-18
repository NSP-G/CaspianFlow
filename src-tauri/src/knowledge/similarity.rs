//! Cosine similarity and vector utilities for semantic routing.
//!
//! These are pure functions with no external dependencies, making them
//! easy to unit-test and reuse in P12 (Semantic Router).

/// Compute the cosine similarity between two vectors.
///
/// Returns a value in `[-1.0, 1.0]`:
/// - `1.0` = identical direction
/// - `0.0` = orthogonal
/// - `-1.0` = opposite direction
///
/// Returns `0.0` if either vector has zero magnitude (avoids division by zero).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    if a.is_empty() {
        return 0.0;
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

/// L2-normalize a vector in place.
pub fn l2_normalize(vec: &mut [f32]) {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in vec.iter_mut() {
            *v /= norm;
        }
    }
}

/// Return a normalized copy of the vector.
pub fn normalized(vec: &[f32]) -> Vec<f32> {
    let mut out = vec.to_vec();
    l2_normalize(&mut out);
    out
}

/// Euclidean distance between two vectors.
pub fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return f32::MAX;
    }
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f32>()
        .sqrt()
}

/// Find the top-k indices by cosine similarity to a query vector.
///
/// Returns indices sorted by descending similarity.
pub fn top_k_by_similarity(query: &[f32], candidates: &[Vec<f32>], k: usize) -> Vec<(usize, f32)> {
    let mut scored: Vec<(usize, f32)> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| (i, cosine_similarity(query, c)))
        .collect();

    // Sort by similarity descending
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    scored.truncate(k);
    scored
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_vectors() {
        let a = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &a);
        assert!(
            (sim - 1.0).abs() < 1e-6,
            "identical vectors should have similarity 1.0, got {sim}"
        );
    }

    #[test]
    fn test_opposite_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![-1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            (sim + 1.0).abs() < 1e-6,
            "opposite vectors should have similarity -1.0, got {sim}"
        );
    }

    #[test]
    fn test_orthogonal_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            sim.abs() < 1e-6,
            "orthogonal vectors should have similarity 0.0, got {sim}"
        );
    }

    #[test]
    fn test_similar_texts_high_similarity() {
        // Simulate embeddings of similar texts
        let a = vec![0.8, 0.6, 0.0, 0.1];
        let b = vec![0.7, 0.7, 0.1, 0.1];
        let sim = cosine_similarity(&a, &b);
        assert!(
            sim > 0.9,
            "similar vectors should have high similarity, got {sim}"
        );
    }

    #[test]
    fn test_dissimilar_texts_low_similarity() {
        let a = vec![1.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 0.0, 0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            sim < 0.3,
            "dissimilar vectors should have low similarity, got {sim}"
        );
    }

    #[test]
    fn test_zero_vector() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        assert!(
            sim.abs() < 1e-6,
            "zero vector should yield similarity 0.0, got {sim}"
        );
    }

    #[test]
    fn test_different_lengths() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        assert_eq!(sim, 0.0, "different-length vectors should return 0.0");
    }

    #[test]
    fn test_empty_vectors() {
        let a: Vec<f32> = vec![];
        let b: Vec<f32> = vec![];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn test_l2_normalize() {
        let mut v = vec![3.0, 4.0];
        l2_normalize(&mut v);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-6,
            "normalized vector should have unit norm, got {norm}"
        );
    }

    #[test]
    fn test_normalized_copy() {
        let v = vec![0.0, 0.0, 5.0];
        let n = normalized(&v);
        assert!((n[2] - 1.0).abs() < 1e-6);
        // Original unchanged
        assert_eq!(v[2], 5.0);
    }

    #[test]
    fn test_euclidean_distance() {
        let a = vec![0.0, 0.0];
        let b = vec![3.0, 4.0];
        assert!((euclidean_distance(&a, &b) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn test_top_k() {
        let query = vec![1.0, 0.0];
        let candidates = vec![
            vec![0.9, 0.1], // high sim
            vec![0.0, 1.0], // low sim
            vec![0.8, 0.2], // medium sim
        ];

        let top = top_k_by_similarity(&query, &candidates, 2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, 0); // candidate 0 is most similar
        assert_eq!(top[1].0, 2); // candidate 2 is second
    }

    #[test]
    fn test_top_k_more_than_candidates() {
        let query = vec![1.0, 0.0];
        let candidates = vec![vec![1.0, 0.0], vec![0.0, 1.0]];

        let top = top_k_by_similarity(&query, &candidates, 5);
        assert_eq!(top.len(), 2); // can't return more than available
    }

    #[test]
    fn test_top_k_empty() {
        let query = vec![1.0, 0.0];
        let candidates: Vec<Vec<f32>> = vec![];
        let top = top_k_by_similarity(&query, &candidates, 3);
        assert!(top.is_empty());
    }
}
