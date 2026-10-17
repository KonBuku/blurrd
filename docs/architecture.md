# Architecture

This page explains how blurrd is built: the crates and their boundaries,
the jobs of the engine, the data they keep, how the GPU is used and how
the desktop app talks to the engine. [development.md](development.md)
covers building and testing, [privacy.md](privacy.md) what the
verification guarantees.

## Overview

blurrd is a Rust workspace with a React frontend. The engine is a library
shared by two front ends: the `blurrd` command-line tool and the Tauri
desktop app. Every job (analysis, render, streaming run, re-verification,
audit) is implemented once in the engine and reports through the same
event type, so the CLI's `--json` output and the app's job channel carry
identical events.

```text
             apps/desktop/src (React)           blurrd-cli
                     │ IPC (generated types)        │
             apps/desktop/src-tauri (blurrd-app)    │
                     └──────────────┬───────────────┘
                              blurrd-engine
              ┌───────────┬─────────┼──────────┬────────────┐
        blurrd-detect  blurrd-ocr  blurrd-blur  blurrd-video
              └───────────┴────┬────┴──────────┘     (FFmpeg)
                          blurrd-core
```

| Crate            | Responsibility                                                                                                                                                                                                                                                                                        |
| ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `blurrd-core`    | Types shared by everything: regions, HUD geometry and layouts, tracks and blur plans, project, settings, presets, verify reports, job events, errors, JSON Schemas and TypeScript bindings. No I/O and no `unsafe`.                                                                                   |
| `blurrd-blur`    | The blur on YUV 4:2:0 planes (I420, NV12, P010), applied only inside the padded boxes, and its quality metrics.                                                                                                                                                                                       |
| `blurrd-video`   | FFmpeg: probing, decoding (software, NVDEC, D3D11VA), frame-accurate seeking, ROI copies and colour conversion, encoding with NVENC, AMF, Quick Sync or Media Foundation, muxing, preview proxies and thumbnails; Windows system calls (GPU list, power requests). The only crate with `unsafe` code. |
| `blurrd-detect`  | The detectors of every region (kill feed, banner, voice, chat, the four screen surfaces), the HUD calibration, pixel classes, morphology and the temporal models that turn per-frame detections into tracks. Pure image code.                                                                         |
| `blurrd-ocr`     | PP-OCRv5 text detection and recognition on ONNX Runtime (DirectML or CPU), the name dictionary and the leak rules with their whitelist.                                                                                                                                                               |
| `blurrd-engine`  | The jobs: analysis pipeline, verification and fixes, render, streaming run, re-verification, audit, edits and the project cache.                                                                                                                                                                      |
| `blurrd-cli`     | The `blurrd` command-line tool.                                                                                                                                                                                                                                                                       |
| `blurrd-app`     | The Tauri shell of the desktop app (`apps/desktop/src-tauri`): commands, job queue, media serving.                                                                                                                                                                                                    |
| `blurrd-testkit` | Development only: synthetic HUD frames with exact ground truth, quality metrics, golden files.                                                                                                                                                                                                        |

## Coordinates and time

- **Frames** are always identified by their presentation-order index: the
  n-th frame the decoder outputs. Decode order never appears in an API.
  Every frame's timestamp is stored in the PTS table (`pts.bin`); seeking
  maps an index to its timestamp through that table instead of counting
  frames, and a render compares every decoded frame's timestamp with the
  table (`ProjectMismatch` on any difference).
- **Boxes** in the project are integer rectangles in source pixels,
  half-open, so a render is a plain lookup.
- **Detectors** work in a 1920×1080 reference space. `HudGeometry`
  (`blurrd-core`) maps reference rectangles to source pixels for the
  calibrated layout: `Wide16x9`, `Native4x3`, `Stretched4x3`, `Wide16x10`,
  `Pillarbox` and `Letterbox` (bars detected in the picture), `Scaled16x9`
  (a 16:9 picture scaled into another frame) or `Custom`. Each region has
  an anchor (left, right or centre), so one code path covers every
  resolution and aspect ratio. Crops of other sizes are resampled to the
  reference scale exactly as FFmpeg's `scale` filter does, because the
  detectors were validated on such frames.

## Analysis

`Engine::analyze` turns a recording into a project:

1. **Probe and calibrate.** blurrd reads the stream header, then samples
   one keyframe every 4 seconds (8 to 24 samples) while the packet scan
   that builds the PTS table runs. The calibrator scores every HUD layout
   hypothesis on the structure it finds (kill-feed row slots and row
   ends, chat line grid, voice slot pitch, banner geometry) and learns the
   kill feed's bar colours. It picks the layout with the clearest lead;
   with low confidence, every region runs in Safe mode and a
   `CalibrationLow` warning is emitted. HDR videos also get their paper
   white measured for tone mapping (`HdrToneMapped` when that fails).
