//! Writes reference frames blurred by [`CpuBlur`] for the desktop app's
//! WebGL preview, which must match the export within a documented
//! tolerance (architecture plan, M24).
//!
//! ```text
//! cargo run -p blurrd-blur --example reference_frames -- [OUT_DIR] [--i420 FILE WIDTH HEIGHT]...
//! ```
//!
//! `OUT_DIR` defaults to `target/blur-reference`. Every case blurs a
//! BT.709 limited-range I420 frame and stores the input and the expected
//! output as packed 8-bit RGB (`*.rgb`, chroma upsampled by repetition),
//! described in `cases.json`. The built-in frames are synthetic; `--i420`
//! adds a raw `yuv420p` frame (for example one decoded from the user's
//! own footage) with kill-feed, banner and chat boxes placed relative to
//! the frame height. Nothing is written into the repository.

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use blurrd_blur::{
    BlurBackend, CpuBlur, PixelParams, PlaneMut, YuvColour, YuvFrameMut, prepare_boxes,
};
use blurrd_core::{BlurBox, BlurParams, BlurStyle, RectI, Region, SolidColor, Strength};

/// An I420 frame that owns its planes.
struct Frame {
    name: String,
    width: u32,
    height: u32,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

impl Frame {
    /// A synthetic HUD-like frame, rendered in RGB like a game frame and
    /// converted to I420 (BT.709, limited range, chroma averaged over 2×2
    /// pixels): a gradient with fine grain, coloured bars with text-like
    /// white strokes, and saturated colour bands.
    fn synthetic(width: u32, height: u32) -> Self {
        let scale = height as f32 / 360.0;
        let pixel = |col: u32, row: u32| -> [f32; 3] {
            let shade =
                40.0 + 120.0 * col as f32 / width as f32 + 40.0 * row as f32 / height as f32;
            let grain = (hash(col, row) % 9) as f32 - 4.0;
            let tint = match (row as f32 / (scale * 30.0)) as u32 % 4 {
                0 => [1.25, 0.55, 0.5],
                1 => [0.45, 1.1, 0.95],
                2 => [0.8 + 0.4 * col as f32 / width as f32, 0.9, 1.1],
                _ => [1.0, 0.95, 0.6],
            };
            let cell =
                (col as f32 / (6.0 * scale)) as u32 ^ ((row as f32 / (9.0 * scale)) as u32 * 31);
            let in_bar = (row as f32 / scale) as u32 % 60 < 18;
            if in_bar && hash(cell, 7).is_multiple_of(3) && !(col + row).is_multiple_of(3) {
                [235.0, 238.0, 240.0]
            } else {
                tint.map(|t| (shade * t + grain).clamp(0.0, 255.0))
            }
        };
        let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
        let mut frame = Self {
            name: format!("synthetic_{width}x{height}"),
            width,
            height,
            y: Vec::with_capacity((width * height) as usize),
            u: vec![0; (cw * ch) as usize],
            v: vec![0; (cw * ch) as usize],
        };
        let mut chroma = vec![[0.0f32; 3]; (cw * ch) as usize];
        for row in 0..height {
            for col in 0..width {
                let [r, g, b] = pixel(col, row);
                let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                frame.y.push((16.0 + luma * 219.0 / 255.0).round() as u8);
                let sum = &mut chroma[((row / 2) * cw + col / 2) as usize];
                sum[0] += (b - luma) / 1.8556;
                sum[1] += (r - luma) / 1.5748;
                sum[2] += 1.0;
            }
        }
        for (i, [cb, cr, n]) in chroma.into_iter().enumerate() {
            frame.u[i] = (128.0 + cb / n * 224.0 / 255.0).round().clamp(16.0, 240.0) as u8;
            frame.v[i] = (128.0 + cr / n * 224.0 / 255.0).round().clamp(16.0, 240.0) as u8;
        }
        frame
    }

