//! The frozen high-frequency residual gate on synthetic, name-like text.
//!
//! The threshold itself was calibrated on real kill-feed boxes (see
//! `HF_RESIDUAL_MAX`); this test keeps the smooth styles below it on
//! generated glyphs for every strength, scale, sample format and with and
//! without noise.

mod support;

use blurrd_blur::metrics::{HF_RESIDUAL_MAX, hf_residual, laplacian_variance};
use blurrd_blur::{BlurBackend, CpuBlur, PixelParams, SampleFormat, YuvColour, prepare_boxes};
use blurrd_core::{BlurBox, BlurParams, BlurStyle, Region, Strength};
use support::{Format, OwnedFrame, Rng};

#[test]
fn smooth_styles_stay_below_the_gate() {
    let mut worst = 0.0f64;
    let mut measured = 0;
    for format in [Format::Nv12, Format::P010] {
        let scale = if format == Format::P010 { 4 } else { 1 };
        for sy in [2.0f32 / 3.0, 1.0, 4.0 / 3.0, 2.0] {
            for seed in 0..6u64 {
                let mut rng = Rng(seed);
                let mut source =
                    OwnedFrame::flat(format, 640, 160, [40 * scale, 128 * scale, 128 * scale]);
                let height = (14.0 * sy).round() as u32;
                let stroke = (2.0 * sy).round().max(1.0) as u32;
                let ink = (200 + rng.below(50) as u16) * scale;
                let text = {
                    let count = 5 + rng.below(8);
                    source.draw_text(&mut rng, (50, 40), count, height, stroke, ink)
                };
                let rect = text;
                let y_format = if format == Format::P010 {
                    SampleFormat::U16Le
                } else {
                    SampleFormat::U8
                };
                for style in [BlurStyle::PixelBlur, BlurStyle::Gaussian] {
                    for strength in [Strength::Low, Strength::Medium, Strength::High] {
                        for noise in [false, true] {
                            for block_scale in [1.0, 1.5] {
                                let params = BlurParams {
                                    style,
                                    strength,
                                    noise,
                                    ..BlurParams::default()
                                };
                                let pixel =
                                    PixelParams::new(&params, sy, YuvColour::default()).unwrap();
                                let blur_box = BlurBox {
                                    rect,
                                    region: Region::KillFeed,
                                    seed: seed as u32,
                                    grid_origin: (rect.x0, rect.y0),
                                    block_scale,
                                };
                                let mut prepared = vec![];
                                prepare_boxes(&[blur_box], 640, 160, &pixel, &mut prepared);
                                let mut output = source.clone();
                                CpuBlur
                                    .blur(&mut output.view(), (0, 0), &prepared, &pixel)
                                    .unwrap();
                                let mut original = source.clone();
                                let src_plane = original.view();
                                let out_plane = output.view();
                                assert!(
                                    laplacian_variance(src_plane.luma().as_plane(), y_format, rect)
                                        .unwrap()
                                        > 1000.0
                                );
                                let r = hf_residual(
                                    src_plane.luma().as_plane(),
                                    out_plane.luma().as_plane(),
                                    y_format,
                                    rect,
                                )
                                .unwrap();
                                worst = worst.max(r);
                                measured += 1;
                                assert!(
                                    r <= HF_RESIDUAL_MAX,
                                    "{format:?} sy {sy} {style:?} {strength:?} noise {noise} \
                                     scale {block_scale}: residual {r}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    println!("{measured} boxes, worst residual {worst:.5} (gate {HF_RESIDUAL_MAX})");
}
