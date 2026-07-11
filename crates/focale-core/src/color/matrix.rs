//! 3×3 matrix maths for colour transforms.
//!
//! Pixel-path arithmetic is `f32` with a fixed, explicit expression order.
//! Rust never enables fast-math or FMA contraction, so every operation is
//! rounded individually and results are bit-identical across platforms.
//! Inversion runs in `f64` internally (fixed cofactor-expansion order) for
//! accuracy; the rounding back to `f32` is equally deterministic.

/// A row-major 3×3 `f32` matrix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat3(pub [[f32; 3]; 3]);

impl Mat3 {
    /// The identity matrix.
    pub const IDENTITY: Mat3 = Mat3([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    /// Matrix–vector product `self × v`.
    #[must_use]
    pub fn mul_vec(self, v: [f32; 3]) -> [f32; 3] {
        let m = self.0;
        [
            m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
            m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
            m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
        ]
    }

    /// Inverse, or `None` if the determinant is exactly zero.
    ///
    /// Computed in `f64` via cofactor expansion in a fixed order, then
    /// rounded back to `f32`; deterministic across platforms.
    #[must_use]
    pub fn invert(self) -> Option<Mat3> {
        let m = self.0;
        let m64 = [
            [f64::from(m[0][0]), f64::from(m[0][1]), f64::from(m[0][2])],
            [f64::from(m[1][0]), f64::from(m[1][1]), f64::from(m[1][2])],
            [f64::from(m[2][0]), f64::from(m[2][1]), f64::from(m[2][2])],
        ];
        invert3_f64(&m64).map(|inv| mat3_from_f64(&inv))
    }
}

impl std::ops::Mul for Mat3 {
    type Output = Mat3;

    /// Matrix product `self × rhs`.
    ///
    /// Each entry is the fixed expression `a·b + a·b + a·b`, rounded per
    /// operation.
    fn mul(self, rhs: Mat3) -> Mat3 {
        let a = self.0;
        let b = rhs.0;
        let mut out = [[0.0_f32; 3]; 3];
        for (i, row) in out.iter_mut().enumerate() {
            for (j, entry) in row.iter_mut().enumerate() {
                *entry = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
            }
        }
        Mat3(out)
    }
}

/// A row-major 3×3 `f64` matrix for derivation-grade maths (chromatic
/// adaptation, matrix inversion, test re-derivations).
pub(crate) type Mat3F64 = [[f64; 3]; 3];

/// `f64` matrix product `a × b` with fixed expression order.
pub(crate) fn mul3_f64(a: &Mat3F64, b: &Mat3F64) -> Mat3F64 {
    let mut out = [[0.0_f64; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, entry) in row.iter_mut().enumerate() {
            *entry = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

/// `f64` matrix–vector product `m × v` with fixed expression order.
pub(crate) fn mul_vec3_f64(m: &Mat3F64, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// `f64` inverse via cofactor expansion (fixed order), or `None` if the
/// determinant is exactly zero.
pub(crate) fn invert3_f64(m: &Mat3F64) -> Option<Mat3F64> {
    let c00 = m[1][1] * m[2][2] - m[1][2] * m[2][1];
    let c01 = m[1][2] * m[2][0] - m[1][0] * m[2][2];
    let c02 = m[1][0] * m[2][1] - m[1][1] * m[2][0];
    let det = m[0][0] * c00 + m[0][1] * c01 + m[0][2] * c02;
    if det == 0.0 {
        return None;
    }
    let inv_det = 1.0 / det;
    Some([
        [
            c00 * inv_det,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inv_det,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv_det,
        ],
        [
            c01 * inv_det,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv_det,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * inv_det,
        ],
        [
            c02 * inv_det,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * inv_det,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inv_det,
        ],
    ])
}

/// Rounds an `f64` matrix to an `f32` [`Mat3`].
pub(crate) fn mat3_from_f64(m: &Mat3F64) -> Mat3 {
    let mut out = [[0.0_f32; 3]; 3];
    for (row, src) in out.iter_mut().zip(m.iter()) {
        for (entry, v) in row.iter_mut().zip(src.iter()) {
            *entry = *v as f32;
        }
    }
    Mat3(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::testutil::{assert_mat_close, assert_vec_close};

    #[test]
    fn identity_is_neutral() {
        let m = Mat3([[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 10.0]]);
        assert_eq!(m * Mat3::IDENTITY, m);
        assert_eq!(Mat3::IDENTITY * m, m);
        assert_eq!(Mat3::IDENTITY.mul_vec([1.5, -2.0, 0.25]), [1.5, -2.0, 0.25]);
    }

    #[test]
    fn mul_vec_known_product() {
        let m = Mat3([[1.0, 0.0, 2.0], [0.0, 3.0, 0.0], [-1.0, 0.0, 1.0]]);
        assert_vec_close(m.mul_vec([2.0, 1.0, 4.0]), [10.0, 3.0, 2.0], 0.0);
    }

    #[test]
    fn invert_round_trips_to_identity() {
        let m = Mat3([[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 10.0]]);
        let inv = m.invert().expect("matrix is invertible");
        assert_mat_close(m * inv, Mat3::IDENTITY, 1e-6);
        assert_mat_close(inv * m, Mat3::IDENTITY, 1e-6);
    }

    #[test]
    fn invert_singular_returns_none() {
        let m = Mat3([[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [7.0, 8.0, 10.0]]);
        assert_eq!(m.invert(), None);
    }

    #[test]
    fn invert_f64_round_trips_to_identity() {
        let m: Mat3F64 = [[0.5, 0.1, -0.2], [0.0, 1.5, 0.3], [-0.4, 0.2, 2.0]];
        let inv = invert3_f64(&m).expect("matrix is invertible");
        let id = mul3_f64(&m, &inv);
        for (i, row) in id.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!((v - expected).abs() < 1e-12);
            }
        }
    }
}
