//! Bringing 8-bit 4:2:0 frames to the detectors' reference scale exactly
//! as FFmpeg does.
//!
//! The detector thresholds were tuned, and the OCR leak sweep validates
//! them, on frames that FFmpeg 7.1's `scale` filter brought to the
//! reference scale (`scale=W:H:flags=bicubic`, then `format=rgb24`). A
//! recording at another size has to reach the detectors through the same
//! arithmetic, or names that the validated path keeps become readable.
//! [`SwsScaler`] is an integer port of that swscale configuration
//! (`libswscale` n7.1.1: `initFilter` for `SWS_BICUBIC` with the default
//! parameters, the 8-to-15-bit horizontal scaler, the C vertical scaler
//! of packed 24-bit RGB output with its conversion tables), so its output
//! is byte-identical to FFmpeg's (checked against the `ffmpeg` command
//! line on real recordings and pinned by the tests below).

#![allow(
    clippy::many_single_char_names,
    reason = "the port keeps swscale's names (d, dd, y, u, v) so it can be read against the C"
)]

use blurrd_blur::{SampleRange, YuvMatrix};
use blurrd_video::RgbImage;

/// The bicubic `C` parameter swscale uses by default (`0.6` in 24-bit
/// fixed point, truncated as C truncates it).
const BICUBIC_C: i64 = 10_066_329;
/// Near-zero coefficients at the ends of a filter that add up to at most
/// this share of the filter's weight are dropped (`SWS_MAX_REDUCE_CUTOFF`).
const REDUCE_CUTOFF: f64 = 0.002;
/// Head room of the luma table for filter overshoot
/// (`YUVRGB_TABLE_LUMA_HEADROOM`).
const LUMA_HEADROOM: i64 = 512;

/// C integer division (truncates toward zero; Rust's `/` does the same).
const fn c_div(a: i64, b: i64) -> i64 {
    a / b
}

/// FFmpeg's `ROUNDED_DIV`.
const fn rounded_div(a: i64, b: i64) -> i64 {
    if a >= 0 {
        (a + (b >> 1)) / b
    } else {
        (a - (b >> 1)) / b
    }
}

/// FFmpeg's `av_log2` (0 for 0).
fn av_log2(v: i64) -> i64 {
    if v > 0 {
        63 - i64::from(v.leading_zeros())
    } else {
        0
    }
}

/// swscale's `get_local_pos` for an unset chroma siting: the position of
/// the first sample of a plane subsampled by `1 << subsample`, in 1/256
/// samples of that plane's grid.
///
/// FFmpeg 7.1's `scale` filter leaves the siting unset (`-513`) whatever
/// chroma location the frames carry, so swscale treats every source as
/// centre-sited; the validated frames were made that way.
const fn unset_local_pos(subsample: u32) -> i64 {
    let pos = (128 << subsample) - 128;
    (pos + 128) >> subsample
}

/// One axis of a separable filter: per output sample, the first source
/// sample and `size` coefficients.
#[derive(Debug, Clone)]
struct Filter {
    size: usize,
    pos: Vec<usize>,
    coeffs: Vec<i16>,
}

