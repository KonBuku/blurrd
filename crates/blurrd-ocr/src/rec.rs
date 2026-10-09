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
use crate::resample::{Normalise, resize_into_planes};

/// Input height of the recognition model.
pub const REC_HEIGHT: u32 = 48;
/// The fixed input widths. Every width is compiled once at warm-up, so
/// recognition never meets a new shape (architecture plan 5.5).
pub const WIDTH_BUCKETS: [u32; 3] = [160, 320, 640];
/// Input columns per output step of the recognition model.
const COLUMNS_PER_STEP: u32 = 8;

/// One recognised text line.
#[derive(Debug, Clone, PartialEq)]
pub struct Recognition {
    /// The text. It stays in memory; blurrd never logs or stores it.
    pub text: String,
    /// Mean probability of the decoded characters, `0.0..=1.0` (0 when no
    /// character was read).
    pub conf: f32,
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

/// Writes one crop as CHW into `out` (`3 × REC_HEIGHT × bucket` values):
/// scaled to the model height, normalised to `-1..=1` in blue, green, red
/// order, padded with 0 on the right. Returns the number of input columns
/// the crop covers.
pub(crate) fn preprocess(image: RgbView<'_>, bucket: u32, out: &mut [f32]) -> u32 {
    let width = scaled_width(image).min(bucket);
    out.fill(0.0);
    let norm = Normalise {
        scale: [1.0 / 127.5; 3],
        offset: [-1.0; 3],
    };
    let plane = (REC_HEIGHT * bucket) as usize;
    resize_into_planes(image, width, REC_HEIGHT, out, bucket as usize, plane, norm);
    width
}

/// Greedy CTC decoding of one item: `probs` holds `steps × classes`
/// probabilities; only the first `used_steps` steps (those covering the
/// crop rather than padding) are read. Class `i + 1` is `charset[i]`.
pub(crate) fn decode(
    probs: &[f32],
    classes: usize,
    used_steps: usize,
    charset: &[String],
) -> Recognition {
    let mut text = String::new();
    let mut conf_sum = 0.0f32;
    let mut count = 0u32;
    let mut previous = 0usize;
    for step in probs.chunks_exact(classes).take(used_steps) {
        let (best, p) = argmax(step);
        if best != 0
            && best != previous
            && let Some(ch) = charset.get(best - 1)
        {
            text.push_str(ch);
            conf_sum += p;
            count += 1;
        }
        previous = best;
    }
    Recognition {
        text,
        conf: if count == 0 {
            0.0
        } else {
            conf_sum / count as f32
        },
    }
}

/// Decodes the output of one recognition batch: `shape` and `data` are the
/// model output for `batch` items of `width` input columns, `columns` the
/// columns each real item covers (padding slots have no entry).
///
/// # Errors
///
/// [`OcrError::Signature`] when the output does not have one step per
/// [`COLUMNS_PER_STEP`] input columns and `classes` classes per step.
pub(crate) fn decode_batch(
    shape: &[i64],
    data: &[f32],
    columns: &[u32],
    width: u32,
    batch: usize,
    classes: usize,
    charset: &[String],
) -> Result<Vec<Recognition>, OcrError> {
    let steps = (width / COLUMNS_PER_STEP) as usize;
    let per_item = steps * classes;
    if shape.len() != 3 || shape[2] as usize != classes || data.len() != batch * per_item {
        return Err(OcrError::Signature(format!(
            "recognition output shape {shape:?} for batch {batch} x width {width}"
        )));
    }
    Ok(columns
        .iter()
        .zip(data.chunks_exact(per_item))
        .map(|(&cols, probs)| decode(probs, classes, used_steps(cols, width), charset))
        .collect())
}

/// Output steps that cover `columns` input columns (plus one for the
/// receptive field reaching into the padding).
pub(crate) fn used_steps(columns: u32, bucket: u32) -> usize {
    (columns.div_ceil(COLUMNS_PER_STEP) + 1).min(bucket / COLUMNS_PER_STEP) as usize
}

/// Index and value of the largest element (the first on ties).
fn argmax(values: &[f32]) -> (usize, f32) {
    let mut best = (0, f32::NEG_INFINITY);
    for (i, &v) in values.iter().enumerate() {
        if v > best.1 {
            best = (i, v);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::{bucket_for, decode, decode_batch, preprocess, scaled_width, used_steps};
    use crate::image::RgbView;

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
        let columns = preprocess(image, 160, &mut out);
        assert_eq!(columns, 40);
        assert!((out[0] - 1.0).abs() < 1e-6);
        assert_eq!(out[40], 0.0);
        assert!((out[2 * 48 * 160 + 39] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn decodes_ctc_output() {
        let charset: Vec<String> = ["a", "b", " "].iter().map(|s| (*s).to_owned()).collect();
        // Steps: a a blank a b b (b) -> "aab"; the last step is not used.
        let steps = [1, 1, 0, 1, 2, 2, 2];
        let classes = 4;
        let mut probs = vec![0.0f32; steps.len() * classes];
        for (t, &c) in steps.iter().enumerate() {
            probs[t * classes + c] = 0.8;
        }
        let read = decode(&probs, classes, 6, &charset);
        assert_eq!(read.text, "aab");
        assert!((read.conf - 0.8).abs() < 1e-6);
        let empty = decode(&vec![0.0; 3 * classes], classes, 3, &charset);
        assert_eq!((empty.text.as_str(), empty.conf), ("", 0.0));
    }

    #[test]
    fn decodes_batches_and_checks_the_output_shape() {
        let charset: Vec<String> = ["a", "b"].iter().map(|s| (*s).to_owned()).collect();
        let classes = 3;
        // Batch 2 of width 16 (2 steps each), one real item reading "ab".
        let mut data = vec![0.0f32; 2 * 2 * classes];
        data[1] = 0.9;
        data[classes + 2] = 0.7;
        let reads = decode_batch(&[2, 2, 3], &data, &[16], 16, 2, classes, &charset).unwrap();
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].text, "ab");
        assert!(decode_batch(&[2, 2, 4], &data, &[16], 16, 2, classes, &charset).is_err());
        assert!(decode_batch(&[2, 2, 3], &data[1..], &[16], 16, 2, classes, &charset).is_err());
    }
}
