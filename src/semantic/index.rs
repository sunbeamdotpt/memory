// Semantic Index — cosine similarity search over in-memory vectors.
// Uses a HashMap so deletion is O(1) and the index stays consistent
// with the database after deletes.

use std::collections::HashMap;

pub struct SemanticIndex {
    vectors: HashMap<String, Vec<f32>>,
}

impl SemanticIndex {
    pub fn new(_dimension: usize) -> Self {
        Self {
            vectors: HashMap::new(),
        }
    }

    pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b.iter()).map(|(&x, &y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm_a == 0.0 || norm_b == 0.0 {
            0.0
        } else {
            dot / (norm_a * norm_b)
        }
    }

    pub fn add_vector(&mut self, vector: &[f32], id: &str) -> bool {
        self.vectors.insert(id.to_string(), vector.to_vec());
        true
    }

    /// Remove a vector by id. Returns true if it existed.
    pub fn remove_vector(&mut self, id: &str) -> bool {
        self.vectors.remove(id).is_some()
    }

    /// Return the top-k most similar ids with their scores, highest first.
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(String, f32)> {
        let mut results: Vec<(String, f32)> = self
            .vectors
            .iter()
            .map(|(id, vec)| (id.clone(), Self::cosine_similarity(query, vec)))
            .collect();

        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(k);
        results
    }
}
