//! The `blurrd` binary end to end: arguments, exit codes, JSON events,
//! output naming and project checks. CI renders with the software test
//! encoder; the `gpu_` tests (Media Foundation, NVENC fallback) run
//! locally with `cargo nextest run -p blurrd-cli --run-ignored all gpu`.

#![allow(clippy::unwrap_used, reason = "test code: an error is a test failure")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use blurrd_core::{
    BlurParams, DetectorInfo, Fingerprint, HudGeometry, MediaInfo, Mode, Override, Project,
    ProjectMedia, RectI, Region, RegionSet, Sensitivity, TemporalParams, VerifyState, schema,
};
use ffmpeg_the_third::{Rational as FfRational, codec, encoder, format, frame};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// A folder for one test, removed when dropped.
struct Workdir(PathBuf);

impl Workdir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("blurrd-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Workdir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Writes a 320×240 MPEG-4 clip of `frames` frames at 60 fps.
fn write_clip(path: &Path, frames: i64) {
    ffmpeg_the_third::init().unwrap();
    let mut out = format::output(path).unwrap();
    let codec = encoder::find_by_name("mpeg4").unwrap();
    let mut ctx = codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .unwrap();
    ctx.set_width(320);
    ctx.set_height(240);
    ctx.set_format(format::Pixel::YUV420P);
    ctx.set_time_base(FfRational::new(1, 60));
    ctx.set_gop(30);
    ctx.set_flags(codec::Flags::GLOBAL_HEADER);
    let mut enc = ctx.open().unwrap();
    let mut stream = out.add_stream(codec).unwrap();
    stream.set_parameters(codec::Parameters::from(&enc));
    stream.set_time_base(FfRational::new(1, 60));
    out.write_header().unwrap();
    let stream_tb = out.stream(0).unwrap().time_base();
    let mut picture = frame::Video::new(format::Pixel::YUV420P, 320, 240);
    let mut packet = codec::packet::Packet::empty();
    for pts in 0..=frames {
        if pts < frames {
            for plane in 0..3 {
                let value = if plane == 0 {
                    40 + (pts % 150) as u8
                } else {
                    128
                };
                picture.data_mut(plane).fill(value);
            }
            picture.set_pts(Some(pts));
            enc.send_frame(&picture).unwrap();
        } else {
            enc.send_eof().unwrap();
        }
        while enc.receive_packet(&mut packet).is_ok() {
            packet.set_stream(0);
            packet.rescale_ts(FfRational::new(1, 60), stream_tb);
            packet.write_interleaved(&mut out).unwrap();
        }
    }
    out.write_trailer().unwrap();
}

/// Hex SHA-256 of the clip's PTS table in the `pts.bin` format.
fn pts_sha256(clip: &Path) -> (u32, String) {
    let table = blurrd_video::pts_table(clip).unwrap();
    let hash = Sha256::digest(table.to_bytes())
        .iter()
        .fold(String::new(), |mut hex, b| {
            use std::fmt::Write as _;
            let _ = write!(hex, "{b:02x}");
            hex
        });
    (table.len(), hash)
}

/// Writes a project for `clip` with one manual box on frames 5 to 9.
fn write_project(clip: &Path, project_path: &Path, verified: bool, wrong_hash: bool) {
    let info: MediaInfo = blurrd_video::probe(clip).unwrap();
    let (frames, mut hash) = pts_sha256(clip);
    if wrong_hash {
        hash = "0".repeat(64);
    }
    let media = ProjectMedia {
        path: clip.to_path_buf(),
        fingerprint: Fingerprint {
            size: info.size,
            mtime: 0,
            blake3_head_tail: String::from("0"),
        },
        width: info.width,
        height: info.height,
        fps: info.fps,
        time_base: info.time_base,
        frame_count: frames,
        pts_sha256: hash,
        is_vfr: info.is_vfr,
        has_b_frames: info.has_b_frames,
        keyframes_sha256: String::from("0"),
    };
    let detector = DetectorInfo {
        version: String::from("test"),
        regions: RegionSet::all(),
        mode: Mode::Tight,
        sensitivity: Sensitivity::Balanced,
        temporal: TemporalParams::default(),
    };
    let mut project = Project::new(
        media,
        HudGeometry::default(),
        detector,
        BlurParams::default(),
    );
    project.overrides.push(Override::Add {
        uid: uuid::Uuid::from_u128(7),
        region: Region::Manual,
        f0: 5,
        f1: 9,
        rect: RectI::new(64, 64, 192, 112).unwrap(),
    });
    if verified {
        project.verify.state = VerifyState::Clean;
        project.verify.plan_revision = project.revision;
    }
    std::fs::write(project_path, schema::to_json_pretty(&project).unwrap()).unwrap();
}

/// Runs the binary with a private cache folder.
fn blurrd(dir: &Workdir, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_blurrd"));
    command.args(args).env("LOCALAPPDATA", dir.path("cache"));
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().unwrap()
}

/// The JSON lines of stdout.
fn json_lines(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn types(lines: &[Value]) -> Vec<String> {
    lines
        .iter()
        .map(|l| l["type"].as_str().unwrap().to_owned())
        .collect()
}

/// A clip and a project in a fresh folder.
fn setup(name: &str, verified: bool, wrong_hash: bool) -> (Workdir, PathBuf, PathBuf) {
    let dir = Workdir::new(name);
    let clip = dir.path("clip.mp4");
    write_clip(&clip, 90);
    let project = dir.path("clip.blurrd.json");
    write_project(&clip, &project, verified, wrong_hash);
    (dir, clip, project)
}

fn s(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn probe_prints_media_info() {
    let dir = Workdir::new("probe");
    let clip = dir.path("clip.mp4");
    write_clip(&clip, 30);
    let out = blurrd(&dir, &["probe", s(&clip), "--json"], &[]);
    assert!(out.status.success(), "{out:?}");
    let info: MediaInfo = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!((info.width, info.height), (320, 240));
    assert_eq!(info.keyframes.unwrap_or_default().first(), Some(&0));

    let text = blurrd(&dir, &["probe", s(&clip)], &[]);
    assert!(text.status.success());
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(stdout.contains("320x240"), "{stdout}");

    let missing = blurrd(&dir, &["probe", s(&dir.path("nope.mp4")), "--json"], &[]);
    assert_eq!(missing.status.code(), Some(1));
    let error = &json_lines(&missing)[0];
    assert_eq!(error["type"], "Error");
    assert_eq!(error["error"]["kind"], "NotFound");
}

#[test]
fn renders_unverified_projects_under_a_marked_name() {
    let (dir, clip, project) = setup("unverified", false, false);
    let output = dir.path("out.mp4");
    let args = [
        "render",
        s(&clip),
        "--plan",
        s(&project),
        "-o",
        s(&output),
        "--encoder",
        "mpeg4",
        "--json",
    ];
    let out = blurrd(&dir, &args, &[]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert!(!output.exists());
    assert!(dir.path("out_UNVERIFIED.mp4").is_file());
    let lines = json_lines(&out);
    let kinds = types(&lines);
    assert_eq!(kinds.first().map(String::as_str), Some("Stage"));
    assert!(kinds.contains(&String::from("Started")));
    assert!(kinds.contains(&String::from("Progress")));
    assert_eq!(kinds[kinds.len() - 2], "Finished");
    let report = lines.last().unwrap();
    assert_eq!(report["type"], "Report");
    assert_eq!(report["frames"], 90);
    assert_eq!(report["blurred_frames"], 5);
    assert_eq!(report["encoder"], "mpeg4");
    assert_eq!(report["verified"], false);
    // Every event belongs to the same job.
    let job = &lines[0]["job"];
    assert!(lines[..lines.len() - 1].iter().all(|l| &l["job"] == job));

    let accepted = blurrd(
        &dir,
        &[&args[..args.len() - 1], &["--accept-unverified"]].concat(),
        &[],
    );
    assert_eq!(accepted.status.code(), Some(0), "{accepted:?}");
    assert!(output.is_file());
    let summary = String::from_utf8_lossy(&accepted.stdout);
    assert!(summary.starts_with("wrote "), "{summary}");
}

#[test]
fn verified_projects_render_normally() {
    let (dir, clip, project) = setup("verified", true, false);
    let output = dir.path("out.mkv");
    let out = blurrd(
        &dir,
        &[
            "render",
            s(&clip),
            "--plan",
            s(&project),
            "-o",
            s(&output),
            "--encoder",
            "ffv1",
            "--strength",
            "high",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(output.is_file());
    assert_eq!(blurrd_video::pts_table(&output).unwrap().len(), 90);
}

#[test]
fn rejects_a_video_that_does_not_match_the_project() {
    let (dir, clip, project) = setup("mismatch", true, true);
    let output = dir.path("out.mp4");
    let out = blurrd(
        &dir,
        &[
            "render",
            s(&clip),
            "--plan",
            s(&project),
            "-o",
            s(&output),
            "--encoder",
            "mpeg4",
            "--json",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let failed = json_lines(&out).pop().unwrap();
    assert_eq!(failed["type"], "Failed");
    assert_eq!(failed["error"]["kind"], "ProjectMismatch");
    let leftovers: Vec<_> = std::fs::read_dir(&dir.0)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("out"))
        .collect();
    assert_eq!(leftovers, Vec::<String>::new());
}

#[test]
fn invalid_arguments_exit_with_2() {
    let dir = Workdir::new("usage");
    for args in [
        &["render", "clip.mp4", "-o", "out.mp4"][..],
        &[
            "render",
            "clip.mp4",
            "--plan",
            "p.json",
            "-o",
            "o.mp4",
            "--quality",
            "best",
        ],
        &[
            "render", "clip.mp4", "--plan", "p.json", "-o", "o.mp4", "--preset", "p9",
        ],
        &["frobnicate"],
    ] {
        let out = blurrd(&dir, args, &[]);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}

#[test]
#[ignore = "needs Windows Media Foundation with an H.264 encoder"]
fn gpu_renders_with_media_foundation() {
    let (dir, clip, project) = setup("mf", true, false);
    let output = dir.path("out.mp4");
    let out = blurrd(
        &dir,
        &[
            "render",
            s(&clip),
            "--plan",
            s(&project),
            "-o",
            s(&output),
            "--encoder",
            "h264_mf",
            "--json",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let report = json_lines(&out).pop().unwrap();
    assert_eq!(report["encoder"], "h264_mf");
    let info = blurrd_video::probe(&output).unwrap();
    assert_eq!(info.video_codec, "h264");
    assert_eq!(blurrd_video::pts_table(&output).unwrap().len(), 90);
}

#[test]
#[ignore = "needs an NVIDIA GPU"]
fn gpu_a_failing_nvenc_falls_back_with_a_warning() {
    let (dir, clip, project) = setup("fallback", true, false);
    let args = |output: &Path| {
        vec![
            String::from("render"),
            s(&clip).to_owned(),
            String::from("--plan"),
            s(&project).to_owned(),
            String::from("-o"),
            s(output).to_owned(),
            String::from("--json"),
        ]
    };
    // A normal render probes the encoders (NVENC works) and caches that.
    let first = dir.path("first.mp4");
    let normal = blurrd(
        &dir,
        &args(&first).iter().map(String::as_str).collect::<Vec<_>>(),
        &[],
    );
    assert_eq!(normal.status.code(), Some(0), "{normal:?}");
    assert_eq!(json_lines(&normal).pop().unwrap()["encoder"], "h264_nvenc");
    // Then CUDA sees no device, so NVENC fails to open although the cached
    // probe lists it: the render falls back and says so.
    let second = dir.path("second.mp4");
    let broken = blurrd(
        &dir,
        &args(&second).iter().map(String::as_str).collect::<Vec<_>>(),
        &[("CUDA_VISIBLE_DEVICES", "-1")],
    );
    assert_eq!(broken.status.code(), Some(0), "{broken:?}");
    let lines = json_lines(&broken);
    let warning = lines
        .iter()
        .find(|l| l["type"] == "Warning")
        .expect("a warning");
    assert_eq!(warning["code"], "EncoderFallback");
    assert!(warning["message"].as_str().unwrap().contains("h264_nvenc"));
    let report = lines.last().unwrap();
    assert_ne!(report["encoder"], "h264_nvenc");
    assert_eq!(report["fallbacks"][0]["from"], "h264_nvenc");
    assert!(second.is_file());
}

#[test]
#[ignore = "needs BLURRD_TEST_MEDIA and an NVIDIA GPU"]
fn real_media_render_end_to_end() {
    let media = std::env::var_os("BLURRD_TEST_MEDIA").expect("set BLURRD_TEST_MEDIA");
    let clip = PathBuf::from(media).join("VALORANT_replay_2024.06.22-11.50.mp4");
    let dir = Workdir::new("real");
    let project = dir.path("clip.blurrd.json");
    write_project(&clip, &project, true, false);
    let output = dir.path("out.mp4");
    let started = std::time::Instant::now();
    let out = blurrd(
        &dir,
        &[
            "render",
            s(&clip),
            "--plan",
            s(&project),
            "-o",
            s(&output),
            "--json",
        ],
        &[],
    );
    let wall = started.elapsed();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let report = json_lines(&out).pop().unwrap();
    println!(
        "CLI render: {} frames, encoder {} x{}, render {:.0} fps, wall clock {wall:?} \
         (probe, PTS scan, encoder setup included)",
        report["frames"],
        report["encoder"],
        report["sessions"],
        report["fps"].as_f64().unwrap()
    );
    assert_eq!(report["frames"], 3601);
    assert_eq!(report["moov"], "reserved");
}
