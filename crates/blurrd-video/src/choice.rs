//! Choosing a decoder per video from a cached micro-benchmark.
//!
//! Software decoding of 8-bit H.264 is faster than NVDEC on a fast CPU
//! (about 1000 against 700 frames per second at 1080p), but it occupies
//! every core; HEVC, AV1 and 10-bit video decode much faster on the GPU.
//! The first time a combination of codec, picture size, bit depth, GPU,
//! driver and FFmpeg version is seen, both paths decode the start of the
//! file for at most half a second each and the result is cached in
//! `caps/decoders.json`.

use std::path::Path;
use std::time::{Duration, Instant};

use blurrd_core::MediaInfo;
use serde::{Deserialize, Serialize};

use crate::sys::gpu;
use crate::{DecodeOptions, Decoder, HwDecode};

/// Schema name of the cache file.
const SCHEMA: &str = "blurrd-decoders/1";
/// File name of the cache inside the cache folder.
const CACHE_FILE: &str = "decoders.json";
/// Time budget per decoder in the benchmark.
const BUDGET: Duration = Duration::from_millis(500);
/// Frames per decoder in the benchmark (after the first frame).
const MAX_FRAMES: u32 = 240;
/// Entries kept in the cache; the oldest are dropped first.
const MAX_ENTRIES: usize = 64;

/// What the decoded frames are for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DecodePurpose {
    /// Decode as fast as possible.
    Throughput,
    /// Other work (detectors, OCR) needs the CPU: prefer the GPU decoder
    /// whenever it reaches at least half the software speed.
    SpareCpu,
}

/// One measured combination.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Entry {
    codec: String,
    width: u32,
    height: u32,
    bit_depth: u8,
    /// LUID of the first (high-performance) GPU, empty without one.
    gpu: String,
    driver: String,
    ffmpeg: String,
    software_fps: Option<f64>,
    hardware: Option<String>,
    hardware_fps: Option<f64>,
}

impl Entry {
    fn same_key(&self, other: &Self) -> bool {
        (
            &self.codec,
            self.width,
            self.height,
            self.bit_depth,
            &self.gpu,
            &self.driver,
            &self.ffmpeg,
        ) == (
            &other.codec,
            other.width,
            other.height,
            other.bit_depth,
            &other.gpu,
            &other.driver,
            &other.ffmpeg,
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    schema: String,
    entries: Vec<Entry>,
}

/// Picks the decoder for a video from a cached benchmark.
#[derive(Debug)]
pub struct DecoderChoice;

impl DecoderChoice {
    /// The decoder to use for `info`. Measures and caches on the first call
    /// for a combination; later calls only read `cache_dir/decoders.json`.
    /// Never fails: without a working GPU decoder (or on any error) the
    /// answer is [`HwDecode::Software`].
    pub fn resolve(info: &MediaInfo, cache_dir: &Path, purpose: DecodePurpose) -> HwDecode {
        let adapters = gpu::adapters();
        let first = adapters.first();
        let mut key = Entry {
            codec: info.video_codec.clone(),
            width: info.width,
            height: info.height,
            bit_depth: info.bit_depth,
            gpu: first.map(|a| a.info.luid.to_string()).unwrap_or_default(),
            driver: first.map(|a| a.info.driver.clone()).unwrap_or_default(),
            ffmpeg: crate::ffmpeg_version(),
            software_fps: None,
            hardware: None,
            hardware_fps: None,
        };
        let path = cache_dir.join(CACHE_FILE);
        let mut cache = read_cache(&path);
        if let Some(entry) = cache.iter().find(|e| e.same_key(&key)) {
            return decide(entry, purpose);
        }

        let hardware = match first {
            Some(adapter) if adapter.is_nvidia() => Some(HwDecode::Nvdec),
            Some(_) => Some(HwDecode::D3d11va),
            None => None,
        };
        key.software_fps = benchmark(&info.path, HwDecode::Software);
        if let Some(hw) = hardware {
            key.hardware = Some(hw_name(hw).to_owned());
            key.hardware_fps = benchmark(&info.path, hw);
        }
        tracing::info!(
            codec = %key.codec,
            software_fps = ?key.software_fps,
            hardware = ?key.hardware,
            hardware_fps = ?key.hardware_fps,
            "decoder benchmark"
        );
        let choice = decide(&key, purpose);
        cache.push(key);
        let excess = cache.len().saturating_sub(MAX_ENTRIES);
        cache.drain(..excess);
        if let Err(error) = write_cache(&path, cache) {
            tracing::warn!(%error, "could not write the decoder cache");
        }
        choice
    }
}

fn hw_name(hw: HwDecode) -> &'static str {
    match hw {
        HwDecode::Nvdec => "nvdec",
        HwDecode::D3d11va => "d3d11va",
        HwDecode::Auto => "auto",
        HwDecode::Software => "software",
    }
}

/// The decision rule, applied to a measured entry.
fn decide(entry: &Entry, purpose: DecodePurpose) -> HwDecode {
    let hardware = match entry.hardware.as_deref() {
        Some("nvdec") => HwDecode::Nvdec,
        Some("d3d11va") => HwDecode::D3d11va,
        _ => return HwDecode::Software,
    };
    let Some(hw_fps) = entry.hardware_fps else {
        return HwDecode::Software;
    };
    let Some(sw_fps) = entry.software_fps else {
        return hardware;
    };
    // HEVC, AV1 and 10-bit streams go to the GPU whenever it works (plan,
    // section 8); for the rest the measurement decides.
    let gpu_first = entry.bit_depth > 8 || matches!(entry.codec.as_str(), "hevc" | "av1");
    let threshold = match purpose {
        _ if gpu_first => 0.0,
        DecodePurpose::Throughput => 1.0,
        DecodePurpose::SpareCpu => 0.5,
    };
    if hw_fps >= sw_fps * threshold {
        hardware
    } else {
        HwDecode::Software
    }
}

/// Frames per second of `hw` on the start of the file, or `None` when it
/// cannot decode it (for hardware: when no frame came from the GPU).
fn benchmark(path: &Path, hw: HwDecode) -> Option<f64> {
    let mut decoder = Decoder::open(path, &DecodeOptions::with_hw(hw)).ok()?;
    // The first frame includes decoder start-up; time the rest.
    decoder.next_frame().ok()??;
    let start = Instant::now();
    let mut frames = 0u32;
    while frames < MAX_FRAMES && start.elapsed() < BUDGET {
        match decoder.next_frame() {
            Ok(Some(_)) => frames += 1,
            Ok(None) => break,
            Err(_) => return None,
        }
    }
    if hw != HwDecode::Software && decoder.hardware_frames() == 0 {
        return None;
    }
    let seconds = start.elapsed().as_secs_f64();
    (frames > 0 && seconds > 0.0).then(|| f64::from(frames) / seconds)
}

fn read_cache(path: &Path) -> Vec<Entry> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<CacheFile>(&bytes).ok())
        .filter(|file| file.schema == SCHEMA)
        .map(|file| file.entries)
        .unwrap_or_default()
}