    /// A raw `yuv420p` frame read from `path`.
    fn read_i420(path: &Path, width: u32, height: u32) -> Result<Self, Box<dyn Error>> {
        let bytes = fs::read(path)?;
        let luma = (width * height) as usize;
        let chroma = (width.div_ceil(2) * height.div_ceil(2)) as usize;
        if bytes.len() < luma + 2 * chroma {
            return Err(format!(
                "{} is smaller than one {width}x{height} I420 frame",
                path.display()
            )
            .into());
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("frame");
        Ok(Self {
            name: format!("{stem}_{width}x{height}"),
            width,
            height,
            y: bytes[..luma].to_vec(),
            u: bytes[luma..luma + chroma].to_vec(),
            v: bytes[luma + chroma..luma + 2 * chroma].to_vec(),
        })
    }

    /// Packed RGB (BT.709, limited range), chroma repeated over 2×2 pixels.
    fn rgb(&self) -> Vec<u8> {
        let cw = self.width.div_ceil(2) as usize;
        let mut out = Vec::with_capacity((self.width * self.height * 3) as usize);
        for row in 0..self.height as usize {
            for col in 0..self.width as usize {
                let c = (row / 2) * cw + col / 2;
                let luma = 1.164_384 * (f32::from(self.y[row * self.width as usize + col]) - 16.0);
                let (pu, pv) = (f32::from(self.u[c]) - 128.0, f32::from(self.v[c]) - 128.0);
                for value in [
                    luma + 1.792_741 * pv,
                    luma - 0.213_249 * pu - 0.532_909 * pv,
                    luma + 2.112_402 * pu,
                ] {
                    out.push((value.round()).clamp(0.0, 255.0) as u8);
                }
            }
        }
        out
    }

    /// Blurs `boxes` into a copy of the frame with [`CpuBlur`].
    fn blurred(&self, params: &PixelParams, boxes: &[BlurBox]) -> Result<Self, Box<dyn Error>> {
        let mut out = Self {
            name: String::new(),
            width: self.width,
            height: self.height,
            y: self.y.clone(),
            u: self.u.clone(),
            v: self.v.clone(),
        };
        let (cw, ch) = (self.width.div_ceil(2), self.height.div_ceil(2));
        let mut frame = YuvFrameMut::I420 {
            y: PlaneMut {
                data: &mut out.y,
                stride: self.width as usize,
                width: self.width,
                height: self.height,
            },
            u: PlaneMut {
                data: &mut out.u,
                stride: cw as usize,
                width: cw,
                height: ch,
            },
            v: PlaneMut {
                data: &mut out.v,
                stride: cw as usize,
                width: cw,
                height: ch,
            },
        };
        let mut prepared = Vec::new();
        prepare_boxes(boxes, self.width, self.height, params, &mut prepared);
        CpuBlur.blur(&mut frame, (0, 0), &prepared, params)?;
        Ok(out)
    }
}

/// A small integer hash for the synthetic content.
fn hash(a: u32, b: u32) -> u32 {
    let mut z = a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77);
    z ^= z >> 15;
    z = z.wrapping_mul(0x2C1B_3C6D);
    z ^ (z >> 12)
}

/// One reference case.
struct Case {
    name: String,
    sy: f32,
    params: BlurParams,
    boxes: Vec<BlurBox>,
}

fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> Result<RectI, Box<dyn Error>> {
    Ok(RectI::new(x0, y0, x1, y1)?)
}

/// A box with its grid at the top-left corner of `r` shifted by `offset`.
fn blur_box(r: RectI, seed: u32, offset: (i32, i32), block_scale: f32) -> BlurBox {
    BlurBox {
        rect: r,
        region: Region::KillFeed,
        seed,
        grid_origin: (r.x0 + offset.0, r.y0 + offset.1),
        block_scale,
    }
}

/// Kill-feed rows, a banner and a chat name placed relative to the frame.
fn hud_boxes(width: u32, height: u32) -> Result<Vec<BlurBox>, Box<dyn Error>> {
    let s = height as f32 / 1080.0;
    let at = |v: u16| (f32::from(v) * s).round() as i32;
    let right = i32::try_from(width)?;
    Ok(vec![
        blur_box(
            rect(right - at(560), at(92), right - at(380), at(118))?,
            11,
            (0, 0),
            1.0,
        ),
        blur_box(
            rect(right - at(250), at(92), right - at(40), at(118))?,
            12,
            (3, 2),
            1.0,
        ),
        blur_box(
            rect(right - at(520), at(132), right - at(330), at(158))?,
            13,
            (0, 0),
            1.5,
        ),
        blur_box(rect(at(810), at(470), at(1110), at(520))?, 14, (0, 0), 1.0),
        blur_box(rect(at(34), at(860), at(160), at(884))?, 15, (1, 0), 1.0),
    ])
}

/// Boxes that exercise clamping, odd coordinates and overlap.
fn edge_boxes(width: u32, height: u32) -> Result<Vec<BlurBox>, Box<dyn Error>> {
    let (w, h) = (i32::try_from(width)?, i32::try_from(height)?);
    Ok(vec![
        blur_box(rect(0, 0, 61, 23)?, 21, (0, 0), 1.0),
        blur_box(rect(w - 77, h - 19, w, h)?, 22, (-3, 1), 1.0),
        blur_box(rect(201, 117, 333, 143)?, 23, (0, 0), 1.5),
        blur_box(rect(301, 131, 397, 160)?, 24, (5, 5), 1.0),
    ])
}

fn params(style: BlurStyle, strength: Strength) -> BlurParams {
    BlurParams {
        style,
        strength,
        ..BlurParams::default()
    }
}

/// The JSON of `params` in the project format.
fn params_json(p: &BlurParams) -> String {
    let style = match p.style {
        BlurStyle::Gaussian => "\"Gaussian\"".to_owned(),
        BlurStyle::Pixelate => "\"Pixelate\"".to_owned(),
        BlurStyle::Solid {
            color: SolidColor::Rgb([r, g, b]),
        } => {
            format!("{{\"Solid\":{{\"color\":{{\"Rgb\":[{r},{g},{b}]}}}}}}")
        }
        BlurStyle::Solid { .. } => "{\"Solid\":{\"color\":\"BoxMean\"}}".to_owned(),
        _ => "\"PixelBlur\"".to_owned(),
    };
    let strength = match p.strength {
        Strength::Low => "\"Low\"".to_owned(),
        Strength::High => "\"High\"".to_owned(),
        Strength::Custom(px) => format!("{{\"Custom\":{px}}}"),
        _ => "\"Medium\"".to_owned(),
    };
    format!(
        "{{\"style\":{style},\"strength\":{strength},\"pad\":[{},{}],\"radius\":{},\"feather\":{},\"noise\":{}}}",
        p.pad.0, p.pad.1, p.radius, p.feather, p.noise
    )
}

/// The cases of the synthetic 640×360 frame.
fn synthetic_cases(width: u32, height: u32) -> Result<Vec<Case>, Box<dyn Error>> {
    let hud = hud_boxes(width, height)?;
    let case = |name: &str, sy: f32, params: BlurParams, boxes: &[BlurBox]| Case {
        name: name.to_owned(),
        sy,
        params,
        boxes: boxes.to_vec(),
    };
    let hard = BlurParams {
        feather: 0.0,
        radius: 0,
        noise: false,
        ..BlurParams::default()
    };
    Ok(vec![
        case("pixelblur_medium", 1.0, BlurParams::default(), &hud),
        case(
            "pixelblur_low_720p",
            2.0 / 3.0,
            params(BlurStyle::PixelBlur, Strength::Low),
            &hud,
        ),
        case(
            "pixelblur_high_4k",
            2.0,
            params(BlurStyle::PixelBlur, Strength::High),
            &hud,
        ),
        case(
            "gaussian_medium",
            1.0,
            params(BlurStyle::Gaussian, Strength::Medium),
            &hud,
        ),
        case(
            "pixelate_medium",
            1.0,
            params(BlurStyle::Pixelate, Strength::Medium),
            &hud,
        ),
        case(
            "solid_box_mean",
            1.0,
            params(
                BlurStyle::Solid {
                    color: SolidColor::BoxMean,
                },
                Strength::Medium,
            ),
            &hud,
        ),
        case(
            "solid_rgb",
            1.0,
            params(
                BlurStyle::Solid {
                    color: SolidColor::Rgb([20, 22, 28]),
                },
                Strength::Medium,
            ),
            &hud,
        ),
        case(
            "edges_overlap",
            1.0,
            BlurParams::default(),
            &edge_boxes(width, height)?,
        ),
        case(
            "hard_edges_custom_block",
            1.0,
            BlurParams {
                strength: Strength::Custom(12),
                ..hard
            },
            &hud,
        ),
    ])
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mut out_dir = PathBuf::from("target/blur-reference");
    let mut extra = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--i420" {
            let (Some(path), Some(w), Some(h)) = (args.next(), args.next(), args.next()) else {
                return Err("--i420 needs FILE WIDTH HEIGHT".into());
            };
            extra.push(Frame::read_i420(Path::new(&path), w.parse()?, h.parse()?)?);
        } else {
            out_dir = PathBuf::from(arg);
        }
    }
    fs::create_dir_all(&out_dir)?;
    let mut jobs: Vec<(Frame, Vec<Case>)> = Vec::new();
    let small = Frame::synthetic(640, 360);
    let small_cases = synthetic_cases(small.width, small.height)?;
    jobs.push((small, small_cases));
    let full = Frame::synthetic(1920, 1080);
    let full_cases = vec![Case {
        name: "pixelblur_medium_1080p".to_owned(),
        sy: 1.0,
        params: BlurParams::default(),
        boxes: hud_boxes(1920, 1080)?,
    }];
    jobs.push((full, full_cases));
    for frame in extra {
        let cases = vec![Case {
            name: format!("{}_default", frame.name),
            sy: frame.height as f32 / 1080.0,
            params: BlurParams::default(),
            boxes: hud_boxes(frame.width, frame.height)?,
        }];
        jobs.push((frame, cases));
    }

    let mut json = String::from("{\"cases\":[");
    let mut first = true;
    for (frame, cases) in &jobs {
        let input = format!("{}.rgb", frame.name);
        fs::write(out_dir.join(&input), frame.rgb())?;
        for case in cases {
            let pixel = PixelParams::new(&case.params, case.sy, YuvColour::default())?;
            let expected = format!("{}.rgb", case.name);
            fs::write(
                out_dir.join(&expected),
                frame.blurred(&pixel, &case.boxes)?.rgb(),
            )?;
            let boxes: Vec<String> = case
                .boxes
                .iter()
                .map(|b| {
                    format!(
                        "{{\"rect\":[{},{},{},{}],\"seed\":{},\"grid_origin\":[{},{}],\"block_scale\":{}}}",
                        b.rect.x0, b.rect.y0, b.rect.x1, b.rect.y1, b.seed, b.grid_origin.0, b.grid_origin.1, b.block_scale
                    )
                })
                .collect();
            if !first {
                json.push(',');
            }
            first = false;
            write!(
                json,
                "{{\"name\":\"{}\",\"input\":\"{input}\",\"expected\":\"{expected}\",\"width\":{},\"height\":{},\"sy\":{},\"params\":{},\"boxes\":[{}]}}",
                case.name,
                frame.width,
                frame.height,
                case.sy,
                params_json(&case.params),
                boxes.join(",")
            )?;
        }
    }
    json.push_str("]}\n");
    fs::write(out_dir.join("cases.json"), json)?;
    eprintln!("reference frames written to {}", out_dir.display());
    Ok(())
}
