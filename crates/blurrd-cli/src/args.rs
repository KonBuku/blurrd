//! Command-line arguments.

use std::path::PathBuf;
use std::str::FromStr;

use blurrd_core::{
    AudioMode, Codec, EncoderSetting, NvencSessions, RateControl, Region, RegionSet, Strength,
};
use clap::{Args, Parser, Subcommand, ValueEnum};

/// blurrd blurs player names in gameplay recordings.
///
/// It blurs names in the supported HUD regions (kill feed, ACE/clutch
/// banner, voice-chat speaker, text-chat sender) and on the scoreboard, the
/// death recap, the "Spectating" banner and the match-end/MVP screens.
/// Name tags above players' heads in the game world are out of scope.
#[derive(Debug, Parser)]
#[command(name = "blurrd", version, about, long_about)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
    #[command(flatten)]
    pub(crate) global: Global,
}

/// Options for every command.
#[derive(Debug, Args)]
pub(crate) struct Global {
    /// Write machine-readable output: one JSON object per line on stdout
    /// (job events, then a final report) and JSON logs on stderr.
    #[arg(long, global = true)]
    pub(crate) json: bool,
    /// Log level on stderr: error, warn, info, debug or trace.
    #[arg(long, global = true, default_value = "warn", value_name = "LEVEL")]
    pub(crate) log_level: String,
    /// Decoder threads (default: chosen by the decoder).
    #[arg(long, global = true, value_name = "N")]
    pub(crate) threads: Option<u32>,
    /// Use no GPU: software decoding, and Media Foundation instead of
    /// NVENC, AMF or Quick Sync for encoding.
    #[arg(long, global = true)]
    pub(crate) no_gpu: bool,
}

/// The commands.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Show what blurrd knows about a recording: streams, timing, colour
    /// tags, keyframes.
    Probe(ProbeArgs),
    /// Find the names in a recording and save the project (the boxes to
    /// blur) in the cache, optionally also as a file.
    Analyze(AnalyzeArgs),
    /// Blur a recording with an analysed project and export it.
    Render(RenderArgs),
    /// Analyse and blur recordings in one pass and export them.
    Run(RunArgs),
    /// Read a rendered output with OCR and report every name that is still
    /// readable; an output with one is renamed `*_UNVERIFIED`.
    Audit(AuditArgs),
}

/// Arguments of `blurrd probe`.
#[derive(Debug, Args)]
pub(crate) struct ProbeArgs {
    /// The recording.
    pub(crate) input: PathBuf,
}

/// Which names to find.
#[derive(Debug, Args)]
pub(crate) struct DetectArgs {
    /// Regions to blur, comma-separated: killfeed, banner, voice, chat,
    /// scoreboard, recap, spectating, matchend, or all (every supported
    /// region). Name tags above players' heads are out of scope.
    #[arg(long, default_value = "all", value_parser = parse_regions, value_name = "LIST")]
    pub(crate) regions: RegionSet,
    /// Skip the OCR verification (the result stays unverified).
    #[arg(long)]
    pub(crate) no_verify: bool,
}

/// Arguments of `blurrd analyze`.
#[derive(Debug, Args)]
pub(crate) struct AnalyzeArgs {
    /// The recording.
    pub(crate) input: PathBuf,
    #[command(flatten)]
    pub(crate) detect: DetectArgs,
    /// Also write the project to this file (`.blurrd.json`), for
    /// `blurrd render --plan`.
    #[arg(short, long, value_name = "FILE")]
    pub(crate) output: Option<PathBuf>,
}

/// Arguments of `blurrd render`.
#[derive(Debug, Args)]
pub(crate) struct RenderArgs {
    /// The recording.
    pub(crate) input: PathBuf,
    /// The project file (`.blurrd.json`) with the boxes to blur.
    #[arg(long, value_name = "FILE")]
    pub(crate) plan: PathBuf,
    /// The output file; its extension picks the container (.mp4, .mov or
    /// .mkv). An existing file is replaced; the input never is.
    #[arg(short, long, value_name = "FILE")]
    pub(crate) output: PathBuf,
    #[command(flatten)]
    pub(crate) export: ExportArgs,
}

/// Arguments of `blurrd run`.
#[derive(Debug, Args)]
pub(crate) struct RunArgs {
    /// The recordings.
    #[arg(required = true)]
    pub(crate) inputs: Vec<PathBuf>,
    /// The output file, or a folder for `<name>_blurrd.<ext>` files (a
    /// folder when there are several recordings).
    #[arg(short, long, value_name = "FILE|DIR")]
    pub(crate) output: PathBuf,
    #[command(flatten)]
    pub(crate) detect: DetectArgs,
    #[command(flatten)]
    pub(crate) export: ExportArgs,
}

