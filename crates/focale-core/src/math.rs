//! Deterministic transcendental functions for the export path.
//!
//! `std`'s float transcendentals (`powf`, `exp`, `cbrt`, …) call the
//! platform maths library, whose results differ across libc versions and
//! operating systems — the pipeline regression golden caught exactly such a
//! divergence between glibc 2.39 and 2.42. Everything on the export path
//! therefore uses these wrappers over the pure-Rust [`libm`] crate: the same
//! bits on every machine, forever (PRD §2.1).
//!
//! Pure IEEE 754 operations (`+ - * /`, `sqrt`, `floor`, `abs`, `mul_add`
//! avoided) are correctly-rounded by the standard and stay on `std`.

/// `x^y` (f32), deterministic.
#[inline]
pub fn powf(x: f32, y: f32) -> f32 {
    libm::powf(x, y)
}

/// Cube root (f32), deterministic.
#[inline]
pub fn cbrt(x: f32) -> f32 {
    libm::cbrtf(x)
}

/// `e^x` (f32), deterministic.
#[inline]
pub fn exp(x: f32) -> f32 {
    libm::expf(x)
}

/// `2^x` (f32), deterministic.
#[inline]
pub fn exp2(x: f32) -> f32 {
    libm::exp2f(x)
}

/// Natural logarithm (f32), deterministic.
#[inline]
pub fn ln(x: f32) -> f32 {
    libm::logf(x)
}

/// Sine (f32), deterministic.
#[inline]
pub fn sin(x: f32) -> f32 {
    libm::sinf(x)
}

/// Cosine (f32), deterministic.
#[inline]
pub fn cos(x: f32) -> f32 {
    libm::cosf(x)
}

/// Four-quadrant arctangent `atan2(y, x)` (f32), deterministic.
#[inline]
pub fn atan2(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_std_closely() {
        // libm and std agree to ~1 ULP on ordinary values; this is a sanity
        // net, not a determinism proof (determinism is by construction).
        for x in [0.001_f32, 0.18, 0.5, 1.0, 2.2, 10.0] {
            assert!((powf(x, 2.4) - x.powf(2.4)).abs() <= x.powf(2.4) * 1e-6);
            assert!((exp2(x) - x.exp2()).abs() <= x.exp2() * 1e-6);
            assert!((cbrt(x) - x.cbrt()).abs() <= x.cbrt() * 1e-6);
        }
        assert_eq!(powf(0.0, 2.0), 0.0);
        assert_eq!(exp(0.0), 1.0);
    }
}
