//! Text recognition: PP-OCR pre-processing and CTC decoding.
//!
//! Crops are scaled to the model's input height of [`REC_HEIGHT`] pixels
//! with their natural aspect ratio (the pixel aspect of the view undone)
//! and placed into the smallest of the fixed
//! [`WIDTH_BUCKETS`] that holds them; wider crops are squeezed into the
//! widest bucket. The model predicts one character class per 8 input
//! columns; greedy CTC decoding takes the best class per step, merges
//! repeats and drops the blank class 0.

use crate::error::OcrError;
use crate::image::RgbView;
use crate::resample::{Normalise, Resampler};

/// Input height of the recognition model.
pub const REC_HEIGHT: u32 = 48;
/// The fixed input widths. Every width is compiled once at warm-up, so
/// recognition never meets a new shape (architecture plan 5.5).
pub const WIDTH_BUCKETS: [u32; 3] = [160, 320, 640];
/// Input columns per output step of the recognition model.
const COLUMNS_PER_STEP: u32 = 8;

/// One recognised text line.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Recognition {
    /// The text. It stays in memory; blurrd never logs or stores it.
    pub text: String,
    /// Mean probability of the decoded characters, `0.0..=1.0` (0 when no
    /// character was read).
    pub conf: f32,
    /// Where each decoded character is, in text order.
    pub chars: Vec<CharSpan>,
}

/// One decoded character of a [`Recognition`]: where it is in the text and
/// in the crop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharSpan {
    /// Byte offset of the character in [`Recognition::text`].
    pub start: usize,
    /// Byte offset just after it.
    pub end: usize,
    /// Left edge of the output steps that read it, as a fraction of the
    /// crop's width (`0.0..=1.0`).
    pub x0: f32,
    /// Right edge of those steps, as a fraction of the crop's width.
    pub x1: f32,
    /// Its probability.
    pub prob: f32,
}

impl Recognition {
    /// The horizontal extent of the characters in the byte range
    /// `start..end` of the text, as fractions of the crop's width, or
    /// `None` when no character starts inside the range.
    pub fn extent(&self, start: usize, end: usize) -> Option<(f32, f32)> {
        self.chars
            .iter()
            .filter(|c| c.start >= start && c.start < end)
            .fold(None, |acc, c| match acc {
                None => Some((c.x0, c.x1)),
                Some((x0, x1)) => Some((f32::min(x0, c.x0), f32::max(x1, c.x1))),
            })
    }

    /// The mean probability of the characters in the byte range
    /// `start..end` of the text (0 when none starts inside it).
    pub fn conf_of(&self, start: usize, end: usize) -> f32 {
        let (sum, count) = self
            .chars
            .iter()
            .filter(|c| c.start >= start && c.start < end)
            .fold((0.0f32, 0u32), |(s, n), c| (s + c.prob, n + 1));
        if count == 0 { 0.0 } else { sum / count as f32 }
    }
}

/// The input width a crop scales to at [`REC_HEIGHT`] in its natural
/// proportions, before bucketing.
pub(crate) fn scaled_width(image: RgbView<'_>) -> u32 {
    ((image.natural_width() * REC_HEIGHT as f32 / image.height() as f32).ceil() as u32).max(1)
}

/// Index into [`WIDTH_BUCKETS`] of the bucket for a scaled width.
pub(crate) fn bucket_for(scaled: u32) -> usize {
    WIDTH_BUCKETS
        .iter()
        .position(|&b| scaled <= b)
        .unwrap_or(WIDTH_BUCKETS.len() - 1)
}

/// Writes one crop as CHW into `out` (`3 × REC_HEIGHT × bucket` values)
/// with `resampler`: scaled to the model height, normalised to `-1..=1`
/// in blue, green, red order, padded with 0 on the right. Returns the
/// number of input columns the crop covers.
pub(crate) fn preprocess(
    resampler: &mut Resampler,
    image: RgbView<'_>,
    bucket: u32,
    out: &mut [f32],
) -> u32 {
    let width = scaled_width(image).min(bucket);
    out.fill(0.0);
    let norm = Normalise {
        scale: [1.0 / 127.5; 3],
        offset: [-1.0; 3],
    };
    let plane = (REC_HEIGHT * bucket) as usize;
    resampler.resize_into_planes(image, width, REC_HEIGHT, out, bucket as usize, plane, norm);
    width
}