impl Filter {
    /// swscale's `initFilter` for a bicubic filter of `dst` samples from
    /// `src`, increment `inc` (16.16 fixed point), aligned to `align` taps,
    /// normalised to `one`, with the given sitings.
    #[allow(
        clippy::too_many_lines,
        reason = "one function mirrors initFilter step for step, which is what keeps it auditable against the C"
    )]
    fn bicubic(
        inc: i64,
        src: i64,
        dst: i64,
        align: i64,
        one: i64,
        src_pos: i64,
        dst_pos: i64,
    ) -> Self {
        let fone: i64 = 1 << (54 - av_log2(src / dst).min(8));
        let dst_n = dst as usize;
        let (mut size, mut filter, mut pos): (usize, Vec<Vec<i64>>, Vec<i64>);
        if (inc - 0x10000).abs() < 10 && src_pos == dst_pos {
            size = 1;
            filter = vec![vec![fone]; dst_n];
            pos = (0..dst).collect();
        } else {
            let size_factor = 4;
            let mut s = if inc <= 1 << 16 {
                1 + size_factor
            } else {
                1 + (size_factor * src + dst - 1) / dst
            };
            s = s.min(src - 2).max(1);
            size = s as usize;
            filter = Vec::with_capacity(dst_n);
            pos = Vec::with_capacity(dst_n);
            let mut dst_in_src = ((dst_pos * inc) >> 7) - ((src_pos * 0x10000) >> 7);
            for _ in 0..dst {
                let first = c_div(dst_in_src - (s - 2) * (1 << 16), 1 << 17);
                pos.push(first);
                let mut row = Vec::with_capacity(size);
                for xx in (first..).take(size) {
                    let mut d = (xx * (1 << 17) - dst_in_src).abs() << 13;
                    if inc > 1 << 16 {
                        d = d * dst / src;
                    }
                    let coeff = if d >= 1 << 31 {
                        0
                    } else {
                        // The products reach about 2^61: i128 keeps the C
                        // int64 results without any risk of overflow.
                        let (d, b, c) = (i128::from(d), 0i128, i128::from(BICUBIC_C));
                        let dd = (d * d) >> 30;
                        let ddd = (dd * d) >> 30;
                        let k = if d < 1 << 30 {
                            (12 * (1 << 24) - 9 * b - 6 * c) * ddd
                                + (-18 * (1 << 24) + 12 * b + 6 * c) * dd
                                + (6 * (1 << 24) - 2 * b) * (1 << 30)
                        } else {
                            (-b - 6 * c) * ddd
                                + (6 * b + 30 * c) * dd
                                + (-12 * b - 48 * c) * d
                                + (8 * b + 24 * c) * (1 << 30)
                        };
                        let k = k / i128::from((1i64 << 54) / fone);
                        i64::try_from(k).unwrap_or(0)
                    };
                    row.push(coeff);
                }
                filter.push(row);
                dst_in_src += 2 * inc;
            }
        }
        // Drop near-zero coefficients at both ends.
        let cut = REDUCE_CUTOFF * fone as f64;
        let mut min_size = 0usize;
        for i in (0..dst_n).rev() {
            let mut keep = size;
            let mut cutoff = 0i64;
            for _ in 0..size {
                cutoff += filter[i][0].abs();
                if cutoff as f64 > cut || (i < dst_n - 1 && pos[i] >= pos[i + 1]) {
                    break;
                }
                filter[i].rotate_left(1);
                filter[i][size - 1] = 0;
                pos[i] += 1;
            }
            cutoff = 0;
            for j in (1..size).rev() {
                cutoff += filter[i][j].abs();
                if cutoff as f64 > cut {
                    break;
                }
                keep -= 1;
            }
            min_size = min_size.max(keep);
        }
        let mut align = align;
        if min_size == 1 && align == 2 {
            // The x86 vertical scaler's special case for one tap.
            align = 1;
        }
        let aligned = (min_size as i64 + align - 1) & !(align - 1);
        let new_size = aligned as usize;
        for row in &mut filter {
            row.resize(new_size.max(size), 0);
            row.truncate(new_size);
        }
        size = new_size;
        // Fold taps outside the source onto its edge samples.
        for (f, p) in filter.iter_mut().zip(&mut pos) {
            if *p < 0 {
                for j in 1..size {
                    let left = (j as i64 + *p).max(0) as usize;
                    f[left] += f[j];
                    f[j] = 0;
                }
                *p = 0;
            }
            if *p + size as i64 > src {
                let shift = *p + (size as i64 - src).min(0);
                let mut acc = 0;
                for j in (0..size).rev() {
                    if *p + j as i64 >= src {
                        acc += f[j];
                        f[j] = 0;
                    }
                }
                for j in (0..size).rev() {
                    f[j] = if (j as i64) < shift {
                        0
                    } else {
                        f[j - shift as usize]
                    };
                }
                *p -= shift;
                f[(src - 1 - *p) as usize] += acc;
            }
        }
        // Normalise to `one`, carrying the rounding error along the taps.
        let mut coeffs = Vec::with_capacity(dst_n * size);
        for f in &filter {
            let mut sum: i64 = f.iter().sum();
            sum = (sum + one / 2) / one;
            if sum == 0 {
                sum = 1;
            }
            let mut error = 0;
            for &c in f {
                let v = c + error;
                let int_v = rounded_div(v, sum);
                // Stored as int16_t, as swscale stores it.
                coeffs.push(int_v as i16);
                error = v - int_v * sum;
            }
        }
        Self {
            size,
            pos: pos.into_iter().map(|p| p.max(0) as usize).collect(),
            coeffs,
        }
    }

    /// `hScale8To15`: one row of 8-bit samples to 15-bit intermediates.
    fn horizontal(&self, src: &[u8], out: &mut [i16]) {
        for ((o, &p), c) in out
            .iter_mut()
            .zip(&self.pos)
            .zip(self.coeffs.chunks_exact(self.size))
        {
            let mut val = 0i32;
            for (&s, &k) in src[p..p + self.size].iter().zip(c) {
                val += i32::from(s) * i32::from(k);
            }
            // Stored as int16_t after the clip to 2^15 - 1.
            *o = (val >> 7).min((1 << 15) - 1) as i16;
        }
    }

    /// The vertical taps of output row `j`: first source row and
    /// coefficients.
    fn taps(&self, j: usize) -> (usize, &[i16]) {
        (
            self.pos[j],
            &self.coeffs[j * self.size..(j + 1) * self.size],
        )
    }
}

