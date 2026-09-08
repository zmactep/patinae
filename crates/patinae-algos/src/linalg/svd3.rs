//! Analytical 3×3 SVD decomposition
//!
//! Computes A = U · diag(S) · Vᵀ for a 3×3 matrix using the Jacobi
//! eigenvalue algorithm on AᵀA (symmetric positive semi-definite).
//!
//! All matrices use column-major layout: `m[col][row]`.

use lin_alg::f64::{Mat3 as Mat3d, Vec3 as Vec3d};

/// Result of 3×3 SVD decomposition: A = U · diag(S) · Vᵀ
#[derive(Debug, Clone)]
pub struct Svd3 {
    /// Left singular vectors (3×3 orthogonal matrix, column-major: `u[col][row]`)
    pub u: [[f32; 3]; 3],
    /// Singular values (sorted descending, non-negative)
    pub s: [f32; 3],
    /// Right singular vectors transposed (3×3 matrix, column-major: `vt[col][row]`)
    pub vt: [[f32; 3]; 3],
}

/// Compute SVD of a 3×3 matrix (column-major: `matrix[col][row]`)
pub fn svd3(matrix: &[[f32; 3]; 3]) -> Svd3 {
    // Work in f64 for numerical stability
    let a = mat_to_f64(matrix);

    // 1. Compute AᵀA (symmetric positive semi-definite)
    let ata = mat_mul_ata(&a);

    // 2. Jacobi eigendecomposition of AᵀA → eigenvalues and eigenvectors
    let (eigenvalues, eigvec_cols) = jacobi_eigen_3x3(&ata);

    // 3. Sort by descending eigenvalue, compute singular values
    let mut order = [0usize, 1, 2];
    if eigenvalues[order[0]] < eigenvalues[order[1]] {
        order.swap(0, 1);
    }
    if eigenvalues[order[0]] < eigenvalues[order[2]] {
        order.swap(0, 2);
    }
    if eigenvalues[order[1]] < eigenvalues[order[2]] {
        order.swap(1, 2);
    }

    let sigma = [
        eigenvalues[order[0]].max(0.0).sqrt(),
        eigenvalues[order[1]].max(0.0).sqrt(),
        eigenvalues[order[2]].max(0.0).sqrt(),
    ];
    let mut v_cols = [
        eigvec_cols[order[0]],
        eigvec_cols[order[1]],
        eigvec_cols[order[2]],
    ];

    // Ensure V is right-handed (det(V) = +1)
    let det_v = triple_product(&v_cols[0], &v_cols[1], &v_cols[2]);
    if det_v < 0.0 {
        v_cols[2] = (-Vec3d::from(v_cols[2])).to_arr();
    }

    // 4. U columns: u_i = A · v_i / sigma_i
    // A^T A squares the condition number. Roundoff in a zero eigenvalue
    // becomes a spurious singular value of order sqrt(epsilon) * sigma_max.
    let rank_tolerance = (sigma[0] * 1e-7).max(1e-10);
    let mut u_cols = [[0.0f64; 3]; 3];
    for i in 0..3 {
        if sigma[i] > rank_tolerance {
            let av = mat_vec_mul(&a, &v_cols[i]);
            u_cols[i] = (Vec3d::from(av) * (1.0 / sigma[i])).to_arr();
        }
    }

    // Handle degenerate cases
    if sigma[0] > rank_tolerance && sigma[1] > rank_tolerance && sigma[2] <= rank_tolerance {
        u_cols[2] = cross(&u_cols[0], &u_cols[1]);
        normalize(&mut u_cols[2]);
    } else if sigma[0] > rank_tolerance && sigma[1] <= rank_tolerance {
        u_cols[1] = arbitrary_perpendicular(&u_cols[0]);
        u_cols[2] = cross(&u_cols[0], &u_cols[1]);
        normalize(&mut u_cols[2]);
    } else if sigma[0] <= rank_tolerance {
        u_cols = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }

    // Ensure U is right-handed
    let det_u = triple_product(&u_cols[0], &u_cols[1], &u_cols[2]);
    if det_u < 0.0 {
        u_cols[2] = (-Vec3d::from(u_cols[2])).to_arr();
    }

    // 5. Build output in column-major format
    // U: u_out[col][row] = u_cols[col][row]
    let u = [
        [
            u_cols[0][0] as f32,
            u_cols[0][1] as f32,
            u_cols[0][2] as f32,
        ],
        [
            u_cols[1][0] as f32,
            u_cols[1][1] as f32,
            u_cols[1][2] as f32,
        ],
        [
            u_cols[2][0] as f32,
            u_cols[2][1] as f32,
            u_cols[2][2] as f32,
        ],
    ];

    // Vᵀ: vt[col][row] = (Vᵀ)_{row,col} = V_{col,row} = v_cols[row][col]
    let vt = [
        [
            v_cols[0][0] as f32,
            v_cols[1][0] as f32,
            v_cols[2][0] as f32,
        ],
        [
            v_cols[0][1] as f32,
            v_cols[1][1] as f32,
            v_cols[2][1] as f32,
        ],
        [
            v_cols[0][2] as f32,
            v_cols[1][2] as f32,
            v_cols[2][2] as f32,
        ],
    ];

    Svd3 {
        u,
        s: [sigma[0] as f32, sigma[1] as f32, sigma[2] as f32],
        vt,
    }
}

