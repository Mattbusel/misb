//! f64 helpers that `core` lacks without `std`.

pub(crate) fn floor(x: f64) -> f64 {
    if !x.is_finite() || x >= 4_503_599_627_370_496.0 || x <= -4_503_599_627_370_496.0 {
        return x;
    }
    let t = x as i64 as f64;
    if t > x {
        t - 1.0
    } else {
        t
    }
}

#[cfg_attr(not(feature = "alloc"), allow(dead_code))]
pub(crate) fn round(x: f64) -> f64 {
    if x >= 0.0 {
        floor(x + 0.5)
    } else {
        -floor(-x + 0.5)
    }
}

/// 2^k for k in -1022..=1023.
pub(crate) fn pow2(k: i32) -> f64 {
    f64::from_bits(((k + 1023) as u64) << 52)
}

/// ceil(log2(x)) for x > 0.
pub(crate) fn ceil_log2(x: f64) -> i32 {
    let mut k = 0i32;
    if x >= 1.0 {
        while pow2(k) < x {
            k += 1;
        }
    } else {
        while pow2(k - 1) >= x {
            k -= 1;
        }
    }
    k
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helpers() {
        assert_eq!(floor(-0.5), -1.0);
        assert_eq!(floor(2.0), 2.0);
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(-2.5), -3.0);
        assert_eq!(pow2(10), 1024.0);
        assert_eq!(pow2(-1), 0.5);
        assert_eq!(ceil_log2(180.0), 8);
        assert_eq!(ceil_log2(256.0), 8);
        assert_eq!(ceil_log2(19900.0), 15);
        assert_eq!(ceil_log2(0.3), -1);
    }
}
