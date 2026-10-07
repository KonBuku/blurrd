//! Rational time units and inclusive frame ranges.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A positive rational number such as a frame rate (`60/1`) or a stream
/// time base (`1/15360`). Written as `[num, den]` in JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(try_from = "[u32; 2]", into = "[u32; 2]")]
#[ts(as = "[u32; 2]")]
pub struct Rational {
    /// Numerator.
    pub num: u32,
    /// Denominator, never zero.
    pub den: u32,
}

/// The error returned for a rational with a zero denominator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("rational {num}/0 has a zero denominator")]
pub struct ZeroDenominatorError {
    /// Numerator of the rejected rational.
    pub num: u32,
}

impl Rational {
    /// Creates a rational.
    ///
    /// # Errors
    ///
    /// Returns [`ZeroDenominatorError`] when `den` is zero.
    pub const fn new(num: u32, den: u32) -> Result<Self, ZeroDenominatorError> {
        if den == 0 {
            Err(ZeroDenominatorError { num })
        } else {
            Ok(Self { num, den })
        }
    }

    /// The value as a floating-point number.
    pub fn as_f64(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }
}

impl TryFrom<[u32; 2]> for Rational {
    type Error = ZeroDenominatorError;

    fn try_from([num, den]: [u32; 2]) -> Result<Self, Self::Error> {
        Self::new(num, den)
    }
}

impl From<Rational> for [u32; 2] {
    fn from(r: Rational) -> Self {
        [r.num, r.den]
    }
}

/// An inclusive range of presentation frame indices, written as `[f0, f1]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(try_from = "[u32; 2]", into = "[u32; 2]")]
#[ts(as = "[u32; 2]")]
pub struct FrameRange {
    /// First frame.
    pub f0: u32,
    /// Last frame (inclusive), never before `f0`.
    pub f1: u32,
}

/// The error returned for a frame range that ends before it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("frame range {f0}..={f1} ends before it starts")]
pub struct ReversedRangeError {
    /// First frame of the rejected range.
    pub f0: u32,
    /// Last frame of the rejected range.
    pub f1: u32,
}

impl FrameRange {
    /// Creates a range.
    ///
    /// # Errors
    ///
    /// Returns [`ReversedRangeError`] when `f1 < f0`.
    pub const fn new(f0: u32, f1: u32) -> Result<Self, ReversedRangeError> {
        if f1 < f0 {
            Err(ReversedRangeError { f0, f1 })
        } else {
            Ok(Self { f0, f1 })
        }
    }

    /// Whether `frame` is inside the range.
    pub const fn contains(&self, frame: u32) -> bool {
        self.f0 <= frame && frame <= self.f1
    }
}

impl TryFrom<[u32; 2]> for FrameRange {
    type Error = ReversedRangeError;

    fn try_from([f0, f1]: [u32; 2]) -> Result<Self, Self::Error> {
        Self::new(f0, f1)
    }
}

impl From<FrameRange> for [u32; 2] {
    fn from(r: FrameRange) -> Self {
        [r.f0, r.f1]
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameRange, Rational};

    #[test]
    fn rational_rejects_zero_denominator() {
        assert!(Rational::new(60, 0).is_err());
        assert!(serde_json::from_str::<Rational>("[1,0]").is_err());
        let fps: Rational = serde_json::from_str("[60000,1001]").unwrap();
        assert!((fps.as_f64() - 59.94).abs() < 0.001);
        assert_eq!(serde_json::to_string(&fps).unwrap(), "[60000,1001]");
    }

    #[test]
    fn frame_range_is_inclusive() {
        let range = FrameRange::new(10, 12).unwrap();
        assert!(range.contains(10) && range.contains(12) && !range.contains(13));
        assert!(FrameRange::new(5, 4).is_err());
        assert!(serde_json::from_str::<FrameRange>("[7,3]").is_err());
    }
}