/// `ff_yuv2rgb_c_init_tables` for 24-bit output: the luma table and the
/// per-chroma offsets into it.
#[derive(Debug, Clone)]
struct Tables {
    luma: Vec<u8>,
    luma_offset: i32,
    r_v: [i32; 256],
    g_u: [i32; 256],
    g_v: [i32; 256],
    b_u: [i32; 256],
}

impl Tables {
    fn new(matrix: YuvMatrix, range: SampleRange) -> Self {
        // ff_yuv2rgb_coeffs: crv, cbu, cgu, cgv.
        let [crv, cbu, cgu, cgv]: [i64; 4] = match matrix {
            YuvMatrix::Bt709 => [117_489, 138_438, 13_975, 34_925],
            YuvMatrix::Bt2020 => [110_013, 140_363, 12_277, 42_626],
            // BT.601 (FFmpeg's default for unspecified matrices).
            _ => [104_597, 132_201, 25_675, 53_279],
        };
        let (mut crv, mut cbu, mut cgu, mut cgv) = (crv, cbu, -cgu, -cgv);
        let full = range == SampleRange::Full;
        let (mut cy, oy): (i64, i64) = if full {
            crv = crv * 224 / 255;
            cbu = cbu * 224 / 255;
            cgu = cgu * 224 / 255;
            cgv = cgv * 224 / 255;
            (1 << 16, 0)
        } else {
            ((1 << 16) * 255 / 219, 16 << 16)
        };
        // Contrast and saturation 1 << 16, brightness 0.
        cy = (cy * (1 << 16)) >> 16;
        let fix = |c: i64| c_div(c * (1 << 16) + 0x8000, cy.max(1));
        let (crv, cbu, cgu, cgv) = (fix(crv), fix(cbu), fix(cgu), fix(cgv));
        let size = 1024 + 2 * LUMA_HEADROOM;
        let mut yb = -(384 << 16) - LUMA_HEADROOM * cy - oy;
        let luma = (0..size)
            .map(|_| {
                let v = ((yb + 0x8000) >> 16).clamp(0, 255) as u8;
                yb += cy;
                v
            })
            .collect();
        let offsets = |inc: i64| -> [i32; 256] {
            std::array::from_fn(|v| (-(inc >> 9) + ((v as i64 * inc) >> 16)) as i32)
        };
        Self {
            luma,
            luma_offset: (if full { 384 } else { 326 }) + LUMA_HEADROOM as i32,
            r_v: offsets(crv),
            g_u: offsets(cgu),
            g_v: offsets(cgv),
            b_u: offsets(cbu),
        }
    }

    #[inline]
    fn at(&self, index: i32) -> u8 {
        self.luma[index.clamp(0, self.luma.len() as i32 - 1) as usize]
    }
}

/// swscale's bicubic `yuv420p` → `rgb24` scaler for one source and
/// output size (see the module documentation).
#[derive(Debug, Clone)]
pub(crate) struct SwsScaler {
    src: (usize, usize),
    dst: (usize, usize),
    luma_h: Filter,
    chroma_h: Filter,
    luma_v: Filter,
    chroma_v: Filter,
    tables: Tables,
    /// Horizontally scaled rows: luma, U, V.
    rows: [Vec<i16>; 3],
}

