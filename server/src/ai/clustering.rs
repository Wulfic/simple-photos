//! Agglomerative face clustering using cosine similarity of face embeddings.
//!
//! Groups detected faces into identity clusters so that all photos of the
//! same person are automatically linked. The user can then name clusters
//! ("Mom", "John"), merge duplicates, and split mis-grouped faces.

use crate::ai::face::cosine_similarity;

use tracing;

/// A cluster assignment: (face_detection_id, cluster_id).
pub type ClusterAssignment = (i64, i64);

/// Run agglomerative clustering on face embeddings.
///
/// Takes a list of (face_detection_id, embedding) pairs and returns
/// cluster assignments. Faces with similarity above `threshold` are
/// merged into the same cluster.
///
/// This is O(n²) on the number of faces — suitable for personal photo
/// libraries (typically < 100k faces). For larger datasets, consider
/// approximate nearest neighbour algorithms.
pub fn cluster_faces(faces: &[(i64, Vec<f32>)], threshold: f32) -> Vec<ClusterAssignment> {
    if faces.is_empty() {
        return vec![];
    }

    let n = faces.len();
    tracing::debug!(
        face_count = n,
        threshold = threshold,
        "Face clustering: beginning agglomerative pass"
    );

    // Start with each face in its own cluster
    let mut cluster_ids: Vec<usize> = (0..n).collect();

    // Build pairwise similarity matrix (upper triangle only)
    // For large N this could be optimised, but for personal photo libraries
    // (typically <100k faces) this is acceptable.
    let mut similarities: Vec<(usize, usize, f32)> = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            // Skip comparing embeddings of different dimensionality
            // (e.g. 512-dim ArcFace vs 128-dim histogram fallback)
            if faces[i].1.len() != faces[j].1.len() {
                continue;
            }
            let sim = cosine_similarity(&faces[i].1, &faces[j].1);
            if sim >= threshold * 0.8 {
                // Only store pairs that might merge
                similarities.push((i, j, sim));
            }
        }
    }

    // Sort by similarity descending
    similarities.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    tracing::debug!(
        candidate_pairs = similarities.len(),
        "Face clustering: similarity pairs computed"
    );

    // Merge clusters greedily (single-linkage)
    let mut merges = 0usize;
    for (i, j, sim) in &similarities {
        if *sim < threshold {
            break;
        }
        let ci = find_root(&cluster_ids, *i);
        let cj = find_root(&cluster_ids, *j);
        if ci != cj {
            // Merge: assign all members of cj to ci
            let target = ci.min(cj);
            let source = ci.max(cj);
            for k in 0..n {
                if find_root(&cluster_ids, k) == source {
                    cluster_ids[k] = target;
                }
            }
            merges += 1;
        }
    }

    // Flatten cluster IDs to contiguous values
    let mut cluster_map: std::collections::HashMap<usize, i64> = std::collections::HashMap::new();
    let mut next_id: i64 = 1;

    let result: Vec<ClusterAssignment> = faces
        .iter()
        .enumerate()
        .map(|(idx, (face_id, _))| {
            let root = find_root(&cluster_ids, idx);
            let cid = *cluster_map.entry(root).or_insert_with(|| {
                let id = next_id;
                next_id += 1;
                id
            });
            (*face_id, cid)
        })
        .collect();

    let unique_output_clusters = cluster_map.len();
    tracing::debug!(
        input_faces = n,
        merges_performed = merges,
        output_clusters = unique_output_clusters,
        "Face clustering: agglomerative pass complete"
    );

    result
}

/// Find the root cluster for an element (path compression style but iterative).
fn find_root(clusters: &[usize], mut idx: usize) -> usize {
    while clusters[idx] != idx {
        idx = clusters[idx];
    }
    idx
}

/// Compute the average (centroid) embedding for a group of face embeddings.
#[allow(dead_code)] // Part of planned incremental clustering enhancement
pub fn centroid_embedding(embeddings: &[&[f32]]) -> Vec<f32> {
    if embeddings.is_empty() {
        return vec![];
    }
    let dim = embeddings[0].len();
    let mut centroid = vec![0.0f32; dim];
    for emb in embeddings {
        for (i, v) in emb.iter().enumerate() {
            if i < dim {
                centroid[i] += v;
            }
        }
    }
    let n = embeddings.len() as f32;
    for v in &mut centroid {
        *v /= n;
    }

    // L2 normalise
    let norm: f32 = centroid.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 1e-6 {
        for v in &mut centroid {
            *v /= norm;
        }
    }

    centroid
}