/// Greedy CTC decoding of one item: `best` and `probs` hold the best
/// class of every output step and its probability (the model takes the
/// best class on the device, see [`crate::graph`]); only the first
/// `used_steps` steps (those covering the crop rather than padding) are
/// read. Class `i + 1` is `charset[i]`, class 0 the blank. `columns` is the
/// number of input columns the crop covers; each character's position is
/// the span of the steps that read it, relative to them.
pub(crate) fn decode(
    best: &[i64],
    probs: &[f32],
    used_steps: usize,
    columns: u32,
    charset: &[String],
) -> Recognition {
    let mut read = Recognition::default();
    let mut conf_sum = 0.0f32;
    let mut previous = 0usize;
    let width = columns.max(1) as f32;
    let edge = |step: usize| ((step as u32 * COLUMNS_PER_STEP) as f32 / width).min(1.0);
    for (index, (&class, &p)) in best.iter().zip(probs).take(used_steps).enumerate() {
        // A negative index cannot come from ArgMax; read it as the blank.
        let class = usize::try_from(class).unwrap_or(0);
        if class != 0 && class == previous {
            // The same character over one more step.
            if let Some(last) = read.chars.last_mut() {
                last.x1 = edge(index + 1);
            }
        } else if class != 0
            && let Some(ch) = charset.get(class - 1)
        {
            let start = read.text.len();
            read.text.push_str(ch);
            read.chars.push(CharSpan {
                start,
                end: read.text.len(),
                x0: edge(index),
                x1: edge(index + 1),
                prob: p,
            });
            conf_sum += p;
        }
        previous = class;
    }
    if !read.chars.is_empty() {
        read.conf = conf_sum / read.chars.len() as f32;
    }
    read
}

/// Decodes the output of one recognition batch: `best` and `probs` (with
/// the shapes `best_shape` and `probs_shape`) are the best class per step
/// and its probability for `batch` items of `width` input columns,
/// `columns` the columns each real item covers (padding slots have no
/// entry).
///
/// # Errors
///
/// [`OcrError::Signature`] when the outputs do not have one step per
/// [`COLUMNS_PER_STEP`] input columns for every item.
pub(crate) fn decode_batch(
    (best_shape, best): (&[i64], &[i64]),
    (probs_shape, probs): (&[i64], &[f32]),
    columns: &[u32],
    width: u32,
    batch: usize,
    charset: &[String],
) -> Result<Vec<Recognition>, OcrError> {
    let steps = (width / COLUMNS_PER_STEP) as usize;
    let expected = [batch as i64, steps as i64];
    if best_shape != expected
        || probs_shape != expected
        || best.len() != batch * steps
        || probs.len() != batch * steps
    {
        return Err(OcrError::Signature(format!(
            "recognition output shapes {best_shape:?} and {probs_shape:?} for batch {batch} x              width {width}"
        )));
    }
    Ok(columns
        .iter()
        .zip(best.chunks_exact(steps).zip(probs.chunks_exact(steps)))
        .map(|(&cols, (best, probs))| decode(best, probs, used_steps(cols, width), cols, charset))
        .collect())
}

/// Output steps that cover `columns` input columns (plus one for the
/// receptive field reaching into the padding).
pub(crate) fn used_steps(columns: u32, bucket: u32) -> usize {
    (columns.div_ceil(COLUMNS_PER_STEP) + 1).min(bucket / COLUMNS_PER_STEP) as usize
}

#[cfg(test)]
mod tests {
    use super::{bucket_for, decode, decode_batch, preprocess, scaled_width, used_steps};
    use crate::image::RgbView;
    use crate::resample::Resampler;

