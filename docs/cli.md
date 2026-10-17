# Command-line interface

`blurrd` is the command-line tool. It uses the same engine as the desktop
app.

```text
blurrd probe   <input> [--json]
blurrd analyze <input> [--regions LIST] [--no-verify] [-o project.blurrd.json]
blurrd render  <input> --plan <project.blurrd.json> -o <output> [export options]
blurrd run     <input>... -o <output|folder> [--regions LIST] [--no-verify] [export options]
blurrd audit   <input> --plan <project.blurrd.json> -o <output> [--every-frame]
```

Global options (before or after the command):

| Option              | Meaning                                                                                  |
| ------------------- | ---------------------------------------------------------------------------------------- |
| `--json`            | Machine-readable output: one JSON object per line on stdout, JSON logs on stderr.        |
| `--log-level LEVEL` | `error`, `warn` (default), `info`, `debug` or `trace`. `RUST_LOG` overrides it.          |
| `--threads N`       | Decoder threads (default: chosen by the decoder).                                        |
| `--no-gpu`          | Software decoding, Media Foundation instead of NVENC, AMF or Quick Sync, OCR on the CPU. |

## `blurrd probe`

Shows what blurrd knows about a recording: codec, picture size, frame rate
(and whether it varies), frame and keyframe counts, colour tags, B-frames,
edit lists, audio and subtitle streams. With `--json` it prints the
`MediaInfo` object (see `crates/blurrd-core/bindings/MediaInfo.ts`).

## `blurrd analyze`

Finds the player names in a recording and saves the project (the boxes to
blur, per frame) in the cache folder `%LOCALAPPDATA%\blurrd\projects`, next
to the recording's PTS table. A project of the same recording that is
already in the cache keeps its manual edits.

| Option              | Meaning                                                                  |
| ------------------- | ------------------------------------------------------------------------ |
| `--regions LIST`    | Comma-separated region keys, or `all` (default: every supported region). |
| `--no-verify`       | Skip the OCR verification. The result stays unverified.                  |
| `-o, --output FILE` | Also write the project to this file, for `blurrd render --plan`.         |

The region keys are `killfeed`, `banner`, `voice`, `chat`, `scoreboard`,
`recap`, `spectating` and `matchend`. Name tags above players' heads in
the game world are out of scope and never blurred. Without ONNX Runtime and
the OCR models (or with `--no-verify`) the result is not verified, and
outputs made from it are named `*_UNVERIFIED`.

With OCR the analysis verifies its result. While the detectors run, OCR
reads the protected zones of the recording (the kill feed and banner area
and the left panel on every frame with HUD structure and one frame in six
elsewhere, a screen surface's names while it is open, the whole frame
every half second). Afterwards blurrd makes the blurred pixels of every
name part the boxes do not cover completely, exactly as the render will,
and reads them again. A name that stays readable gets an extra box (a fix
box over the name's slot, grown up to three times) and is listed as a
fixed leak; one that stays readable after that is open. Names outside
every supported region (name tags above heads) are listed as out of scope
and never change the result. The report, `verify-<revision>.json` next to
the project in the cache, holds frames, regions, boxes, confidences and
counts, never text. The result is `Clean` when no leak is open and OCR
read every name the detectors boxed; it is `Unverified` when OCR could not
read some of them on the recording, and `LeaksRemain` with an open leak.

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
made for: the file's fingerprint (size and a hash of its first and last
megabyte), the picture size, the frame count, and the hashes of every frame
timestamp and of the keyframes. A changed file fails with `ProjectMismatch`
instead of blurring the wrong frames. The timestamps come from the cache
when the recording was analysed on this computer, otherwise from one pass
over the file.

## `blurrd run`

Analyses and blurs recordings in one pass: the render trails the analysis
and blurs each frame as soon as its boxes are final, so the output is the
same as `analyze` followed by `render`, in about the time of the slower of
the two. The project is saved in the cache as by `analyze`. It takes
`--regions` and `--no-verify` like `analyze` and the export options of
`render` (`--strength` sets the blur strength). The verification ends
after the render wrote its last frames: a clean output gets its own name,
and when the verification added fix boxes the project is rendered again
(a fix means a detector missed a name, which is rare).

