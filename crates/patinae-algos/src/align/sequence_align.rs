//! Needleman-Wunsch global sequence alignment with affine gap penalties
//!
//! Standard dynamic programming alignment for matching residues between
//! structures prior to structural superposition.

use super::substitution_matrix::{SubstitutionMatrix, BLOSUM62};

/// Scoring parameters for sequence alignment
#[derive(Debug, Clone)]
pub struct AlignmentScoring {
    /// Substitution matrix for residue pair scores
    pub matrix: &'static SubstitutionMatrix,
    /// Gap opening penalty (default: -10.0 for BLOSUM62)
    pub gap_open: f32,
    /// Gap extension penalty (default: -1.0)
    pub gap_extend: f32,
}

impl Default for AlignmentScoring {
    fn default() -> Self {
        Self {
            matrix: &BLOSUM62,
            gap_open: -10.0,
            gap_extend: -1.0,
        }
    }
}

/// A pair of aligned positions
#[derive(Debug, Clone, Copy)]
pub enum AlignedPair {
    /// Both sequences have a residue at this position
    Match { source: usize, target: usize },
    /// Gap in source (insertion in target)
    GapSource { target: usize },
    /// Gap in target (insertion in source)
    GapTarget { source: usize },
}

/// Result of sequence alignment
#[derive(Debug, Clone)]
pub struct AlignmentResult {
    /// Aligned pairs
    pub pairs: Vec<AlignedPair>,
    /// Alignment score
    pub score: f32,
    /// Number of matched positions (non-gap)
    pub n_matched: usize,
    /// Sequence identity (identical_matches / aligned_length)
    pub identity: f32,
}

