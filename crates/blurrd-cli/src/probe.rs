//! `blurrd probe`: what blurrd knows about a recording.

use std::fmt::Write as _;
use std::io::Write as _;

use blurrd_core::{AppError, ColorRange, MediaInfo};

use crate::args::ProbeArgs;
use crate::output::write_json;

/// Probes the input and prints the result: the [`MediaInfo`] as one JSON
/// line with `--json`, a summary otherwise.
pub(crate) fn probe(args: &ProbeArgs, json: bool) -> Result<(), AppError> {
    let info = blurrd_video::probe(&args.input)?;
    if json {
        write_json(&info);
    } else {
        let _ = std::io::stdout()
            .lock()
            .write_all(summary(&info).as_bytes());
    }
    Ok(())
}

/// A readable summary of `info`, one fact per line.
fn summary(info: &MediaInfo) -> String {
    let yes_no = |flag: bool| if flag { "yes" } else { "no" };
    let mut out = String::new();
    let mut line = |label: &str, value: String| {
        let _ = writeln!(out, "{label:<14}{value}");
    };
    line("file", info.path.display().to_string());
    line(
        "size",
        format!("{:.1} MB, {:.2} s", info.size as f64 / 1e6, info.duration_s),
    );
    line(
        "video",
        format!(
            "{} {} ({}-bit), {}x{}",
            info.video_codec, info.profile, info.bit_depth, info.width, info.height
        ),
    );
    let fps = f64::from(info.fps.num) / f64::from(info.fps.den.max(1));
    line(
        "frame rate",
        format!(
            "{fps:.3} fps ({}/{}){}",
            info.fps.num,
            info.fps.den,
            if info.is_vfr { ", variable" } else { "" }
        ),
    );
    line(
        "frames",
        info.frame_count
            .map_or_else(|| String::from("unknown"), |n| n.to_string()),
    );
    line(
        "keyframes",
        info.keyframes
            .as_ref()
            .map_or_else(|| String::from("unknown"), |k| k.len().to_string()),
    );
    let range = match info.color.range {
        ColorRange::Full => "full",
        ColorRange::Limited => "limited",
        _ => "unspecified",
    };
    line(
        "colour",
        format!(
            "primaries {}, transfer {}, matrix {}, {range} range{}",
            info.color.primaries,
            info.color.transfer,
            info.color.matrix,
            info.hdr.map_or("", |_| ", HDR")
        ),
    );
    line("b-frames", yes_no(info.has_b_frames).to_owned());
    line("edit list", yes_no(info.has_edit_list).to_owned());
    for audio in &info.audio {
        line(
            "audio",
            format!(
                "#{} {}, {} channels, {} Hz{}",
                audio.index,
                audio.codec,
                audio.channels,
                audio.sample_rate,
                audio
                    .language
                    .as_deref()
                    .map(|l| format!(", {l}"))
                    .unwrap_or_default()
            ),
        );
    }
    line("subtitles", yes_no(info.has_subtitles).to_owned());
    out
}
