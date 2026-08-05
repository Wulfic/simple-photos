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

    // Merge clusters greedily via union-find with a **centroid-linkage** guard.
    //
    // Pure single-linkage (merge whenever *any* cross-cluster pair clears the
    // threshold) chains: if A≈B and B≈C, A and C fuse even when A and C look
    // nothing alike. On a large library there are always enough borderline
    // faces (odd crops, similar lighting, glasses) to form a chain bridging two
    // genuinely different people into one giant cluster — the "combining people
    // who don't look alike" bug.
    //
    // The fix: before fusing two clusters, also require their running mean
    // embeddings (centroids) to agree at `threshold`. A single bridge face can
    // still join the cluster it truly belongs to, but it can no longer drag a
    // whole *other* identity along, because two distinct people's centroids stay
    // far apart. First-time single-vs-single merges are unaffected (a lone
    // cluster's centroid == its one face, so the centroid check == the pair
    // check). `cosine_similarity` is scale-invariant, so summed (un-normalised)
    // centroids compare identically to normalised ones.
    //
    // Union stays O(1) (point one root at the other); `find_root` + path
    // compression keep lookups near-constant, so the pass is still ~O(n²) on the
    // candidate pairs (item #16 performance work preserved).
    let mut centroid_sums: Vec<Vec<f32>> = faces.iter().map(|(_, emb)| emb.clone()).collect();
    let mut merges = 0usize;
    for (i, j, sim) in &similarities {
        if *sim < threshold {
            break;
        }
        let ci = find_root(&mut cluster_ids, *i);
        let cj = find_root(&mut cluster_ids, *j);
        if ci == cj {
            continue;
        }
        // Centroid-linkage guard: skip merges that would bridge two clusters
        // whose means have drifted apart (chaining across distinct identities).
        // Only compares equal-dimensionality centroids — mixed 512-d ArcFace vs
        // fallback vectors are already filtered out of `similarities`.
        if centroid_sums[ci].len() == centroid_sums[cj].len() {
            let centroid_sim = cosine_similarity(&centroid_sums[ci], &centroid_sums[cj]);
            if centroid_sim < threshold {
                continue;
            }
        }
        // Union: attach the higher-indexed root under the lower so cluster IDs
        // stay deterministic. Fold the source centroid sum into the target so
        // the running mean tracks every member.
        let target = ci.min(cj);
        let source = ci.max(cj);
        let source_sum = std::mem::take(&mut centroid_sums[source]);
        let target_sum = &mut centroid_sums[target];
        for k in 0..target_sum.len().min(source_sum.len()) {
            target_sum[k] += source_sum[k];
        }
        cluster_ids[source] = target;
        merges += 1;
    }

    // Flatten cluster IDs to contiguous values
    let mut cluster_map: std::collections::HashMap<usize, i64> = std::collections::HashMap::new();
    let mut next_id: i64 = 1;

    let result: Vec<ClusterAssignment> = faces
        .iter()
        .enumerate()
        .map(|(idx, (face_id, _))| {
            let root = find_root(&mut cluster_ids, idx);
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

/// Find the root cluster for an element using iterative union-find with path
/// compression. Compression flattens the chain so repeated lookups stay near
/// O(1) even after many single-linkage unions — essential now that merges are
/// O(1) unions rather than full relabels (item #16 clustering hotspot).
fn find_root(clusters: &mut [usize], idx: usize) -> usize {
    // First pass: walk to the root.
    let mut root = idx;
    while clusters[root] != root {
        root = clusters[root];
    }
    // Second pass: point every node on the path directly at the root.
    let mut cur = idx;
    while clusters[cur] != root {
        let next = clusters[cur];
        clusters[cur] = root;
        cur = next;
    }
    root
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

    /// Centroid-linkage must resist chaining. Neighbours A-B and B-C each clear
    /// the pair threshold, but once A and B form a cluster its centroid sits
    /// between them, so C (a full step further out) no longer agrees with that
    /// centroid and is NOT dragged in. This is the opposite of the old
    /// single-linkage behaviour and is exactly what stops distinct identities
    /// fusing through a bridge face on large libraries.
    #[test]
    fn test_centroid_linkage_resists_chaining() {
        let v = |deg: f32| {
            let t = deg.to_radians();
            vec![t.cos(), t.sin(), 0.0f32]
        };
        // 0°, 30°, 60°: cos(30°)=0.866 clears the 0.8 pair gate for both
        // adjacent pairs. Centroid of {0°,30°} ≈ 15°; 60° vs 15° = cos(45°)=0.707
        // < 0.8, so the C merge is correctly rejected.
        let faces = vec![
            (1, v(0.0)),
            (2, v(30.0)),
            (3, v(60.0)),
            (4, vec![0.0, 0.0, 1.0]),
        ];
        let assignments = cluster_faces(&faces, 0.8);
        let m: std::collections::HashMap<i64, i64> = assignments.iter().copied().collect();

        assert_eq!(m[&1], m[&2], "A and B (cos 0.866) must share a cluster");
        assert_ne!(
            m[&2], m[&3],
            "C must NOT chain into {{A,B}} — its centroid (15°) vs C (60°) is below threshold"
        );
        assert_ne!(
            m[&1], m[&4],
            "orthogonal vector must stay in its own cluster"
        );
    }

    /// Regression for the reported bug: on large libraries, distinct people were
    /// being merged. Two tight, well-separated clusters (person P near 0°, person
    /// Q near 45°) plus one borderline "bridge" face that is individually within
    /// threshold of a member of EACH cluster. Pure single-linkage fuses all of
    /// them into one identity; centroid-linkage must keep P and Q apart and only
    /// attach the bridge to the single closest cluster.
    #[test]
    fn test_bridge_face_does_not_fuse_distinct_identities() {
        let v = |deg: f32| {
            let t: f32 = deg.to_radians();
            vec![t.cos(), t.sin(), 0.0f32]
        };
        let faces = vec![
            // Person P — tightly grouped around 0°.
            (1, v(-5.0)),
            (2, v(0.0)),
            (3, v(5.0)),
            // Person Q — tightly grouped around 45°.
            (4, v(40.0)),
            (5, v(45.0)),
            (6, v(50.0)),
            // Bridge face at 22°: cos(22°)≈0.93 to P's 0°, cos(18°)≈0.95 to Q's
            // 40° — over threshold to a member of BOTH clusters.
            (7, v(22.0)),
        ];
        let assignments = cluster_faces(&faces, 0.8);
        let m: std::collections::HashMap<i64, i64> = assignments.iter().copied().collect();

        // P stays one identity, Q stays one identity, and the two are distinct.
        assert_eq!(m[&1], m[&2], "P faces must cluster together");
        assert_eq!(m[&2], m[&3], "P faces must cluster together");
        assert_eq!(m[&4], m[&5], "Q faces must cluster together");
        assert_eq!(m[&5], m[&6], "Q faces must cluster together");
        assert_ne!(
            m[&2], m[&5],
            "distinct people P and Q must NOT be fused by the bridge face"
        );
    }
}