impl SwsScaler {
    /// A scaler from `src` to `dst` pixels (`width, height`, all even) for
    /// samples in `matrix` and `range`.
    pub(crate) fn new(
        src: (u32, u32),
        dst: (u32, u32),
        matrix: YuvMatrix,
        range: SampleRange,
    ) -> Self {
        let (sw, sh) = (i64::from(src.0), i64::from(src.1));
        let (dw, dh) = (i64::from(dst.0), i64::from(dst.1));
        let inc = |s: i64, d: i64| ((s << 16) + (d >> 1)) / d;
        let ceil_half = |v: i64| -((-v) >> 1);
        let (csw, csh) = (ceil_half(sw), ceil_half(sh));
        // Packed RGB output keeps chroma horizontally subsampled and
        // interpolates it vertically per output row.
        let (cdw, cdh) = (ceil_half(dw), dh);
        let luma_pos = unset_local_pos(0);
        let chroma_pos = unset_local_pos(1);
        let luma_h = Filter::bicubic(inc(sw, dw), sw, dw, 4, 1 << 14, luma_pos, luma_pos);
        let chroma_h = Filter::bicubic(inc(csw, cdw), csw, cdw, 4, 1 << 14, chroma_pos, chroma_pos);
        let luma_v = Filter::bicubic(inc(sh, dh), sh, dh, 2, 1 << 12, luma_pos, luma_pos);
        let chroma_v = Filter::bicubic(inc(csh, cdh), csh, cdh, 2, 1 << 12, chroma_pos, luma_pos);
        Self {
            src: (src.0 as usize, src.1 as usize),
            dst: (dst.0 as usize, dst.1 as usize),
            luma_h,
            chroma_h,
            luma_v,
            chroma_v,
            tables: Tables::new(matrix, range),
            rows: Default::default(),
        }
    }