2. **Decode once.** One decoder (software frame threads or NVDEC; NVDEC
   when OCR runs, to keep the CPU free) feeds reference-counted frames
   through a bounded channel.
3. **Crop and detect.** A fixed pool of workers copies the HUD regions of
   each frame (even-aligned YUV), converts them to RGB and runs the
   per-frame detectors. A reorder buffer with a bounded window restores
   frame order.
4. **Temporal models.** One analyzer thread runs the temporal model of
   every region in frame order: kill-feed row linking, following and end
   detection, banner episodes, voice and chat tracking, screen-surface
   gates. A frame's boxes are final at most `LOOKAHEAD_MAX` (240) frames
   after it was decoded. Uncertain frames (pre-roll, slide-in, gap fill,
   colour guess) use slot boxes, confident tracking keeps tight boxes.
5. **Pass 0 of the verification.** Up to four OCR readers (kill feed and
   banner zone, left panel, screen surfaces, full frame), each with its
   own ONNX Runtime engine, read the protected zones at the HUD's layout
   events and at least once a second, and the full frame once a second.
   The engines are built on their own thread while the analysis already
   runs. Recognition runs only on name-shaped lines.
6. **Previews.** A preview worker writes timeline thumbnails and, for
   videos WebView2 cannot play, an H.264 proxy.
7. **Verify and fix.** After the last frame, the verification compares
   every name part pass 0 read in a name slot with the boxes; parts the
   boxes do not cover completely are blurred in memory with the export's
   own blur (`CpuBlur` on frames decoded again by a frame seeker) and read
   again. Readable names get `OcrFix` slot-box tracks, up to three
   rounds. The result and the report are written with the project.

The tracks, the edits, the settings and the verification state go into
`project.json`; the per-frame structure signals and the zones pass 0 read
go into `structure.bin`, so a later re-verification or audit checks
exactly what the analysis checked.

## Render

`Engine::render` checks the input against the project (fingerprint,
picture size, frame count, hashes of the timestamps and keyframes), then:

```text
demux/decode ─► PTS check ─► bounded channel ─► blur + encode (1 or 2 sessions) ─► packets ─► mux ─► rename
```

- Frames without boxes pass through untouched. Frames with boxes are
  copied into an encoder-owned buffer and blurred there; decoder-owned
  frames are never written, because later frames may reference them.
- The blur reads only pixels inside the padded box (a property test
  guarantees it), works on the YUV planes with the chroma mask max-pooled
  to 4:2:0, and costs microseconds per box on the CPU.
- H.264 on NVENC runs as two sessions over keyframe-aligned segments;
  HEVC and AV1 use NVENC's split-frame encoding. Keyframes follow the
  source. When an encoder fails, the next one in the chain (NVENC, AMF,
  Quick Sync, Media Foundation) takes over with an `EncoderFallback`
  warning.
- The muxer copies audio and subtitle packets unchanged, strips recorder
  metadata, keeps the MP4/MOV index at the front and writes to
  `<name>.tmp.<ext>`, renamed only when complete. A project that is not
  verified gives `<name>_UNVERIFIED.<ext>`.

## Streaming run

`Engine::run` analyses and renders at the same time. The render decoder
trails the analysis decoder; each frame's final boxes wait in a box store
until the render reaches that frame, and the render waits (back-pressure)
instead of ever blurring a frame whose boxes are not final. The output is
therefore identical to an analysis followed by a render, byte for byte.

The verification runs when the analysis has every frame and the render
has written all but its last frames. A clean result renames the output
from `*_UNVERIFIED` to its own name. When the verification added fix
boxes, the project is rendered again: only the GOPs that contain a frame
with changed boxes are encoded, every other packet is copied from the
streamed output (guarded by equal codec headers, the frame count and
keyframe starts; anything unexpected falls back to a full render).

## Re-verification and audit

- `Engine::reverify_ranges` checks the frame ranges that edits changed:
  it decodes only those ranges, reads every frame there with the same
  rules, keeps the rest of the previous report and adds fix boxes where
  needed. The engine keeps the OCR engines of the last re-verification
  warm for the next one.
- `Engine::audit` reads a rendered output: the source zones as the
  analysis read them, then the output's own pixels of every name part.
  It writes `audit-<revision>.json` with the output's SHA-256 and renames
  an output with a readable name to `*_UNVERIFIED`.

## Data on disk