// ============================================================================
// Internal helpers (all in f64)
// ============================================================================

fn mat_to_f64(m: &[[f32; 3]; 3]) -> [[f64; 3]; 3] {
    [
        [m[0][0] as f64, m[0][1] as f64, m[0][2] as f64],
        [m[1][0] as f64, m[1][1] as f64, m[1][2] as f64],
        [m[2][0] as f64, m[2][1] as f64, m[2][2] as f64],
    ]
}

/// Compute AᵀA where A is column-major. Result is column-major symmetric.
fn mat_mul_ata(a: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let m = Mat3d::from(*a);
    let result = m.transpose() * m;
    [
        [result.data[0], result.data[1], result.data[2]],
        [result.data[3], result.data[4], result.data[5]],
        [result.data[6], result.data[7], result.data[8]],
    ]
}

/// Multiply column-major matrix A by vector v: result = A · v
fn mat_vec_mul(a: &[[f64; 3]; 3], v: &[f64; 3]) -> [f64; 3] {
    (Mat3d::from(*a) * Vec3d::from(*v)).to_arr()
}

fn cross(a: &[f64; 3], b: &[f64; 3]) -> [f64; 3] {
    Vec3d::from(*a).cross(Vec3d::from(*b)).to_arr()
}

fn triple_product(a: &[f64; 3], b: &[f64; 3], c: &[f64; 3]) -> f64 {
    Vec3d::from(*a).dot(Vec3d::from(*b).cross(Vec3d::from(*c)))
}

fn normalize(v: &mut [f64; 3]) {
    let vec = Vec3d::from(*v);
    let len = vec.magnitude();
    if len > 1e-15 {
        let n = vec * (1.0 / len);
        *v = n.to_arr();
    }
}

fn arbitrary_perpendicular(v: &[f64; 3]) -> [f64; 3] {
    Vec3d::from(*v).any_perpendicular().to_arr()
}

