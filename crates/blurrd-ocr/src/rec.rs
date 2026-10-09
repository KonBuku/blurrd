//! The fixed input shapes of text recognition.
//!
//! Crops are scaled to the model's input height of [`REC_HEIGHT`] pixels
//! and placed into one of the fixed [`WIDTH_BUCKETS`], so the recognition
//! sessions are created once per bucket and never meet a new shape.

/// Input height of the recognition model.
pub const REC_HEIGHT: u32 = 48;
/// The fixed input widths. Every width is compiled once at warm-up, so
/// recognition never meets a new shape (architecture plan 5.5).
pub const WIDTH_BUCKETS: [u32; 3] = [160, 320, 640];