`-o` names the output file. With several recordings, or when it names an
existing folder, the outputs go into that folder as `<name>_blurrd.<ext>`
(MP4, MOV and Matroska inputs keep their container, others become MP4).
Each recording is its own job; a failed one does not stop the others, a
Ctrl+C stops the run. When a recording's cached project has manual edits,
it is analysed first and then rendered with them.

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

## `blurrd audit`

Reads a rendered output with OCR, as an end-to-end check of what the
encoder wrote. It reads the recording's protected zones as the analysis
does (one frame in six, or every frame with `--every-frame`), then the
output's pixels of every name part found there, not only the parts the
boxes leave open, and lists every name that is still readable. An output
with a readable name is renamed `<name>_UNVERIFIED.<ext>` and the exit code
is 3. Items that cannot be read with confidence make the result
`Unverified` but keep the name and exit code 0. The report,
`audit-<revision>.json` in the cache, carries the output's SHA-256.

| Option              | Meaning                                   |
| ------------------- | ----------------------------------------- |
| `--plan FILE`       | The project the output was rendered with. |
| `-o, --output FILE` | The rendered output.                      |
| `--every-frame`     | Read every frame instead of one in six.   |

### Leak report

`analyze`, `run` and `audit` end with a leak report: the result and its
counts, then one line per leak with its frames, region, how it was found
(`dictionary`: it matches a name read elsewhere in the recording;
`dictionary-free`: confident text of name length in a name slot), its
state (`fixed`, `OPEN`) and its box in source pixels. It never shows text.

## JSON output

With `--json`, `analyze`, `render` and `run` print the job's events, one per
line, in the form the desktop app receives them
(`crates/blurrd-core/bindings/JobEvent.ts`), tagged by `type`: `Stage`,
`Started`, `Progress` (at most 20 per second), `Warning`, then `Finished`,
`Canceled` or `Failed`. The last line is a report. A render's report:

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
  "accepted_unverified": false,
  "fallbacks": [],
  "skipped_streams": [],
  "moov": "reserved"
}
```

`verified` says whether the project was verified; `accepted_unverified`
is true when it was not and `--accept-unverified` named the output
normally. The `Finished` event's `verify` likewise reports the project's
own state, never `Clean` for an accepted unverified project.

An analysis reports `project_file` (the project in the cache), `frames`,
`tracks`, `decoder`, `elapsed_ms`, `fps`, `max_delay` (the largest number of
frames a temporal model held a frame, at most 240), `pass0` (OCR counters,
`null` without OCR), `verify` (the result and its counts: `state`, `open`,
`fixed`, `accepted`, `out_of_scope`, `unverifiable_items`), `verify_report`
(the report in the cache), `verify_ms` (the time of the verification after
the detectors; all three `null` without OCR) and `plan` (the file of `-o`,
or `null`). A run reports `runs`: per written recording its `analysis` and
`render` reports, whether it `streamed`, whether it was `rerendered` with
fix boxes, and its `elapsed_ms`.

An audit reports `report_file`, `output` (the final name), `renamed`,
`frames`, `elapsed_ms` and `report` (the whole `blurrd-verify/1` report).

`probe --json` prints the `MediaInfo` object. Errors outside a job are
printed as `{"type": "Error", "error": {"kind": "NotFound", "message": "…",
"detail": "…"}}`.

## Exit codes

| Code | Meaning                                                                                          |
| ---- | ------------------------------------------------------------------------------------------------ |
| 0    | Success.                                                                                         |
| 1    | Error (the message says which).                                                                  |
| 2    | Invalid arguments.                                                                               |
| 3    | The output was written but is not verified (`*_UNVERIFIED`), or the audit found a readable name. |
| 130  | Canceled with Ctrl+C; no output is left behind.                                                  |