/// Jacobi eigenvalue algorithm for 3×3 symmetric matrices.
///
/// Returns (eigenvalues, eigenvector_columns).
/// Uses cyclic Jacobi rotations until convergence.
fn jacobi_eigen_3x3(m: &[[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    // Work with a flat symmetric representation for easier manipulation
    // Store as row-major 3×3 for Jacobi rotations
    let mut a = [
        [m[0][0], m[1][0], m[2][0]], // row 0
        [m[0][1], m[1][1], m[2][1]], // row 1
        [m[0][2], m[1][2], m[2][2]], // row 2
    ];

    // Eigenvector matrix (starts as identity, accumulates rotations)
    let mut v = [[1.0f64, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    // Cyclic Jacobi: sweep through (0,1), (0,2), (1,2) pairs
    for _ in 0..50 {
        // Check convergence: sum of squares of off-diagonal elements
        let off = a[0][1] * a[0][1] + a[0][2] * a[0][2] + a[1][2] * a[1][2];
        if off < 1e-30 {
            break;
        }

        // Apply Jacobi rotation to each off-diagonal pair
        for &(p, q) in &[(0usize, 1usize), (0, 2), (1, 2)] {
            if a[p][q].abs() < 1e-15 {
                continue;
            }
            jacobi_rotate(&mut a, &mut v, p, q);
        }
    }

    // Eigenvalues are the diagonal of a
    let eigenvalues = [a[0][0], a[1][1], a[2][2]];

    // Eigenvectors: column j of v is the j-th eigenvector
    // v is row-major: v[row][col], so column j = [v[0][j], v[1][j], v[2][j]]
    let eigvec_cols = [
        [v[0][0], v[1][0], v[2][0]],
        [v[0][1], v[1][1], v[2][1]],
        [v[0][2], v[1][2], v[2][2]],
    ];

    (eigenvalues, eigvec_cols)
}

/// Apply a single Jacobi rotation to eliminate a[p][q].
fn jacobi_rotate(a: &mut [[f64; 3]; 3], v: &mut [[f64; 3]; 3], p: usize, q: usize) {
    let app = a[p][p];
    let aqq = a[q][q];
    let apq = a[p][q];

    // Compute rotation angle
    let (c, s) = if (app - aqq).abs() < 1e-15 {
        // Special case: equal diagonal elements
        let inv_sqrt2 = 1.0 / 2.0f64.sqrt();
        (inv_sqrt2, if apq > 0.0 { inv_sqrt2 } else { -inv_sqrt2 })
    } else {
        let tau = (aqq - app) / (2.0 * apq);
        let t = if tau >= 0.0 {
            1.0 / (tau + (1.0 + tau * tau).sqrt())
        } else {
            -1.0 / (-tau + (1.0 + tau * tau).sqrt())
        };
        let c = 1.0 / (1.0 + t * t).sqrt();
        (c, t * c)
    };

    // Update matrix A: A' = GᵀAG where G is Givens rotation in (p,q) plane
    // Diagonal elements
    a[p][p] = c * c * app - 2.0 * s * c * apq + s * s * aqq;
    a[q][q] = s * s * app + 2.0 * s * c * apq + c * c * aqq;
    a[p][q] = 0.0;
    a[q][p] = 0.0;

    // Off-diagonal elements involving the third index
    let r = 3 - p - q; // the remaining index
    let arp = a[r][p];
    let arq = a[r][q];
    a[r][p] = c * arp - s * arq;
    a[p][r] = a[r][p];
    a[r][q] = s * arp + c * arq;
    a[q][r] = a[r][q];

    // Accumulate eigenvectors: V' = V · G
    for row in v.iter_mut().take(3) {
        let vip = row[p];
        let viq = row[q];
        row[p] = c * vip - s * viq;
        row[q] = s * vip + c * viq;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Column-major 3×3 matrix multiply: C = A · B
    fn mat_mul(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
        let mut c = [[0.0f32; 3]; 3];
        for col in 0..3 {
            for row in 0..3 {
                for k in 0..3 {
                    c[col][row] += a[k][row] * b[col][k];
                }
            }
        }
        c
    }

    fn diag_mat(s: &[f32; 3]) -> [[f32; 3]; 3] {
        [[s[0], 0.0, 0.0], [0.0, s[1], 0.0], [0.0, 0.0, s[2]]]
    }

    fn transpose(m: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
        [
            [m[0][0], m[1][0], m[2][0]],
            [m[0][1], m[1][1], m[2][1]],
            [m[0][2], m[1][2], m[2][2]],
        ]
    }

    fn assert_orthogonal(m: &[[f32; 3]; 3], label: &str) {
        // For column-major: MᵀM should be I (columns are orthonormal)
        let mt = transpose(m);
        let prod = mat_mul(&mt, m);
        for (i, prod_row) in prod.iter().enumerate() {
            for (j, &val) in prod_row.iter().enumerate() {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!(
                    (val - expected).abs() < 1e-4,
                    "{} not orthogonal: (MᵀM)[{}][{}] = {}, expected {}",
                    label,
                    i,
                    j,
                    val,
                    expected
                );
            }
        }
    }

    fn assert_reconstruction(a: &[[f32; 3]; 3], svd: &Svd3, tol: f32) {
        let s_mat = diag_mat(&svd.s);
        let us = mat_mul(&svd.u, &s_mat);
        let reconstructed = mat_mul(&us, &svd.vt);
        for col in 0..3 {
            for row in 0..3 {
                assert!(
                    (a[col][row] - reconstructed[col][row]).abs() < tol,
                    "Reconstruction A[{}][{}]: {} vs {}",
                    col,
                    row,
                    a[col][row],
                    reconstructed[col][row]
                );
            }
        }
    }

    #[test]
    fn test_identity() {
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let result = svd3(&identity);
        for &s in &result.s {
            assert!((s - 1.0).abs() < 1e-5, "Singular value {} != 1.0", s);
        }
        assert_orthogonal(&result.u, "U");
        assert_orthogonal(&result.vt, "Vt");
        assert_reconstruction(&identity, &result, 1e-5);
    }

    #[test]
    fn test_rotation_matrix() {
        let angle: f32 = std::f32::consts::FRAC_PI_2;
        let c = angle.cos();
        let s = angle.sin();
        let rot = [[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]];
        let result = svd3(&rot);
        for &sv in &result.s {
            assert!((sv - 1.0).abs() < 1e-4, "Singular value {} != 1.0", sv);
        }
        assert_orthogonal(&result.u, "U");
        assert_orthogonal(&result.vt, "Vt");
        assert_reconstruction(&rot, &result, 1e-4);
    }

    #[test]
    fn test_scaling_matrix() {
        let mat = [[3.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 1.0]];
        let result = svd3(&mat);
        assert!((result.s[0] - 3.0).abs() < 1e-4, "s[0]={}", result.s[0]);
        assert!((result.s[1] - 2.0).abs() < 1e-4, "s[1]={}", result.s[1]);
        assert!((result.s[2] - 1.0).abs() < 1e-4, "s[2]={}", result.s[2]);
        assert_orthogonal(&result.u, "U");
        assert_orthogonal(&result.vt, "Vt");
        assert_reconstruction(&mat, &result, 1e-4);
    }

    #[test]
    fn test_general_matrix() {
        let mat = [[1.0, 4.0, 7.0], [2.0, 5.0, 8.0], [3.0, 6.0, 9.0]];
        let result = svd3(&mat);
        assert!(result.s[0] >= result.s[1]);
        assert!(result.s[1] >= result.s[2]);
        for &s in &result.s {
            assert!(s >= 0.0);
        }
        assert_orthogonal(&result.u, "U");
        assert_orthogonal(&result.vt, "Vt");
        assert_reconstruction(&mat, &result, 1e-3);
    }

    #[test]
    fn test_zero_matrix() {
        let mat = [[0.0; 3]; 3];
        let result = svd3(&mat);
        for &s in &result.s {
            assert!(s.abs() < 1e-5);
        }
    }

    #[test]
    fn test_rank_1_matrix() {
        let mat = [[1.0, 2.0, 3.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]];
        let result = svd3(&mat);
        assert!(
            result.s[0] > 1e-3,
            "s[0] should be non-zero: {}",
            result.s[0]
        );
        assert!(result.s[1] < 1e-3, "s[1] should be ~zero: {}", result.s[1]);
        assert!(result.s[2] < 1e-3, "s[2] should be ~zero: {}", result.s[2]);
        assert_orthogonal(&result.u, "U");
        assert_orthogonal(&result.vt, "Vt");
        assert_reconstruction(&mat, &result, 1e-3);
    }

    #[test]
    fn test_symmetric_matrix() {
        let mat = [[2.0, 1.0, 0.0], [1.0, 3.0, 1.0], [0.0, 1.0, 2.0]];
        let result = svd3(&mat);
        assert!(result.s[0] >= result.s[1]);
        assert!(result.s[1] >= result.s[2]);
        assert_orthogonal(&result.u, "U");
        assert_orthogonal(&result.vt, "Vt");
        assert_reconstruction(&mat, &result, 1e-3);
    }
}