/// Arguments of `blurrd audit`.
#[derive(Debug, Args)]
pub(crate) struct AuditArgs {
    /// The recording the output was rendered from.
    pub(crate) input: PathBuf,
    /// The project file (`.blurrd.json`) the output was rendered with.
    #[arg(long, value_name = "FILE")]
    pub(crate) plan: PathBuf,
    /// The rendered output.
    #[arg(short, long, value_name = "FILE")]
    pub(crate) output: PathBuf,
    /// Read every frame (default: one frame in six, as the verification's
    /// dictionary pass).
    #[arg(long)]
    pub(crate) every_frame: bool,
}

/// How to export.
#[derive(Debug, Args)]
pub(crate) struct ExportArgs {
    /// Output codec.
    #[arg(long, value_enum, default_value_t = CodecArg::Source)]
    pub(crate) codec: CodecArg,
    /// Encoder: `auto` (NVENC, AMF, Quick Sync, then Media Foundation) or an
    /// FFmpeg encoder name such as `h264_nvenc` or `h264_mf`.
    #[arg(long, default_value = "auto", value_parser = parse_encoder)]
    pub(crate) encoder: EncoderSetting,
    /// Quality: `source` (match the source bit rate, peaks up to 1.5 times
    /// it), `cq:N` (constant quality, 0-51, lower is better) or `vbr:M`
    /// (M Mbit/s average, peaks up to 1.5 times it).
    #[arg(long, default_value = "source", value_parser = parse_quality)]
    pub(crate) quality: RateControl,
    /// Encoder preset from p1 (fastest) to p7 (best quality).
    #[arg(long, default_value = "p4", value_parser = parse_preset)]
    pub(crate) preset: u8,
    /// Parallel NVENC sessions for H.264: auto, 1 or 2.
    #[arg(long, default_value = "auto", value_parser = parse_sessions)]
    pub(crate) sessions: NvencSessions,
    /// Audio: `copy`, `aac:KBPS` (re-encode) or `none`.
    #[arg(long, default_value = "copy", value_parser = parse_audio)]
    pub(crate) audio: AudioMode,
    /// Blur strength (for render: overrides the project's).
    #[arg(long, value_enum)]
    pub(crate) strength: Option<StrengthArg>,
    /// Name the output normally even if the project is not verified.
    #[arg(long)]
    pub(crate) accept_unverified: bool,
}

/// Output codecs on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum CodecArg {
    /// The source's codec (H.264, HEVC or AV1; others become H.264).
    Source,
    /// H.264 / AVC.
    H264,
    /// H.265 / HEVC.
    Hevc,
    /// AV1.
    Av1,
}

impl From<CodecArg> for Codec {
    fn from(codec: CodecArg) -> Self {
        match codec {
            CodecArg::Source => Self::Source,
            CodecArg::H264 => Self::H264,
            CodecArg::Hevc => Self::Hevc,
            CodecArg::Av1 => Self::Av1,
        }
    }
}

/// Blur strengths on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum StrengthArg {
    /// 8 reference pixel blocks.
    Low,
    /// 10 reference pixel blocks.
    Medium,
    /// 14 reference pixel blocks.
    High,
}

impl From<StrengthArg> for Strength {
    fn from(strength: StrengthArg) -> Self {
        match strength {
            StrengthArg::Low => Self::Low,
            StrengthArg::Medium => Self::Medium,
            StrengthArg::High => Self::High,
        }
    }
}

/// Parses `--regions`: `all` or comma-separated region keys.
fn parse_regions(text: &str) -> Result<RegionSet, String> {
    if text.trim() == "all" {
        return Ok(RegionSet::all());
    }
    let mut set = RegionSet::empty();
    for key in text.split(',').map(str::trim).filter(|k| !k.is_empty()) {
        let region = RegionSet::all()
            .iter()
            .find(|r| r.key() == key)
            .ok_or_else(|| {
                let keys: Vec<&str> = RegionSet::all().iter().map(Region::key).collect();
                format!("`{key}` is not a region; use {} or all", keys.join(", "))
            })?;
        set.insert(region);
    }
    if set.is_empty() {
        return Err(String::from("no region given"));
    }
    Ok(set)
}

/// Peak bit rate as a multiple of the average for `source` and `vbr:M`.
const PEAK_FACTOR: f32 = 1.5;

fn parse_encoder(text: &str) -> Result<EncoderSetting, String> {
    match text.trim() {
        "" => Err(String::from("the encoder name is empty")),
        "auto" => Ok(EncoderSetting::Auto),
        name => Ok(EncoderSetting::Named(name.to_owned())),
    }
}