/// Perform global sequence alignment (Needleman-Wunsch with affine gaps)
///
/// Sequences are slices of single-char residue codes.
pub fn global_align(
    source: &[char],
    target: &[char],
    scoring: &AlignmentScoring,
) -> AlignmentResult {
    let m = source.len();
    let n = target.len();

    if m == 0 && n == 0 {
        return AlignmentResult {
            pairs: Vec::new(),
            score: 0.0,
            n_matched: 0,
            identity: 0.0,
        };
    }

    // Three DP matrices for affine gaps:
    // M[i][j] = best score ending with a match/mismatch at (i,j)
    // X[i][j] = best score ending with a gap in target (source consuming)
    // Y[i][j] = best score ending with a gap in source (target consuming)
    let inf = f32::NEG_INFINITY;
    let rows = m + 1;
    let cols = n + 1;

    let mut dp_m = vec![inf; rows * cols]; // match/mismatch
    let mut dp_x = vec![inf; rows * cols]; // gap in target
    let mut dp_y = vec![inf; rows * cols]; // gap in source

    // Traceback directions
    #[derive(Clone, Copy, PartialEq)]
    enum Trace {
        None,
        Diag, // from M
        Left, // from Y
        Up,   // from X
    }
    let mut tb_m = vec![Trace::None; rows * cols];
    let mut tb_x = vec![Trace::None; rows * cols];
    let mut tb_y = vec![Trace::None; rows * cols];

    let idx = |i: usize, j: usize| -> usize { i * cols + j };

    // Initialization
    dp_m[idx(0, 0)] = 0.0;
    for i in 1..=m {
        dp_x[idx(i, 0)] = scoring.gap_open + (i as f32) * scoring.gap_extend;
        tb_x[idx(i, 0)] = Trace::Up;
    }
    for j in 1..=n {
        dp_y[idx(0, j)] = scoring.gap_open + (j as f32) * scoring.gap_extend;
        tb_y[idx(0, j)] = Trace::Left;
    }

    // Fill
    for i in 1..=m {
        for j in 1..=n {
            let sub = scoring.matrix.score(source[i - 1], target[j - 1]);

            // M[i][j]: match/mismatch — come from any matrix at (i-1, j-1)
            let from_m = dp_m[idx(i - 1, j - 1)] + sub;
            let from_x = dp_x[idx(i - 1, j - 1)] + sub;
            let from_y = dp_y[idx(i - 1, j - 1)] + sub;
            if from_m >= from_x && from_m >= from_y {
                dp_m[idx(i, j)] = from_m;
                tb_m[idx(i, j)] = Trace::Diag;
            } else if from_x >= from_y {
                dp_m[idx(i, j)] = from_x;
                tb_m[idx(i, j)] = Trace::Up;
            } else {
                dp_m[idx(i, j)] = from_y;
                tb_m[idx(i, j)] = Trace::Left;
            }

            // X[i][j]: gap in target (consuming source i) — come from (i-1, j)
            let open_from_m = dp_m[idx(i - 1, j)] + scoring.gap_open + scoring.gap_extend;
            let extend_from_x = dp_x[idx(i - 1, j)] + scoring.gap_extend;
            if open_from_m >= extend_from_x {
                dp_x[idx(i, j)] = open_from_m;
                tb_x[idx(i, j)] = Trace::Diag; // opened from M
            } else {
                dp_x[idx(i, j)] = extend_from_x;
                tb_x[idx(i, j)] = Trace::Up; // extended from X
            }

            // Y[i][j]: gap in source (consuming target j) — come from (i, j-1)
            let open_from_m = dp_m[idx(i, j - 1)] + scoring.gap_open + scoring.gap_extend;
            let extend_from_y = dp_y[idx(i, j - 1)] + scoring.gap_extend;
            if open_from_m >= extend_from_y {
                dp_y[idx(i, j)] = open_from_m;
                tb_y[idx(i, j)] = Trace::Diag; // opened from M
            } else {
                dp_y[idx(i, j)] = extend_from_y;
                tb_y[idx(i, j)] = Trace::Left; // extended from Y
            }
        }
    }

    // Find best terminal score
    let score_m = dp_m[idx(m, n)];
    let score_x = dp_x[idx(m, n)];
    let score_y = dp_y[idx(m, n)];
    let score;

    #[derive(Clone, Copy, PartialEq)]
    enum Matrix {
        M,
        X,
        Y,
    }
    let mut current_matrix;

    if score_m >= score_x && score_m >= score_y {
        score = score_m;
        current_matrix = Matrix::M;
    } else if score_x >= score_y {
        score = score_x;
        current_matrix = Matrix::X;
    } else {
        score = score_y;
        current_matrix = Matrix::Y;
    }

    // Traceback
    let mut pairs = Vec::new();
    let mut i = m;
    let mut j = n;

    while i > 0 || j > 0 {
        match current_matrix {
            Matrix::M => {
                if i == 0 || j == 0 {
                    break;
                }
                let tb = tb_m[idx(i, j)];
                pairs.push(AlignedPair::Match {
                    source: i - 1,
                    target: j - 1,
                });
                current_matrix = match tb {
                    Trace::Diag => Matrix::M,
                    Trace::Up => Matrix::X,
                    Trace::Left => Matrix::Y,
                    Trace::None => break,
                };
                i -= 1;
                j -= 1;
            }
            Matrix::X => {
                if i == 0 {
                    break;
                }
                let tb = tb_x[idx(i, j)];
                pairs.push(AlignedPair::GapTarget { source: i - 1 });
                current_matrix = match tb {
                    Trace::Diag => Matrix::M,
                    Trace::Up => Matrix::X,
                    _ => break,
                };
                i -= 1;
            }
            Matrix::Y => {
                if j == 0 {
                    break;
                }
                let tb = tb_y[idx(i, j)];
                pairs.push(AlignedPair::GapSource { target: j - 1 });
                current_matrix = match tb {
                    Trace::Diag => Matrix::M,
                    Trace::Left => Matrix::Y,
                    _ => break,
                };
                j -= 1;
            }
        }
    }

    // Handle remaining unaligned prefix
    while i > 0 {
        pairs.push(AlignedPair::GapTarget { source: i - 1 });
        i -= 1;
    }
    while j > 0 {
        pairs.push(AlignedPair::GapSource { target: j - 1 });
        j -= 1;
    }

    pairs.reverse();

    // Compute statistics
    let mut n_matched = 0;
    let mut n_identical = 0;
    for pair in &pairs {
        if let AlignedPair::Match {
            source: si,
            target: ti,
        } = pair
        {
            n_matched += 1;
            if source[*si] == target[*ti] {
                n_identical += 1;
            }
        }
    }
    let aligned_length = pairs.len();
    let identity = if aligned_length > 0 {
        n_identical as f32 / aligned_length as f32
    } else {
        0.0
    };

    AlignmentResult {
        pairs,
        score,
        n_matched,
        identity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::align::substitution_matrix::IDENTITY;

    /// Scoring that mimics the old match=2/mismatch=-1 behavior
    fn identity_scoring() -> AlignmentScoring {
        AlignmentScoring {
            matrix: &IDENTITY,
            gap_open: -5.0,
            gap_extend: -0.5,
        }
    }

    #[test]
    fn test_identical_sequences() {
        let seq: Vec<char> = "ACDEFG".chars().collect();
        let result = global_align(&seq, &seq, &identity_scoring());
        assert_eq!(result.n_matched, 6);
        assert!((result.identity - 1.0).abs() < 1e-5);
        assert_eq!(result.pairs.len(), seq.len());
        for (i, pair) in result.pairs.iter().enumerate() {
            assert!(
                matches!(pair, AlignedPair::Match { source, target } if *source == i && *target == i),
                "pair {i}: {pair:?}"
            );
        }
    }

    #[test]
    fn test_single_insertion() {
        let source: Vec<char> = "ACDEFG".chars().collect();
        let target: Vec<char> = "ACDMEFG".chars().collect();
        let result = global_align(&source, &target, &identity_scoring());

        // Should have 6 matches and 1 gap
        assert_eq!(result.n_matched, 6);

        // Count gaps
        let gaps: usize = result
            .pairs
            .iter()
            .filter(|p| {
                matches!(
                    p,
                    AlignedPair::GapTarget { .. } | AlignedPair::GapSource { .. }
                )
            })
            .count();
        assert_eq!(gaps, 1);
    }

    #[test]
    fn test_single_deletion() {
        let source: Vec<char> = "ACDMEFG".chars().collect();
        let target: Vec<char> = "ACDEFG".chars().collect();
        let result = global_align(&source, &target, &identity_scoring());
        assert_eq!(result.n_matched, 6);
    }

    #[test]
    fn test_completely_different() {
        let source: Vec<char> = "AAAA".chars().collect();
        let target: Vec<char> = "LLLL".chars().collect();
        let result = global_align(&source, &target, &identity_scoring());
        assert_eq!(result.n_matched, 4); // All are mismatches, but still aligned
        assert!((result.identity - 0.0).abs() < 1e-5); // No identical residues
    }

    #[test]
    fn test_empty_sequences() {
        let result = global_align(&[], &[], &identity_scoring());
        assert_eq!(result.n_matched, 0);
        assert_eq!(result.pairs.len(), 0);
    }

    #[test]
    fn test_one_empty() {
        let source: Vec<char> = "ACE".chars().collect();
        let result = global_align(&source, &[], &identity_scoring());
        assert_eq!(result.n_matched, 0);
        assert_eq!(result.pairs.len(), 3); // All gap-in-target
    }

    #[test]
    fn test_affine_gap_preference() {
        // Affine gaps: one long gap should be preferred over multiple short gaps
        let source: Vec<char> = "ACDEFGH".chars().collect();
        let target: Vec<char> = "ACFGH".chars().collect(); // DE deleted
        let result = global_align(&source, &target, &identity_scoring());

        // Should have 5 matches (A,C,F,G,H) and 2 gaps (D,E)
        assert_eq!(result.n_matched, 5);
        assert_eq!(result.pairs.len(), 7);
        for (i, pair) in result.pairs.iter().enumerate() {
            let valid = match (i, pair) {
                (0 | 1, AlignedPair::Match { source, target }) => *source == i && *target == i,
                (2 | 3, AlignedPair::GapTarget { source }) => *source == i,
                (4..=6, AlignedPair::Match { source, target }) => *source == i && *target == i - 2,
                _ => false,
            };
            assert!(valid, "traceback pair {i}: {pair:?}");
        }
    }

    #[test]
    fn test_blosum62_default() {
        // With BLOSUM62, similar amino acids should score better
        let source: Vec<char> = "ACDEFGHIKLMNPQRSTVWY".chars().collect();
        let result = global_align(&source, &source, &AlignmentScoring::default());
        assert_eq!(result.n_matched, 20);
        assert!((result.identity - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_blosum62_similar_residues() {
        // D and E are similar (score 2 in BLOSUM62), should prefer aligning them
        // over opening a gap
        let source: Vec<char> = "AADE".chars().collect();
        let target: Vec<char> = "AAED".chars().collect();
        let result = global_align(&source, &target, &AlignmentScoring::default());
        // All 4 should be matched (no gaps), since D-E scores 2 (positive)
        assert_eq!(result.n_matched, 4);
    }
}
