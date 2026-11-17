//! MISB ST 1201 IMAPB: maps a floating point range onto an unsigned integer
//! of a chosen byte length. ST 0601 tags 96 and up and most ST 0903 values
//! use it.
//!
//! With `bPow = ceil(log2(max - min))`, `dPow = 8 * len - 1`,
//! `sF = 2^(dPow - bPow)` and a zero offset when the range straddles zero,
//! a value `x` is stored as `floor(sF * (x - min) + zOffset)`. Integers
//! whose two top bits are set are reserved for special values (infinity,
//! NaN, "below minimum"); [`decode`] returns `None` for them and for
//! integers above the mapped maximum.

use crate::math::{ceil_log2, floor, pow2};

/// The parameters of one IMAPB mapping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Imapb {
    /// Lowest value of the range.
    pub min: f64,
    /// Highest value of the range.
    pub max: f64,
}

impl Imapb {
    /// A mapping over `min..=max`.
    pub const fn new(min: f64, max: f64) -> Self {
        Imapb { min, max }
    }

    fn scales(&self, len: usize) -> Option<(f64, f64, f64)> {
        if !(1..=8).contains(&len)
            || self.max.partial_cmp(&self.min) != Some(core::cmp::Ordering::Greater)
            || !self.min.is_finite()
            || !self.max.is_finite()
        {
            return None;
        }
        let b_pow = ceil_log2(self.max - self.min);
        let d_pow = 8 * len as i32 - 1;
        let sf = pow2(d_pow - b_pow);
        let sr = pow2(b_pow - d_pow);
        let z = if self.min < 0.0 && self.max > 0.0 {
            let s = sf * self.min;
            s - floor(s)
        } else {
            0.0
        };
        Some((sf, sr, z))
    }

    /// Decodes `bytes` (1 to 8 bytes, big-endian). `None` for special values,
    /// out of range integers and bad lengths.
    pub fn decode(&self, bytes: &[u8]) -> Option<f64> {
        let (_, sr, z) = self.scales(bytes.len())?;
        let y = crate::klv::be_uint(bytes)?;
        let bits = 8 * bytes.len() as u32;
        if (y >> (bits - 2)) & 0b11 == 0b11 {
            return None;
        }
        let x = sr * (y as f64 - z) + self.min;
        // allow the rounding slack of one step above the maximum
        if x > self.max + sr {
            return None;
        }
        Some(x)
    }

    /// Encodes `x` into `out` (whose length is the IMAPB length).
    /// Returns `false` if `x` is outside `min..=max` or not finite.
    pub fn encode(&self, x: f64, out: &mut [u8]) -> bool {
        let Some((sf, _, z)) = self.scales(out.len()) else {
            return false;
        };
        if !x.is_finite() || x < self.min || x > self.max {
            return false;
        }
        let y = floor(sf * (x - self.min) + z) as u64;
        let bytes = y.to_be_bytes();
        out.copy_from_slice(&bytes[8 - out.len()..]);
        true
    }

    /// Size of one step at `len` bytes: the resolution of the mapping.
    pub fn resolution(&self, len: usize) -> Option<f64> {
        self.scales(len).map(|(_, sr, _)| sr)
    }
}

/// Decodes with the mapping `min..=max`; see [`Imapb::decode`].
pub fn decode(min: f64, max: f64, bytes: &[u8]) -> Option<f64> {
    Imapb::new(min, max).decode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ST 0903 VMTI horizontal field of view examples (IMAPB 0..180, 2 bytes).
    #[test]
    fn st0903_fov_examples() {
        let m = Imapb::new(0.0, 180.0);
        let mut b = [0u8; 2];
        assert!(m.encode(10.0, &mut b));
        assert_eq!(b, [0x05, 0x00]);
        assert!(m.encode(90.0, &mut b));
        assert_eq!(b, [0x2D, 0x00]);
        assert_eq!(m.decode(&[0x2D, 0x00]), Some(90.0));
        assert!(m.encode(12.5, &mut b));
        assert!((m.decode(&b).unwrap() - 12.5).abs() < 1e-2);
    }

    #[test]
    fn straddling_range_round_trips() {
        let m = Imapb::new(-900.0, 19000.0);
        let mut b = [0u8; 3];
        for x in [-900.0, -1.5, 0.0, 10.0, 1234.567, 18999.9] {
            assert!(m.encode(x, &mut b));
            let back = m.decode(&b).unwrap();
            assert!(
                (back - x).abs() <= m.resolution(3).unwrap(),
                "{x} -> {back}"
            );
        }
        assert!(!m.encode(19000.5, &mut b));
        assert!(!m.encode(f64::NAN, &mut b));
    }

    #[test]
    fn specials_decode_to_none() {
        let m = Imapb::new(0.0, 180.0);
        assert_eq!(m.decode(&[0xC8, 0x00]), None); // +infinity
        assert_eq!(m.decode(&[0xE8, 0x00]), None); // -infinity
        assert_eq!(m.decode(&[0x7F, 0xFF]), None); // above max
        assert_eq!(m.decode(&[]), None);
        assert_eq!(m.decode(&[0; 9]), None);
    }
}