fn parse_quality(text: &str) -> Result<RateControl, String> {
    let number =
        |value: &str| u32::from_str(value).map_err(|_| format!("`{value}` is not a number"));
    match text.split_once(':') {
        None if text == "source" => Ok(RateControl::MatchSource {
            max_factor: PEAK_FACTOR,
        }),
        Some(("cq", level)) => {
            let level = number(level)?;
            u8::try_from(level)
                .ok()
                .filter(|&l| l <= 51)
                .map(RateControl::Cq)
                .ok_or_else(|| format!("constant quality {level} is outside 0-51"))
        }
        Some(("vbr", mbits)) => {
            let target_kbps = number(mbits)?
                .checked_mul(1000)
                .filter(|&k| k > 0)
                .ok_or_else(|| format!("`{mbits}` Mbit/s is not a usable bit rate"))?;
            Ok(RateControl::Vbr {
                target_kbps,
                max_kbps: (target_kbps as f32 * PEAK_FACTOR) as u32,
            })
        }
        _ => Err(format!(
            "`{text}` is not a quality; use source, cq:N or vbr:M"
        )),
    }
}

fn parse_preset(text: &str) -> Result<u8, String> {
    text.strip_prefix('p')
        .and_then(|n| u8::from_str(n).ok())
        .filter(|n| (1..=7).contains(n))
        .ok_or_else(|| format!("`{text}` is not a preset; use p1 to p7"))
}

fn parse_sessions(text: &str) -> Result<NvencSessions, String> {
    match text {
        "auto" => Ok(NvencSessions::Auto),
        "1" => Ok(NvencSessions::One),
        "2" => Ok(NvencSessions::Two),
        _ => Err(format!("`{text}` is not a session count; use auto, 1 or 2")),
    }
}

fn parse_audio(text: &str) -> Result<AudioMode, String> {
    match text.split_once(':') {
        None if text == "copy" => Ok(AudioMode::Copy),
        None if text == "none" => Ok(AudioMode::Remove),
        Some(("aac", kbps)) => u16::from_str(kbps)
            .ok()
            .filter(|&k| (32..=512).contains(&k))
            .map(AudioMode::Aac)
            .ok_or_else(|| format!("`{kbps}` kbit/s is outside 32-512")),
        _ => Err(format!(
            "`{text}` is not an audio mode; use copy, aac:KBPS or none"
        )),
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::{AudioMode, EncoderSetting, NvencSessions, RateControl, Region, RegionSet};
    use clap::CommandFactory;

    use super::{
        Cli, parse_audio, parse_encoder, parse_preset, parse_quality, parse_regions, parse_sessions,
    };

    #[test]
    fn the_command_line_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_every_region_key() {
        assert_eq!(parse_regions("all"), Ok(RegionSet::all()));
        let keys = "killfeed,banner,voice,chat,scoreboard,recap,spectating,matchend";
        assert_eq!(parse_regions(keys), Ok(RegionSet::all()));
        for region in RegionSet::all().iter() {
            let one: RegionSet = [region].into_iter().collect();
            assert_eq!(parse_regions(region.key()), Ok(one));
        }
        assert_eq!(
            parse_regions("killfeed, chat"),
            Ok([Region::KillFeed, Region::TextChat].into_iter().collect())
        );
        for bad in ["", ",", "names", "killfeed,manual", "world"] {
            assert!(parse_regions(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn parses_quality_settings() {
        assert_eq!(
            parse_quality("source"),
            Ok(RateControl::MatchSource { max_factor: 1.5 })
        );
        assert_eq!(parse_quality("cq:19"), Ok(RateControl::Cq(19)));
        assert_eq!(
            parse_quality("vbr:30"),
            Ok(RateControl::Vbr {
                target_kbps: 30_000,
                max_kbps: 45_000
            })
        );
        for bad in ["cq:52", "cq:x", "vbr:0", "best", "vbr:"] {
            assert!(parse_quality(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn parses_the_other_settings() {
        assert_eq!(parse_preset("p1"), Ok(1));
        assert_eq!(parse_preset("p7"), Ok(7));
        assert!(parse_preset("p8").is_err() && parse_preset("4").is_err());
        assert_eq!(parse_sessions("2"), Ok(NvencSessions::Two));
        assert!(parse_sessions("3").is_err());
        assert_eq!(parse_audio("aac:256"), Ok(AudioMode::Aac(256)));
        assert_eq!(parse_audio("none"), Ok(AudioMode::Remove));
        assert!(parse_audio("aac:8").is_err() && parse_audio("mp3").is_err());
        assert_eq!(parse_encoder("auto"), Ok(EncoderSetting::Auto));
        assert_eq!(
            parse_encoder("h264_mf"),
            Ok(EncoderSetting::Named(String::from("h264_mf")))
        );
    }
}