    /// Scales planar 8-bit 4:2:0 samples (`y` with `width` samples per
    /// row, `u` and `v` with `width / 2`) into `out`.
    pub(crate) fn scale(&mut self, y: &[u8], u: &[u8], v: &[u8], out: &mut RgbImage) {
        let (sw, sh) = self.src;
        let (dw, dh) = self.dst;
        let (csw, csh) = (sw.div_ceil(2), sh.div_ceil(2));
        let cdw = dw.div_ceil(2);
        let [rows_y, rows_u, rows_v] = &mut self.rows;
        rows_y.resize(sh * dw, 0);
        rows_u.resize(csh * cdw, 0);
        rows_v.resize(csh * cdw, 0);
        for (src, dst) in y.chunks_exact(sw).zip(rows_y.chunks_exact_mut(dw)) {
            self.luma_h.horizontal(src, dst);
        }
        for (src, dst) in u.chunks_exact(csw).zip(rows_u.chunks_exact_mut(cdw)) {
            self.chroma_h.horizontal(src, dst);
        }
        for (src, dst) in v.chunks_exact(csw).zip(rows_v.chunks_exact_mut(cdw)) {
            self.chroma_h.horizontal(src, dst);
        }
        out.width = dw as u32;
        out.height = dh as u32;
        out.data.resize(3 * dw * dh, 0);
        let mut luma = vec![0i32; dw];
        let (mut cu, mut cv) = (vec![0i32; cdw], vec![0i32; cdw]);
        let vertical =
            |rows: &[i16], width: usize, (first, taps): (usize, &[i16]), acc: &mut [i32]| {
                acc.fill(1 << 18);
                for (k, &c) in taps.iter().enumerate() {
                    let row = &rows[(first + k) * width..(first + k + 1) * width];
                    for (a, &s) in acc.iter_mut().zip(row) {
                        *a = a.wrapping_add(i32::from(s) * i32::from(c));
                    }
                }
                for a in acc.iter_mut() {
                    *a >>= 19;
                }
            };
        let t = &self.tables;
        for (j, line) in out.data.chunks_exact_mut(3 * dw).enumerate() {
            vertical(rows_y, dw, self.luma_v.taps(j), &mut luma);
            vertical(rows_u, cdw, self.chroma_v.taps(j), &mut cu);
            vertical(rows_v, cdw, self.chroma_v.taps(j), &mut cv);
            for (i, px) in line.as_chunks_mut::<6>().0.iter_mut().enumerate() {
                let (u, v) = (cu[i].clamp(0, 255) as usize, cv[i].clamp(0, 255) as usize);
                let (r, g, b) = (t.r_v[v], t.g_u[u] + t.g_v[v], t.b_u[u]);
                let (y0, y1) = (luma[2 * i] + t.luma_offset, luma[2 * i + 1] + t.luma_offset);
                *px = [
                    t.at(y0 + r),
                    t.at(y0 + g),
                    t.at(y0 + b),
                    t.at(y1 + r),
                    t.at(y1 + g),
                    t.at(y1 + b),
                ];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use blurrd_blur::{SampleRange, YuvMatrix};
    use blurrd_video::RgbImage;

    use super::SwsScaler;

    /// A deterministic 4:2:0 test picture with edges, gradients and noise.
    fn picture(w: usize, h: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let mut state = 0x2545_f491_u32;
        let mut noise = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 24) as u8
        };
        let y = (0..w * h)
            .map(|i| {
                let (x, yy) = (i % w, i / w);
                let base = if (x / 7 + yy / 5) % 2 == 0 { 30 } else { 220 };
                base / 2 + (noise() / 2)
            })
            .collect();
        let u = (0..w / 2 * h / 2).map(|_| noise()).collect();
        let v = (0..w / 2 * h / 2).map(|i| (i % 256) as u8).collect();
        (y, u, v)
    }

    fn digest(image: &RgbImage) -> String {
        use std::fmt::Write;

        use sha2::Digest;
        sha2::Sha256::digest(&image.data)
            .iter()
            .fold(String::new(), |mut hex, b| {
                let _ = write!(hex, "{b:02x}");
                hex
            })
    }

    #[test]
    fn matches_ffmpeg_on_up_and_downscales() {
        // Expected digests: SHA-256 of `ffmpeg 7.1.1 -f rawvideo -pix_fmt
        // yuv420p -color_range R -s WxH -i picture.yuv -vf scale=DW:DH:
        // flags=bicubic:in_color_matrix=bt709,format=rgb24 -f rawvideo` on
        // the same picture. The ratios are those of 720p, 900p, 1440p,
        // 2160p and 1366 x 768 recordings to 1080p, plus a horizontal-only
        // scale (vertical identity) and a full-range source.
        for (src, dst, range, expected) in [
            (
                (128, 72),
                (192, 108),
                SampleRange::Limited,
                "92a058867c2d6dccbe97f909ed2fc01b836e4e1f725a75a481137fd43dfc3039",
            ),
            (
                (160, 90),
                (192, 108),
                SampleRange::Limited,
                "122d41d732150ed556005804768345a21e5c5f9b725c7380f641813c2f6591a0",
            ),
            (
                (256, 144),
                (192, 108),
                SampleRange::Limited,
                "3bd33864fb5313ef968f5f63dd19b7f90f3a3b33ccd8c8274b2b8a2f30b6e67e",
            ),
            (
                (384, 216),
                (192, 108),
                SampleRange::Limited,
                "09ceda4d86bdf2b559d4b650b26a28eeca4b4f9bb841bd6584acd4cd8700bbc7",
            ),
            (
                (342, 192),
                (192, 108),
                SampleRange::Limited,
                "6fad43772dd3c5b0114649ba6f6ca32249294dbdf4ece868f30e9996ee17ef75",
            ),
            (
                (144, 108),
                (192, 108),
                SampleRange::Limited,
                "a9ea0f6c8506b05ab3f1fa7e4a5dd2322c5270919025faef5ca22c19fbefb4f2",
            ),
            (
                (256, 144),
                (192, 108),
                SampleRange::Full,
                "6d16f20ab8693cdca5b4ccbd872c22ce18536cc264b31350091aad93036271ff",
            ),
        ] {
            let (y, u, v) = picture(src.0, src.1);
            let mut scaler = SwsScaler::new(
                (src.0 as u32, src.1 as u32),
                (dst.0 as u32, dst.1 as u32),
                YuvMatrix::Bt709,
                range,
            );
            let mut out = RgbImage::default();
            scaler.scale(&y, &u, &v, &mut out);
            assert_eq!((out.width, out.height), (dst.0 as u32, dst.1 as u32));
            assert_eq!(digest(&out), expected, "{src:?} -> {dst:?} {range:?}");
        }
    }

    /// Runs the `ffmpeg` command line `ffmpeg` with `args`; returns its
    /// standard output.
    fn ffmpeg(ffmpeg: &std::ffi::OsStr, args: &[&std::ffi::OsStr]) -> Vec<u8> {
        let output = std::process::Command::new(ffmpeg)
            .args(["-v", "error", "-nostdin"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    #[test]
    #[ignore = "needs BLURRD_TEST_MEDIA and BLURRD_REFERENCE_FFMPEG (FFmpeg 7.1)"]
    fn matches_ffmpeg_on_real_recordings() {
        // Real recording content at the sizes players record at, made by
        // FFmpeg (exact 4:2:0 samples, no encoder in between), brought to
        // 1080p by the scaler and by the FFmpeg 7.1 command line the
        // detectors were validated with: the bytes must be equal. Only
        // pass/fail and sizes are printed.
        use blurrd_testkit::media::{Manifest, MediaDir};
        use std::ffi::OsStr;

        let (Some(media), Some(reference)) = (
            MediaDir::from_env(),
            std::env::var_os("BLURRD_REFERENCE_FFMPEG"),
        ) else {
            eprintln!("skipped: BLURRD_TEST_MEDIA or BLURRD_REFERENCE_FFMPEG is not set");
            return;
        };
        let manifest = Manifest::load_repository().unwrap();
        let clip = media.clip_path(&manifest.clips[0]);
        let raw = std::env::temp_dir().join(format!("blurrd-sws-{}.yuv", std::process::id()));
        let frames = 4;
        let rgb = 1920 * 1080 * 3;
        for (w, h) in [
            (1280, 720),
            (1366, 768),
            (1600, 900),
            (2560, 1440),
            (3840, 2160),
        ] {
            let size = format!("{w}x{h}");
            let make = format!("scale={w}:{h}:flags=bicubic,format=yuv420p");
            let source = ffmpeg(
                &reference,
                &[
                    OsStr::new("-ss"),
                    OsStr::new("20"),
                    OsStr::new("-i"),
                    clip.as_os_str(),
                    OsStr::new("-frames:v"),
                    OsStr::new(&frames.to_string()),
                    OsStr::new("-vf"),
                    OsStr::new(&make),
                    OsStr::new("-f"),
                    OsStr::new("rawvideo"),
                    OsStr::new("-"),
                ],
            );
            std::fs::write(&raw, &source).unwrap();
            let expected = ffmpeg(
                &reference,
                &[
                    OsStr::new("-f"),
                    OsStr::new("rawvideo"),
                    OsStr::new("-pix_fmt"),
                    OsStr::new("yuv420p"),
                    OsStr::new("-color_range"),
                    OsStr::new("tv"),
                    OsStr::new("-s"),
                    OsStr::new(&size),
                    OsStr::new("-i"),
                    raw.as_os_str(),
                    OsStr::new("-vf"),
                    OsStr::new("scale=1920:1080:flags=bicubic:in_color_matrix=bt709,format=rgb24"),
                    OsStr::new("-f"),
                    OsStr::new("rawvideo"),
                    OsStr::new("-"),
                ],
            );
            let (luma, chroma) = (w * h, w / 2 * (h / 2));
            let picture = luma + 2 * chroma;
            assert_eq!(source.len(), frames * picture, "{size}");
            assert_eq!(expected.len(), frames * rgb, "{size}");
            let mut scaler = SwsScaler::new(
                (w as u32, h as u32),
                (1920, 1080),
                YuvMatrix::Bt709,
                SampleRange::Limited,
            );
            let mut out = RgbImage::default();
            for (k, (yuv, reference)) in source
                .chunks_exact(picture)
                .zip(expected.chunks_exact(rgb))
                .enumerate()
            {
                let (y, uv) = yuv.split_at(luma);
                let (u, v) = uv.split_at(chroma);
                scaler.scale(y, u, v, &mut out);
                assert!(
                    out.data == reference,
                    "{size} frame {k} differs from FFmpeg"
                );
            }
            eprintln!("{size}: {frames} frames identical to FFmpeg");
        }
        let _ = std::fs::remove_file(&raw);
    }
}
