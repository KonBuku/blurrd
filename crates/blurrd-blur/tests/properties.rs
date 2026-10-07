//! Property tests of the blur's guarantees: full coverage of the unpadded
//! box in luma and max-pooled chroma, even outward alignment, locality
//! (full frame and region-of-interest crops give identical pixels),
//! determinism, a block grid that follows the text, and the strength floor.

#![allow(
    clippy::many_single_char_names,
    clippy::unwrap_used,
    clippy::cast_possible_wrap,
    reason = "test code: short pixel coordinate names, small synthetic sizes, panics are failures"
)]

mod support;

use blurrd_blur::{BlurBackend, CpuBlur, PixelParams, PreparedBox, YuvColour, prepare_boxes};
use blurrd_core::{BlurBox, BlurParams, BlurStyle, RectI, Region, SolidColor, Strength};
use proptest::prelude::*;
use support::{Format, OwnedFrame};

/// Blurs `frame` (full frame) and returns the prepared boxes.
fn blur_full(frame: &mut OwnedFrame, boxes: &[BlurBox], params: &PixelParams) -> Vec<PreparedBox> {
    let mut prepared = vec![];
    prepare_boxes(boxes, frame.width, frame.height, params, &mut prepared);
    CpuBlur
        .blur(&mut frame.view(), (0, 0), &prepared, params)
        .unwrap();
    prepared
}

fn format() -> impl Strategy<Value = Format> {
    prop::sample::select(Format::ALL.to_vec())
}

fn blur_params() -> impl Strategy<Value = BlurParams> {
    (
        prop_oneof![
            Just(BlurStyle::PixelBlur),
            Just(BlurStyle::Gaussian),
            Just(BlurStyle::Pixelate),
            Just(BlurStyle::Solid {
                color: SolidColor::BoxMean
            }),
        ],
        prop_oneof![
            Just(Strength::Low),
            Just(Strength::Medium),
            Just(Strength::High),
            (0u8..30).prop_map(Strength::Custom),
        ],
        (0u8..8, 0u8..8),
        0u8..16,
        0.0f32..4.0,
        any::<bool>(),
    )
        .prop_map(
            |(style, strength, pad, radius, feather, noise)| BlurParams {
                style,
                strength,
                pad,
                radius,
                feather,
                noise,
            },
        )
}

/// Up to four boxes inside (or partly outside) a `w` × `h` frame.
fn boxes(w: i32, h: i32) -> impl Strategy<Value = Vec<BlurBox>> {
    prop::collection::vec(
        (
            -8..w,
            -8..h,
            1..60i32,
            1..30i32,
            -20..20i32,
            -20..20i32,
            any::<u32>(),
            prop_oneof![Just(1.0f32), Just(1.5f32)],
        )
            .prop_map(|(x, y, bw, bh, gx, gy, seed, block_scale)| BlurBox {
                rect: RectI::new(x, y, x + bw, y + bh).unwrap(),
                region: Region::KillFeed,
                seed,
                grid_origin: (x + gx, y + gy),
                block_scale,
            }),
        1..5,
    )
}

