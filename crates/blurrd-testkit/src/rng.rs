//! A small deterministic random number generator.

/// SplitMix64: fast, deterministic and good enough for test content.
///
/// Every fixture is generated from a seed, so a failing test can be
/// reproduced exactly from the seed it reports.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// A generator starting from `seed`.
    pub const fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next 64 random bits.
    pub const fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..n` (0 if `n` is 0).
    pub const fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        (self.next_u64() % n as u64) as u32
    }

    /// A value in `lo..=hi`.
    pub const fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        let span = hi.abs_diff(lo) + 1;
        lo.wrapping_add(self.below(span) as i32)
    }

    /// A value in `0.0..1.0`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// `true` with probability `p`.
    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }

    /// A random element of `items`.
    ///
    /// # Panics
    ///
    /// Panics if `items` is empty.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u32) as usize]
    }
}

/// A well-mixed 32-bit hash of two coordinates and a seed, for textures
/// that must look the same whichever order pixels are drawn in.
pub(crate) const fn hash2(x: i32, y: i32, seed: u64) -> u32 {
    let mut rng = Rng::new(
        seed ^ ((x as u32 as u64) << 32 | y as u32 as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93),
    );
    (rng.next_u64() >> 32) as u32
}

#[cfg(test)]
mod tests {
    use super::{Rng, hash2};

    #[test]
    fn is_deterministic() {
        let a: Vec<u64> = {
            let mut r = Rng::new(42);
            (0..4).map(|_| r.next_u64()).collect()
        };
        let b: Vec<u64> = {
            let mut r = Rng::new(42);
            (0..4).map(|_| r.next_u64()).collect()
        };
        assert_eq!(a, b);
        assert_ne!(a[0], a[1]);
    }

    #[test]
    fn stays_in_range() {
        let mut r = Rng::new(1);
        for _ in 0..1000 {
            let v = r.range(-3, 5);
            assert!((-3..=5).contains(&v));
            let u = r.unit();
            assert!((0.0..1.0).contains(&u));
        }
        assert_eq!(r.below(0), 0);
        assert_eq!(r.range(4, 4), 4);
    }

    #[test]
    fn hashes_coordinates_independently() {
        assert_eq!(hash2(3, 4, 9), hash2(3, 4, 9));
        assert_ne!(hash2(3, 4, 9), hash2(4, 3, 9));
        assert_ne!(hash2(3, 4, 9), hash2(3, 4, 10));
    }
}