fn write_cache(path: &Path, entries: Vec<Entry>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = CacheFile {
        schema: SCHEMA.to_owned(),
        entries,
    };
    let json = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::{DecodePurpose, Entry, decide};
    use crate::HwDecode;

    fn entry(codec: &str, bit_depth: u8, sw: Option<f64>, hw: Option<f64>) -> Entry {
        Entry {
            codec: codec.to_owned(),
            width: 1920,
            height: 1080,
            bit_depth,
            gpu: String::from("0000000000001234"),
            driver: String::from("32.0.15.7242"),
            ffmpeg: String::from("9.0.2"),
            software_fps: sw,
            hardware: Some(String::from("nvdec")),
            hardware_fps: hw,
        }
    }

    #[test]
    fn h264_goes_to_the_faster_decoder_unless_the_cpu_is_needed() {
        let e = entry("h264", 8, Some(1000.0), Some(700.0));
        assert_eq!(decide(&e, DecodePurpose::Throughput), HwDecode::Software);
        assert_eq!(decide(&e, DecodePurpose::SpareCpu), HwDecode::Nvdec);
        let slow_gpu = entry("h264", 8, Some(1000.0), Some(300.0));
        assert_eq!(
            decide(&slow_gpu, DecodePurpose::SpareCpu),
            HwDecode::Software
        );
    }

    #[test]
    fn hevc_av1_and_ten_bit_prefer_the_gpu() {
        for e in [
            entry("hevc", 8, Some(900.0), Some(600.0)),
            entry("av1", 8, Some(900.0), Some(600.0)),
            entry("h264", 10, Some(900.0), Some(100.0)),
        ] {
            assert_eq!(
                decide(&e, DecodePurpose::Throughput),
                HwDecode::Nvdec,
                "{}",
                e.codec
            );
        }
    }

    #[test]
    fn broken_hardware_falls_back_to_software() {
        let e = entry("hevc", 10, Some(300.0), None);
        assert_eq!(decide(&e, DecodePurpose::SpareCpu), HwDecode::Software);
        let mut no_gpu = entry("h264", 8, Some(300.0), None);
        no_gpu.hardware = None;
        assert_eq!(
            decide(&no_gpu, DecodePurpose::Throughput),
            HwDecode::Software
        );
    }

    #[test]
    fn keys_ignore_measurements() {
        let a = entry("h264", 8, Some(1.0), Some(2.0));
        let mut b = entry("h264", 8, Some(3.0), None);
        assert!(a.same_key(&b));
        b.driver = String::from("other");
        assert!(!a.same_key(&b));
    }
}