/// A frame size, possibly odd.
fn size() -> impl Strategy<Value = (u32, u32)> {
    (40u32..140, 30u32..90)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Every unpadded pixel gets mask 1 (>= 0.999 required) in luma and in
    /// the max-pooled chroma, and a fixed fill colour replaces every such
    /// sample exactly.
    #[test]
    fn covers_every_unpadded_pixel(
        format in format(),
        (w, h) in size(),
        params in blur_params(),
        sy in prop_oneof![Just(2.0f32 / 3.0), Just(1.0f32), Just(1.5f32)],
        seed in any::<u64>(),
        boxes in boxes(140, 90),
    ) {
        let solid = BlurParams {
            style: BlurStyle::Solid { color: SolidColor::Rgb([250, 20, 200]) },
            ..params
        };
        let pixel = PixelParams::new(&solid, sy, YuvColour::default()).unwrap();
        let fill = {
            let mut probe = OwnedFrame::flat(format, 2, 2, [0; 3]);
            let all = [BlurBox { rect: RectI::new(0, 0, 2, 2).unwrap(), region: Region::Manual,
                                 seed: 0, grid_origin: (0, 0), block_scale: 1.0 }];
            blur_full(&mut probe, &all, &pixel);
            [probe.get(0, 0, 0), probe.get(1, 0, 0), probe.get(2, 0, 0)]
        };
        let mut frame = OwnedFrame::noise(format, w, h, seed);
        let prepared = blur_full(&mut frame, &boxes, &pixel);
        for b in &prepared {
            let core = b.core();
            for y in core.y0..core.y1 {
                for x in core.x0..core.x1 {
                    prop_assert!(b.coverage(x, y) >= 0.999);
                    prop_assert_eq!(frame.get(0, x as u32, y as u32), fill[0]);
                }
            }
            for cy in core.y0 / 2..(core.y1 + 1) / 2 {
                for cx in core.x0 / 2..(core.x1 + 1) / 2 {
                    prop_assert!(b.chroma_coverage(cx, cy) >= 0.999);
                    prop_assert_eq!(frame.get(1, cx as u32, cy as u32), fill[1]);
                    prop_assert_eq!(frame.get(2, cx as u32, cy as u32), fill[2]);
                }
            }
        }
    }

    /// The chroma mask is the maximum of the luma mask over the 2x2 luma
    /// pixels of each chroma sample.
    #[test]
    fn chroma_mask_is_a_max_pool(params in blur_params(), boxes in boxes(140, 90)) {
        let pixel = PixelParams::new(&params, 1.0, YuvColour::default()).unwrap();
        let mut prepared = vec![];
        prepare_boxes(&boxes, 140, 90, &pixel, &mut prepared);
        for b in &prepared {
            let p = b.padded();
            for cy in (p.y0 / 2 - 1)..=((p.y1 + 1) / 2) {
                for cx in (p.x0 / 2 - 1)..=((p.x1 + 1) / 2) {
                    let luma = [(0, 0), (1, 0), (0, 1), (1, 1)]
                        .map(|(dx, dy)| b.coverage(2 * cx + dx, 2 * cy + dy));
                    let max = luma.into_iter().fold(0.0f32, f32::max);
                    prop_assert!((b.chroma_coverage(cx, cy) - max).abs() <= f32::EPSILON);
                }
            }
        }
    }

    /// Padded boxes stay in the frame, contain the padded rectangle (cut to
    /// the frame) and have even edges except where they end at an odd frame
    /// edge; nothing outside them is written.
    #[test]
    fn aligns_outward_and_writes_only_padded_boxes(
        format in format(),
        (w, h) in size(),
        params in blur_params(),
        seed in any::<u64>(),
        boxes in boxes(140, 90),
    ) {
        let pixel = PixelParams::new(&params, 1.0, YuvColour::default()).unwrap();
        let source = OwnedFrame::noise(format, w, h, seed);
        let mut frame = source.clone();
        let prepared = blur_full(&mut frame, &boxes, &pixel);
        let (px, py) = pixel.pad();
        for (b, original) in prepared.iter().zip(boxes.iter().filter(|b| {
            b.rect.intersection(&RectI::new(0, 0, w as i32, h as i32).unwrap()).is_some()
        })) {
            let p = b.padded();
            prop_assert!(p.is_inside(w, h));
            prop_assert!(p.x0 % 2 == 0 && p.y0 % 2 == 0);
            prop_assert!(p.x1 % 2 == 0 || p.x1 == w as i32);
            prop_assert!(p.y1 % 2 == 0 || p.y1 == h as i32);
            let r = original.rect;
            prop_assert!(p.x0 <= (r.x0 - px).max(0) && p.y0 <= (r.y0 - py).max(0));
            prop_assert!(p.x1 >= (r.x1 + px).min(w as i32) && p.y1 >= (r.y1 + py).min(h as i32));
        }
        let inside = |x: i32, y: i32| prepared.iter().any(|b| {
            let p = b.padded();
            x >= p.x0 && x < p.x1 && y >= p.y0 && y < p.y1
        });
        for c in 0..3 {
            let s = if c == 0 { 1 } else { 2 };
            let (cw, ch) = frame.channel_size(c);
            for y in 0..ch {
                for x in 0..cw {
                    let touched = (0..s).any(|dy| (0..s).any(|dx| {
                        inside((x * s + dx) as i32, (y * s + dy) as i32)
                    }));
                    if !touched {
                        prop_assert_eq!(frame.get(c, x, y), source.get(c, x, y));
                    }
                }
            }
        }
    }

    /// Locality: changing source pixels outside every padded box changes no
    /// output pixel inside them, and blurring a crop with its origin gives
    /// exactly the pixels of the full-frame blur.
    #[test]
    fn is_local_and_crop_exact(
        format in format(),
        params in blur_params(),
        seed in any::<u64>(),
        boxes in boxes(120, 80),
        (ox, oy) in (0i32..20, 0i32..20),
    ) {
        let (w, h) = (160u32, 110u32);
        let pixel = PixelParams::new(&params, 1.0, YuvColour::default()).unwrap();
        let source = OwnedFrame::noise(format, w, h, seed);
        let mut full = source.clone();
        let prepared = blur_full(&mut full, &boxes, &pixel);

        // Perturb everything outside the padded boxes.
        let mut other = OwnedFrame::noise(format, w, h, seed ^ 0xDEAD_BEEF);
        for b in &prepared {
            let p = b.padded();
            for c in 0..3 {
                let s = if c == 0 { 1 } else { 2 };
                for y in p.y0 / s..(p.y1 + s - 1) / s {
                    for x in p.x0 / s..(p.x1 + s - 1) / s {
                        other.set(c, x as u32, y as u32, source.get(c, x as u32, y as u32));
                    }
                }
            }
        }
        let mut perturbed = other.clone();
        blur_full(&mut perturbed, &boxes, &pixel);
        for b in &prepared {
            let p = b.padded();
            for y in p.y0..p.y1 {
                for x in p.x0..p.x1 {
                    prop_assert_eq!(perturbed.get(0, x as u32, y as u32), full.get(0, x as u32, y as u32));
                }
            }
        }

        // Crop with an even origin that contains every padded box.
        let origin = (ox & !1, oy & !1);
        let mut roi = RectI::new(origin.0, origin.1, w as i32, h as i32).unwrap();
        for b in &prepared {
            let p = b.padded();
            roi = RectI::new(roi.x0.min(p.x0 & !1), roi.y0.min(p.y0 & !1), roi.x1, roi.y1).unwrap();
        }
        let mut crop = source.crop(roi);
        CpuBlur.blur(&mut crop.view(), (roi.x0, roi.y0), &prepared, &pixel).unwrap();
        prop_assert_eq!(crop, full.crop(roi));
    }

    /// The block grid follows `grid_origin`: moving the content, the box
    /// and its grid origin by the same even offset moves the output by
    /// that offset, bit for bit.
    #[test]
    fn grid_follows_the_text(
        format in format(),
        params in blur_params(),
        seed in any::<u64>(),
        (dx, dy) in (0u32..12, 0u32..12),
        (gx, gy) in (-9i32..9, -9i32..9),
    ) {
        let (dx, dy) = (dx * 2, dy * 2);
        let (w, h) = (200u32, 120u32);
        let pixel = PixelParams::new(&params, 1.0, YuvColour::default()).unwrap();
        let base = OwnedFrame::noise(format, w, h, seed);
        let mut shifted = OwnedFrame::noise(format, w, h, seed ^ 1);
        for c in 0..3 {
            let s = if c == 0 { 1 } else { 2 };
            let (cw, ch) = base.channel_size(c);
            for y in 0..ch - dy / s {
                for x in 0..cw - dx / s {
                    shifted.set(c, x + dx / s, y + dy / s, base.get(c, x, y));
                }
            }
        }
        let rect = RectI::new(40, 30, 120, 60).unwrap();
        let make = |off: (i32, i32)| BlurBox {
            rect: RectI::new(rect.x0 + off.0, rect.y0 + off.1, rect.x1 + off.0, rect.y1 + off.1).unwrap(),
            region: Region::KillFeed,
            seed: 42,
            grid_origin: (rect.x0 + gx + off.0, rect.y0 + gy + off.1),
            block_scale: 1.0,
        };
        let mut a = base.clone();
        let pa = blur_full(&mut a, &[make((0, 0))], &pixel);
        let mut b = shifted.clone();
        blur_full(&mut b, &[make((dx as i32, dy as i32))], &pixel);
        let p = pa[0].padded();
        for c in 0..3 {
            let s = if c == 0 { 1u32 } else { 2 };
            for y in p.y0 as u32 / s..p.y1 as u32 / s {
                for x in p.x0 as u32 / s..p.x1 as u32 / s {
                    prop_assert_eq!(a.get(c, x, y), b.get(c, x + dx / s, y + dy / s));
                }
            }
        }
    }
}