| File                                                                     | Content                                                                                                                                                                | Schema                                  |
| ------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------- |
| `projects/<fingerprint>/project.json`                                    | media facts, HUD geometry, detector settings, tracks, edits (overrides) with the revision they were made on, blur parameters, verification state and unverified ranges | `blurrd-project/1`                      |
| `projects/<fingerprint>/pts.bin`                                         | the timestamp of every frame                                                                                                                                           | binary                                  |
| `projects/<fingerprint>/structure.bin`                                   | per-frame HUD structure signals and the zones pass 0 read                                                                                                              | binary (`BLRDSTR2`)                     |
| `projects/<fingerprint>/verify-<revision>.json`, `audit-<revision>.json` | verification and audit reports: frames, regions, boxes, confidences, counts, no text                                                                                   | `blurrd-verify/1`                       |
| `projects/<fingerprint>/thumbs/`, `proxy.mp4`                            | timeline thumbnails and the preview proxy                                                                                                                              | JPEG, MP4                               |
| `caps/encoders.json`, `caps/decoders.json`                               | the encoder and decoder probe of this machine (GPU, driver, FFmpeg version)                                                                                            | JSON                                    |
| `settings.json`, `presets.json` (app config folder)                      | app settings and user presets                                                                                                                                          | `blurrd-settings/1`, `blurrd-presets/1` |

The cache root is `%LOCALAPPDATA%\blurrd`. A project's folder is named by
the recording's fingerprint (size plus a BLAKE3 hash of its first and last
MiB), so a renamed or moved recording finds its project again. Writes to a
cached project go through one read-modify-write under a process-wide lock,
and a job that finishes keeps edits made while it ran. The JSON Schemas
are generated from the Rust types into [schema/](schema/).

## GPU use and fallbacks

| Stage                   | Preferred                                                                         | Fallbacks                                                                 |
| ----------------------- | --------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| Decode for the analysis | software for 8-bit H.264 without OCR, NVDEC with OCR and for HEVC, AV1 and 10-bit | software (dav1d for AV1), D3D11VA                                         |
| Blur                    | CPU on the YUV planes, only inside boxes                                          | none needed                                                               |
| Encode                  | NVENC (H.264: two sessions, preset p3 in the app)                                 | AMF, Quick Sync, Media Foundation                                         |
| OCR                     | ONNX Runtime with DirectML on the selected adapter                                | ONNX Runtime on the CPU; without ONNX Runtime the result stays unverified |
| Preview                 | WebView2 `<video>` plus a WebGL 2 canvas that applies the same blur               | the H.264 proxy for sources WebView2 cannot decode                        |

ONNX Runtime is loaded at run time only when OCR runs, so blurrd starts
without it. The encoder probe runs once per machine in the background and
is cached. GPU names, adapters and memory come from DXGI.

## Threads, memory and cancellation

All queues between threads are bounded channels, so a slow stage slows
the ones before it instead of buffering frames. Frame memory therefore
does not grow with the length of the recording: the analysis holds at most
the look-ahead of HUD crops, the render a few frames per encoder session.
Only the in-memory log of the text lines pass 0 read grows with the
length (about 5,000 lines per minute of a typical recording).
Every job checks its cancellation token once per frame and removes its
temporary output. While a job runs, the process asks Windows not to
throttle it (EcoQoS off) and not to sleep.

## Desktop app

- **Shell:** `apps/desktop/src-tauri` (crate `blurrd-app`) exposes the
  engine as Tauri commands (`commands.rs`). Their payloads are the
  `blurrd-core` types, exported to TypeScript with ts-rs into
  `apps/desktop/src/bindings`, so the frontend and the backend cannot
  drift apart; a contract test checks that every command is declared,
  registered, granted and mocked.
- **Jobs:** one queue runs jobs one after another on a worker thread.
  Each job reports over its own `Channel<JobEvent>`: progress in frame
  order, at most 20 times a second, with a pass index for jobs that
  decode twice. Pause, resume and cancel act on the queue.
- **Paths:** commands accept only paths the user chose through a dialog
  or a drop; everything else is refused.
- **Media:** recordings and preview proxies play from a loopback server
  (127.0.0.1, random port, per-start token, HTTP range requests). The
  `blurrd://` scheme serves thumbnails, the "Exact" patches (frames
  blurred by the engine itself, for a pixel-exact check of the preview)
  and leak crops.
- **Preview:** a WebGL 2 port of `CpuBlur` blurs each presented video
  frame with the export's parameters; frame numbers are taken from the
  frame the browser presented and mapped through the PTS table.
- **Frontend:** React 19, TypeScript, Tailwind CSS 4, Radix UI primitives,
  TanStack Query for IPC data and Zustand stores; the edit history is a
  zundo store whose full override list is sent with the revision it was
  based on.

## Errors and logging

Each crate has its own error type; the engine converts them into
`AppError` (kind, message for the user, technical detail). Production
code never unwraps. Logs use `tracing`: the CLI writes to stderr (JSON
with `--json`), the app to a JSON log file. Logs and reports never contain
OCR text, only frames, regions, boxes, confidences and counts, and only
file names, not their content.
