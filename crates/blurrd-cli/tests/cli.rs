//! The `blurrd` binary end to end: arguments, exit codes, JSON events,
//! output naming and project checks. CI renders with the software test
//! encoder; the `gpu_` tests (Media Foundation, NVENC fallback) run
//! locally with `cargo nextest run -p blurrd-cli --run-ignored all gpu`.

#![allow(clippy::unwrap_used, reason = "test code: an error is a test failure")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use blurrd_core::{
    BlurParams, DetectorInfo, HudGeometry, MediaInfo, Mode, Override, Project, ProjectMedia, RectI,
    Region, RegionSet, Sensitivity, TemporalParams, VerifyState, schema,
};
use blurrd_engine::cache;
use ffmpeg_the_third::{Rational as FfRational, codec, encoder, format, frame};
use serde_json::Value;

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
    write_sized_clip(path, frames, 320, 240);
}

/// Writes a `width × height` MPEG-4 clip of `frames` frames at 60 fps.
fn write_sized_clip(path: &Path, frames: i64, width: u32, height: u32) {
    ffmpeg_the_third::init().unwrap();
    let mut out = format::output(path).unwrap();
    let codec = encoder::find_by_name("mpeg4").unwrap();
    let mut ctx = codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .unwrap();
    ctx.set_width(width);
    ctx.set_height(height);
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
    let mut picture = frame::Video::new(format::Pixel::YUV420P, width, height);
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

/// The frame count and PTS hash of the clip (as the project stores it).
fn pts_sha256(clip: &Path) -> (u32, String) {
    let table = blurrd_video::pts_table(clip).unwrap();
    (table.len(), cache::pts_sha256(&table))
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
        fingerprint: cache::fingerprint(clip).unwrap(),
        width: info.width,
        height: info.height,
        fps: info.fps,
        time_base: info.time_base,
        frame_count: frames,
        pts_sha256: hash,
        is_vfr: info.is_vfr,
        has_b_frames: info.has_b_frames,
        keyframes_sha256: cache::keyframes_sha256(info.keyframes.as_deref()),
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

    // Accepting names the output normally but does not make the result
    // verified.
    std::fs::remove_file(&output).unwrap();
    let accepted = blurrd(&dir, &[&args[..], &["--accept-unverified"]].concat(), &[]);
    assert_eq!(accepted.status.code(), Some(0), "{accepted:?}");
    let lines = json_lines(&accepted);
    let finished = lines.iter().find(|l| l["type"] == "Finished").unwrap();
    assert_ne!(finished["summary"]["verify"], "Clean");
    let report = lines.last().unwrap();
    assert_eq!(report["verified"], false);
    assert_eq!(report["accepted_unverified"], true);
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
fn analyze_render_and_run_a_recording() {
    let dir = Workdir::new("pipeline");
    let clip = dir.path("clip.mp4");
    write_sized_clip(&clip, 90, 640, 360);
    let plan = dir.path("plan.blurrd.json");
    let out = blurrd(
        &dir,
        &[
            "analyze",
            s(&clip),
            "--no-verify",
            "--regions",
            "killfeed,chat,scoreboard",
            "-o",
            s(&plan),
            "--json",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let lines = json_lines(&out);
    let kinds = types(&lines);
    assert!(kinds.contains(&String::from("Started")));
    assert_eq!(kinds[kinds.len() - 2], "Finished");
    let report = lines.last().unwrap();
    assert_eq!(report["type"], "Report");
    assert_eq!(report["frames"], 90);
    assert_eq!(report["plan"], s(&plan));
    let project = cache::read_project(&plan).unwrap();
    assert_eq!(project.media.frame_count, 90);
    assert_eq!(
        project.detector.regions,
        [Region::KillFeed, Region::TextChat, Region::Scoreboard]
            .into_iter()
            .collect()
    );
    // The project is also in the cache folder.
    assert!(
        report["project_file"]
            .as_str()
            .unwrap()
            .contains("projects")
    );

    let rendered = dir.path("rendered.mp4");
    let out = blurrd(
        &dir,
        &[
            "render",
            s(&clip),
            "--plan",
            s(&plan),
            "-o",
            s(&rendered),
            "--encoder",
            "mpeg4",
            "--accept-unverified",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(rendered.is_file());
}

/// Whether ONNX Runtime and the OCR models are installed (the audit needs
/// them).
fn ocr_installed() -> bool {
    let set = |var| std::env::var_os(var).is_some_and(|v| Path::new(&v).exists());
    set("ORT_DYLIB_PATH") && set("BLURRD_MODEL_DIR")
}

#[test]
fn audit_reads_a_rendered_output() {
    if !ocr_installed() {
        eprintln!("skipped: ONNX Runtime or the OCR models are not installed");
        return;
    }
    let (dir, clip, project) = setup("audit", true, false);
    let rendered = dir.path("rendered.mkv");
    let out = blurrd(
        &dir,
        &[
            "render",
            s(&clip),
            "--plan",
            s(&project),
            "-o",
            s(&rendered),
            "--encoder",
            "ffv1",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let out = blurrd(
        &dir,
        &[
            "audit",
            s(&clip),
            "--plan",
            s(&project),
            "-o",
            s(&rendered),
            "--no-gpu",
            "--json",
        ],
        &[],
    );
    // A clip without names: nothing is readable, the output keeps its
    // name.
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let lines = json_lines(&out);
    let report = lines.last().unwrap();
    assert_eq!(report["type"], "Report");
    assert_eq!(report["renamed"], false);
    assert_eq!(report["output"], s(&rendered));
    assert_eq!(report["report"]["summary"]["state"], "Clean");
    assert_eq!(
        report["report"]["output_sha256"].as_str().map(str::len),
        Some(64)
    );
    assert!(rendered.is_file());

    // People get the leak report.
    let out = blurrd(
        &dir,
        &[
            "audit",
            s(&clip),
            "--plan",
            s(&project),
            "-o",
            s(&rendered),
            "--no-gpu",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("verify: clean"), "{text}");
}

#[test]
fn run_writes_one_output_per_recording() {
    let dir = Workdir::new("run");
    let clip = dir.path("clip.mp4");
    write_sized_clip(&clip, 90, 640, 360);
    // A run writes the unverified output (no OCR yet) and reports it.
    let out = blurrd(
        &dir,
        &[
            "run",
            s(&clip),
            "-o",
            s(&dir.path("run.mp4")),
            "--no-verify",
            "--encoder",
            "mpeg4",
            "--json",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert!(dir.path("run_UNVERIFIED.mp4").is_file());
    let report = json_lines(&out).pop().unwrap();
    assert_eq!(report["type"], "Report");
    assert_eq!(report["runs"][0]["streamed"], true);
    assert_eq!(report["runs"][0]["render"]["frames"], 90);

    // Several recordings go into a folder, named after each.
    let second = dir.path("second.mp4");
    write_sized_clip(&second, 30, 640, 360);
    let folder = dir.path("outputs");
    let out = blurrd(
        &dir,
        &[
            "run",
            s(&clip),
            s(&second),
            "-o",
            s(&folder),
            "--no-verify",
            "--encoder",
            "mpeg4",
            "--accept-unverified",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(folder.join("clip_blurrd.mp4").is_file());
    assert!(folder.join("second_blurrd.mp4").is_file());
    let summary = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        summary.lines().filter(|l| l.starts_with("wrote ")).count(),
        2
    );
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
        &["analyze", "clip.mp4", "--regions", "names"],
        &["analyze", "clip.mp4", "--regions", ""],
        &["run", "-o", "out"],
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