/// The same blur on many threads gives the same bytes as on one.
#[test]
fn is_deterministic_across_threads() {
    let mut boxes = vec![];
    for i in 0..12 {
        let (x, y) = (40 + (i % 3) * 300, 60 + (i / 3) * 90);
        boxes.push(BlurBox {
            rect: RectI::new(x, y, x + 160, y + 30).unwrap(),
            region: Region::KillFeed,
            seed: i as u32,
            grid_origin: (x + 3, y + 1),
            block_scale: if i % 2 == 0 { 1.0 } else { 1.5 },
        });
    }
    for format in Format::ALL {
        let source = OwnedFrame::noise(format, 1280, 720, 99);
        let pixel = PixelParams::new(&BlurParams::default(), 0.75, YuvColour::default()).unwrap();
        let mut reference = source.clone();
        blur_full(&mut reference, &boxes, &pixel);
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        let mut frame = source.clone();
                        blur_full(&mut frame, &boxes, &pixel);
                        frame
                    })
                })
                .collect();
            for handle in handles {
                assert_eq!(handle.join().unwrap(), reference);
            }
        });
    }
}

/// Custom strengths below the 8 px floor blur exactly like `Low`.
#[test]
fn enforces_the_strength_floor() {
    let boxes = [BlurBox {
        rect: RectI::new(30, 20, 150, 44).unwrap(),
        region: Region::VoiceChat,
        seed: 5,
        grid_origin: (31, 22),
        block_scale: 1.0,
    }];
    let source = OwnedFrame::noise(Format::Nv12, 200, 80, 3);
    let blurred = |strength| {
        let params = BlurParams {
            strength,
            ..BlurParams::default()
        };
        let pixel = PixelParams::new(&params, 1.0, YuvColour::default()).unwrap();
        let mut frame = source.clone();
        blur_full(&mut frame, &boxes, &pixel);
        frame
    };
    let low = blurred(Strength::Low);
    for px in 0..8 {
        assert_eq!(blurred(Strength::Custom(px)), low, "Custom({px})");
    }
    assert_ne!(blurred(Strength::Custom(9)), low);
}
