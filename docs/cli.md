# Command-line interface

`blurrd` is the command-line tool. It uses the same engine as the desktop
app.

```text
blurrd probe  <input> [--json]
blurrd render <input> --plan <project.blurrd.json> -o <output> [options]
```

Global options (before or after the command):

| Option              | Meaning                                                                                   |
| ------------------- | ----------------------------------------------------------------------------------------- |
| `--json`            | Machine-readable output: one JSON object per line on stdout, JSON logs on stderr.         |
| `--log-level LEVEL` | `error`, `warn` (default), `info`, `debug` or `trace`. `RUST_LOG` overrides it.           |
| `--threads N`       | Decoder threads (default: chosen by the decoder).                                         |
| `--no-gpu`          | Software decoding, and Media Foundation instead of NVENC, AMF or Quick Sync for encoding. |

## `blurrd probe`

Shows what blurrd knows about a recording: codec, picture size, frame rate
(and whether it varies), frame and keyframe counts, colour tags, B-frames,
edit lists, audio and subtitle streams. With `--json` it prints the
`MediaInfo` object (see `crates/blurrd-core/bindings/MediaInfo.ts`).

## `blurrd render`

Blurs the boxes of a project and writes the output.

| Option                | Meaning                                                                                                          |
| --------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `--plan FILE`         | The project (`blurrd-project/1`, usually exported as `.blurrd.json`).                                            |
| `-o, --output FILE`   | The output. Its extension picks the container: `.mp4`, `.mov` or `.mkv`. An existing file is replaced.           |
| `--codec`             | `source` (default; H.264, HEVC and AV1 stay in their codec, anything else becomes H.264), `h264`, `hevc`, `av1`. |
| `--encoder`           | `auto` (default: NVENC, AMF, Quick Sync, then Media Foundation) or an encoder name such as `h264_nvenc`.         |
| `--quality`           | `source` (default: the source's bit rate, peaks up to 1.5×), `cq:N` (constant quality 0–51), `vbr:M` (M Mbit/s). |
| `--preset`            | `p1` (fastest) to `p7` (best quality); default `p4`.                                                             |
| `--sessions`          | Parallel NVENC sessions for H.264: `auto` (default, two), `1` or `2`.                                            |
| `--audio`             | `copy` (default, packet for packet), `aac:KBPS` (re-encode, 32–512) or `none`.                                   |
| `--strength`          | `low`, `medium` or `high`; overrides the project's blur strength.                                                |
| `--accept-unverified` | Name the output normally although the project is not verified.                                                   |

The input is never written. The output is written to `<name>.tmp.<ext>`
next to it and renamed only when it is complete, so a failed or canceled
render leaves nothing behind. The output keeps the source's resolution,
frame rate, timestamps and keyframes; audio and subtitles are copied;
recorder metadata (titles, comments, creation times, account tags) is
removed; MP4 and MOV outputs keep their index at the front of the file.

Before rendering, blurrd checks that the input is the video the project was
made for (picture size, frame count and the hash of every frame
timestamp). A changed file fails with `ProjectMismatch` instead of
blurring the wrong frames.

### Verification and output names

A project is verified when its last OCR verification is `Clean`, covers
the current revision and has no unverified ranges. Otherwise the output is
named `<name>_UNVERIFIED.<ext>` and the exit code is 3: check such a file
before publishing it, or pass `--accept-unverified`.

### Encoders

`--encoder auto` tries NVENC, AMF and Quick Sync on the GPUs that exist, then
Media Foundation, which every Windows installation has. Which encoders work
is checked once per machine (GPU, driver and FFmpeg version) and cached in
`%LOCALAPPDATA%\blurrd\caps\encoders.json`. If an encoder that should work
fails, blurrd uses the next one and emits an `EncoderFallback` warning; it
never changes the output silently. H.264 on NVENC runs as two parallel
sessions on keyframe-aligned segments (the two encoder engines of current
NVIDIA GPUs); HEVC and AV1 split each frame across them instead.

## JSON output

With `--json`, `render` prints the job's events, one per line, in the form
the desktop app receives them (`crates/blurrd-core/bindings/JobEvent.ts`),
tagged by `type`: `Stage`, `Started`, `Progress` (at most 20 per second),
`Warning`, then `Finished`, `Canceled` or `Failed`. A successful render ends
with a report:

```json
{
  "type": "Report",
  "output": "D:/clips/match_blurrd.mp4",
  "frames": 3601,
  "blurred_frames": 2140,
  "encoder": "h264_nvenc",
  "sessions": 2,
  "elapsed_ms": 3909,
  "fps": 921.2,
  "verified": true,
  "fallbacks": [],
  "skipped_streams": [],
  "moov": "reserved"
}
```

`probe --json` prints the `MediaInfo` object. Errors outside a job are
printed as `{"type": "Error", "error": {"kind": "NotFound", "message": "…",
"detail": "…"}}`.

## Exit codes

| Code | Meaning                                                      |
| ---- | ------------------------------------------------------------ |
| 0    | Success.                                                     |
| 1    | Error (the message says which).                              |
| 2    | Invalid arguments.                                           |
| 3    | The output was written but is not verified (`*_UNVERIFIED`). |
| 130  | Canceled with Ctrl+C; no output is left behind.              |