    #[test]
    fn picks_width_buckets() {
        let data = vec![0u8; 100 * 24 * 3];
        let crop = RgbView::new(100, 24, &data).unwrap();
        assert_eq!(scaled_width(crop), 200);
        // A stretched crop is read in its natural, narrower proportions.
        assert_eq!(scaled_width(crop.with_pixel_aspect(0.75)), 150);
        assert_eq!(bucket_for(150), 0);
        assert_eq!(bucket_for(160), 0);
        assert_eq!(bucket_for(200), 1);
        assert_eq!(bucket_for(5000), 2);
        assert_eq!(used_steps(200, 320), 26);
        assert_eq!(used_steps(640, 640), 80);
    }

    #[test]
    fn preprocess_pads_on_the_right() {
        let data = vec![255u8; 10 * 12 * 3];
        let image = RgbView::new(10, 12, &data).unwrap();
        let mut out = vec![5.0f32; 3 * 48 * 160];
        let columns = preprocess(&mut Resampler::default(), image, 160, &mut out);
        assert_eq!(columns, 40);
        assert!((out[0] - 1.0).abs() < 1e-6);
        assert_eq!(out[40], 0.0);
        assert!((out[2 * 48 * 160 + 39] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn decodes_ctc_output() {
        let charset: Vec<String> = ["a", "b", " "].iter().map(|s| (*s).to_owned()).collect();
        // Steps: a a blank a b b (b) -> "aab"; the last step is not used.
        let best = [1, 1, 0, 1, 2, 2, 2];
        let probs = [0.8f32; 7];
        let read = decode(&best, &probs, 6, 48, &charset);
        assert_eq!(read.text, "aab");
        assert!((read.conf - 0.8).abs() < 1e-6);
        // Characters sit where their steps are: the first "a" spans steps 0
        // and 1 (columns 0..16 of 48), "b" steps 4 and 5.
        let spans: Vec<(usize, f32, f32)> =
            read.chars.iter().map(|c| (c.start, c.x0, c.x1)).collect();
        let third = 16.0 / 48.0;
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].0, 0);
        assert!(spans[0].1.abs() < 1e-6 && (spans[0].2 - third).abs() < 1e-6);
        assert!((spans[2].1 - 2.0 * third).abs() < 1e-6 && (spans[2].2 - 1.0).abs() < 1e-6);
        let (x0, x1) = read.extent(1, 3).unwrap();
        assert!((x0 - 0.5).abs() < 1e-6 && (x1 - 1.0).abs() < 1e-6);
        assert!((read.conf_of(0, 1) - 0.8).abs() < 1e-6);
        assert_eq!(read.extent(3, 9), None);
        let empty = decode(&[0, 0, 0], &[0.9; 3], 3, 24, &charset);
        assert_eq!((empty.text.as_str(), empty.conf), ("", 0.0));
        // Classes beyond the character list (or negative) read as nothing.
        let outside = decode(&[9, -1, 1], &[0.9; 3], 3, 24, &charset);
        assert_eq!(outside.text, "a");
    }

    #[test]
    fn decodes_batches_and_checks_the_output_shape() {
        let charset: Vec<String> = ["a", "b"].iter().map(|s| (*s).to_owned()).collect();
        // Batch 2 of width 16 (2 steps each), one real item reading "ab".
        let best = [1, 2, 0, 0];
        let probs = [0.9, 0.7, 0.5, 0.5];
        let shape = [2i64, 2];
        let reads =
            decode_batch((&shape, &best), (&shape, &probs), &[16], 16, 2, &charset).unwrap();
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].text, "ab");
        assert!((reads[0].conf - 0.8).abs() < 1e-6);
        let wrong = [2i64, 3];
        assert!(decode_batch((&wrong, &best), (&shape, &probs), &[16], 16, 2, &charset).is_err());
        assert!(
            decode_batch(
                (&shape, &best[1..]),
                (&shape, &probs),
                &[16],
                16,
                2,
                &charset
            )
            .is_err()
        );
    }
}