/// Determine if a face embedding is close enough to an existing cluster
/// (represented by its centroid) to be assigned to that cluster.
#[allow(dead_code)] // Part of planned incremental clustering enhancement
pub fn should_assign_to_cluster(
    face_embedding: &[f32],
    cluster_centroid: &[f32],
    threshold: f32,
) -> bool {
    cosine_similarity(face_embedding, cluster_centroid) >= threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cluster_identical_faces() {
        let emb = vec![1.0, 0.0, 0.0, 0.0];
        let faces = vec![(1, emb.clone()), (2, emb.clone()), (3, emb.clone())];
        let assignments = cluster_faces(&faces, 0.7);
        assert_eq!(assignments.len(), 3);
        // All should be in the same cluster
        assert_eq!(assignments[0].1, assignments[1].1);
        assert_eq!(assignments[1].1, assignments[2].1);
    }

    #[test]
    fn test_cluster_different_faces() {
        let faces = vec![
            (1, vec![1.0, 0.0, 0.0, 0.0]),
            (2, vec![0.0, 1.0, 0.0, 0.0]),
            (3, vec![0.0, 0.0, 1.0, 0.0]),
        ];
        let assignments = cluster_faces(&faces, 0.7);
        assert_eq!(assignments.len(), 3);
        // All should be in different clusters
        assert_ne!(assignments[0].1, assignments[1].1);
        assert_ne!(assignments[1].1, assignments[2].1);
    }

    #[test]
    fn test_cluster_empty() {
        let assignments = cluster_faces(&[], 0.7);
        assert!(assignments.is_empty());
    }

    #[test]
    fn test_centroid() {
        let e1 = vec![1.0, 0.0];
        let e2 = vec![0.0, 1.0];
        let centroid = centroid_embedding(&[&e1, &e2]);
        assert_eq!(centroid.len(), 2);
        // Should be normalised: [0.707, 0.707] approximately
        assert!((centroid[0] - centroid[1]).abs() < 0.01);
    }

    /// Regression for todo P0-3: clustering MUST use the actual stored
    /// embedding vectors and cosine similarity, not detection-id timing
    /// or any other proxy.  Two near-identical 512-d vectors and a third
    /// orthogonal one must produce exactly two clusters of sizes 2 and 1.
    #[test]
    fn test_cluster_uses_cosine_similarity_512d() {
        // Build a 512-d vector that looks like a real ArcFace embedding.
        let base: Vec<f32> = (0..512).map(|i| ((i as f32) * 0.0123).sin()).collect();
        let l2 = base.iter().map(|v| v * v).sum::<f32>().sqrt();
        let normed: Vec<f32> = base.iter().map(|v| v / l2).collect();

        // Near-identical: tiny gaussian-style perturbation, then re-normalise.
        let mut perturbed: Vec<f32> = normed
            .iter()
            .enumerate()
            .map(|(i, v)| v + ((i as f32 * 0.71).cos() * 0.01))
            .collect();
        let l2p = perturbed.iter().map(|v| v * v).sum::<f32>().sqrt();
        for v in &mut perturbed {
            *v /= l2p;
        }

        // Unrelated: orthogonal direction.
        let unrelated: Vec<f32> = (0..512)
            .map(|i| ((i as f32) * 0.0789 + 1.5).cos())
            .collect();
        let l2u = unrelated.iter().map(|v| v * v).sum::<f32>().sqrt();
        let unrelated: Vec<f32> = unrelated.iter().map(|v| v / l2u).collect();

        // Sanity: similarity must be high between near-twins and low to
        // the unrelated one.  This proves the test inputs themselves
        // exercise the path we care about.
        let sim_close = cosine_similarity(&normed, &perturbed);
        let sim_far = cosine_similarity(&normed, &unrelated);
        assert!(
            sim_close > 0.95,
            "test inputs broken: near-twins similarity {sim_close} should be > 0.95"
        );
        assert!(
            sim_far < 0.5,
            "test inputs broken: unrelated similarity {sim_far} should be < 0.5"
        );

        let faces = vec![
            (101, normed.clone()),
            (102, perturbed.clone()),
            (103, unrelated.clone()),
        ];
        let assignments = cluster_faces(&faces, 0.6);

        // Map face_id → cluster_id.
        let cid: std::collections::HashMap<i64, i64> = assignments.iter().copied().collect();

        assert_eq!(
            cid[&101], cid[&102],
            "near-identical 512-d embeddings (cos sim {sim_close:.3}) must share a cluster, \
             clustering ignored the embedding vectors"
        );
        assert_ne!(
            cid[&101], cid[&103],
            "orthogonal 512-d embeddings (cos sim {sim_far:.3}) must NOT share a cluster, \
             clustering used a proxy instead of cosine similarity"
        );
    }

    /// Verifies that the threshold parameter is respected and that the
    /// cosine-similarity gate is the actual gate (not e.g. a constant).
    #[test]
    fn test_threshold_is_respected() {
        // Two vectors with cosine similarity ≈ 0.6.
        let a = vec![1.0_f32, 0.0, 0.0];
        let b = {
            // 60° apart from a in xy-plane: cos 60° = 0.5.  But to land at
            // 0.6 we use cos⁻¹(0.6) ≈ 53.13°.
            let theta: f32 = 0.6_f32.acos();
            vec![theta.cos(), theta.sin(), 0.0]
        };
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.6).abs() < 0.01, "test setup wrong: sim={sim}");

        // Strict threshold (0.7) must keep them apart.
        let strict = cluster_faces(&[(1, a.clone()), (2, b.clone())], 0.7);
        let strict_map: std::collections::HashMap<i64, i64> = strict.iter().copied().collect();
        assert_ne!(
            strict_map[&1], strict_map[&2],
            "threshold=0.7 should reject pairs with sim={sim:.3}"
        );

        // Lenient threshold (0.5) must merge them.
        let lenient = cluster_faces(&[(1, a), (2, b)], 0.5);
        let lenient_map: std::collections::HashMap<i64, i64> = lenient.iter().copied().collect();
        assert_eq!(
            lenient_map[&1], lenient_map[&2],
            "threshold=0.5 should merge pairs with sim={sim:.3}"
        );
    }
}
