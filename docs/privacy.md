# Privacy and verification

This page states exactly what blurrd blurs and what it does not, how it
checks its own result, where that check has limits, which cases were
verified on real recordings, and what blurrd stores on your computer.

## The promise

blurrd blurs player names in the supported HUD regions (kill feed,
ACE/clutch banner, voice-chat speaker, text-chat sender) and on the
scoreboard, the death recap, the "Spectating" banner and the match-end/MVP
screens, and checks the result with OCR. Name tags above players' heads in
the game world are not blurred. Voice audio is not anonymised. Never
publish a file marked UNVERIFIED without checking it.

| Surface                                 | Region key   | In v0.1                        |
| --------------------------------------- | ------------ | ------------------------------ |
| Kill feed: killer and victim names      | `killfeed`   | blurred                        |
| ACE/clutch round banner                 | `banner`     | blurred                        |
| Voice-chat speaker list                 | `voice`      | blurred                        |
| Text-chat sender names                  | `chat`       | blurred (the message stays)    |
| Scoreboard (Tab)                        | `scoreboard` | blurred                        |
| Death recap and combat report           | `recap`      | blurred                        |
| Spectating widget                       | `spectating` | blurred                        |
| Match-end and MVP screens               | `matchend`   | blurred                        |
| Name tags above heads in the game world | none         | **not blurred** (out of scope) |
| Buy-phase loadout cards above teammates | none         | **not blurred** (out of scope) |
| Voice audio                             | none         | **not changed**                |

The world-space name tags and the buy-phase loadout cards are out of scope
for v0.1 by decision; a later release will handle them. When OCR sees a
name outside every supported region, the report lists it as "out of scope"
with its frames, and the result does not change. While a player is dead,
the game shows the spectated teammate's name above a "switch player" hint;
blurrd's `spectating` region covers that widget.

## How names are covered

Each region has its own detector. It finds the region's rows, slots or
panels from their structure (row pitch, bars, panel edges, text strokes),
learns the colours of the video instead of expecting fixed ones, and maps
everything through the HUD layout it calibrated from frames sampled
across the video. Temporal models follow every item from its first to its
last visible frame, including slide-ins, stack moves, wipes and fades, and
decide each frame's boxes with a look-ahead of up to 240 frames (four
seconds at 60 fps).

Boxes come in two kinds. A tight box covers the name itself where the
tracking is confident. A slot box covers the whole place a name can occupy
and is used for uncertain frames (the first and last frames of an
animation, gaps, colour guesses), for every screen surface, and for the
whole video in Safe mode. When the HUD layout cannot be confirmed, blurrd
switches every region to Safe mode and warns (`CalibrationLow`): that
costs more blur, never less.

The blur itself only reads pixels inside the padded box. It pixelates with
a block size of at least 8 reference pixels (larger on uncertain frames)
on a grid anchored to the text, adds noise per block and blends in a soft
gaussian edge, so that names cannot be sharpened back.

## How the result is checked

With OCR available (ONNX Runtime and the bundled PP-OCRv5 models), every
analysis verifies its own result:

1. **Reading the source.** While the detectors run, OCR reads the
   protected zones of the recording: the kill feed and banner area and the
   left panel (voice and chat) whenever the layout of their items changes,
   again once it has settled, and at least once a second; a screen
   surface's name zone the same way while the surface is open; the whole
   frame once a second. The names it reads form a dictionary that lives
   only in memory.
2. **Positive control.** Each detected item must have been read on the
   source. Items OCR could not read with confidence are listed as
   unverifiable; they are covered by the boxes, but OCR cannot confirm it.
3. **Reading the blurred result.** For every part of a name that the boxes
   do not cover completely, blurrd blurs the decoded frame exactly as the
   export will (the blur reads only pixels inside its box, so this is the
   export's picture before encoding) and reads it again.
4. **Fixing.** A name that is still readable, either because it matches a
   name read elsewhere in the recording (`dictionary`) or because it is
   confident text of name length in a name slot (`dictionary-free`), gets
   a fix box over its slot for the whole time it is visible, and is read
   again, up to three rounds. It is listed as a fixed leak, or as an open
   leak if it stays readable.
5. **Result.** `Clean` when no leak is open, every detected item was read
   and no edited range is waiting for a check. `Unverified` when some
   items could not be read, or edits were not checked yet. `LeaksRemain`
   when a leak stays open.

An export is named normally only when its project's verification is
`Clean` for the current version of the boxes. Otherwise it is named
`<name>_UNVERIFIED.<ext>` (the command-line tool exits with code 3) until
you review the report and accept it in the app, or pass
`--accept-unverified` to the command-line tool. Manual edits mark their
frames as unverified; the app checks those ranges again before it
exports.

`blurrd audit` repeats the check on the encoded file: it reads the same
zones on the same frames as the analysis (every frame with
`--every-frame`), then the export's own pixels of every name part, and
renames an export with a readable name to `*_UNVERIFIED`.

The verification report (`verify-<revision>.json`, schema
`blurrd-verify/1` in [schema/](schema/)) and the audit report contain
frames, regions, boxes, confidences and counts. They never contain the
text OCR read, and neither do the logs.

## Limits of the check

The OCR check finds what the detectors missed in most cases, but not in
all. Know its limits before you publish:

- **OCR has to read the name first.** A name OCR cannot read on the source
  (very small, heavily filtered, unusual script) cannot be matched. Such
  items make the result `Unverified`, not `Clean`.
- **Whole regions the detector never saw are not checked.** OCR counts a
  name slot only on frames where the region's detector saw its HUD
  structure and boxed something nearby. If a detector misses a region
  completely, the check does not catch it; that case is covered by the
  detectors' own tests and the leak sweeps described below. blurrd does
  not yet warn automatically when a region shows its structure but no
  names were detected.
- **Not every frame is read.** The source is read at the HUD's layout
  changes and at least once a second; fix boxes cover the whole time
  between the reads. The app's Verify settings can read every frame
  instead (slower).
- **HUD words are not names.** Words the HUD shows itself are on a list
  that the check ignores: location names, callouts (also joined, such as
  "MidTop"), agent names, distance markers such as "28m", scoreboard
  headers ("PING", "KDA", "CREDS", "LOADOUT", "ULTIMATE"), weapon classes
  and system words. A player whose name is exactly such a word is not
  counted as a leak by the check; the detectors still box it.
- **Encoding.** The check inside the analysis sees the blurred picture
  before encoding; encoding can only remove detail. `blurrd audit` checks
  the encoded file itself.
- **Out of scope stays visible.** Name tags above heads and the buy-phase
  loadout cards are listed when OCR happens to read them, but they are
  never blurred and never make a result fail.

## Verified and unverified cases

blurrd was developed with 31 real VALORANT recordings from one PC: 1080p60
H.264 (8-bit, SDR, no B-frames) with AAC audio, 47 to 60 seconds each
(2,840 to 3,601 frames, 108,627 frames in total). Ten of them were never
used to tune a detector and serve as the held-out check. More recordings
were not available for v0.1, so every case below without real footage is
listed as not verified, even where synthetic tests pass.

"Leak-free" means: an OCR leak sweep over the blurred frames of all 31
recordings (108,627 frames per variant) found no readable player name on a
supported surface. The sweep uses its own OCR pipeline and leak rules,
independent of blurrd's verification.

### Verified on real recordings

| Case                                                                                                            | Result                 |
| --------------------------------------------------------------------------------------------------------------- | ---------------------- |
| Kill feed: single and stacked rows, multikills, self kills, wipe-in, flashes, bright walls                      | leak-free              |
| Round banner: one- and two-word names, bright backgrounds, effects, overlap with the kill feed                  | leak-free              |
| Voice chat: single and stacked speakers, blinking entries                                                       | leak-free              |
| Text chat: team and broadcast lines, system lines, wrapped lines, fades, push-ups, two-word names, low contrast | leak-free              |
| Scoreboard (Tab)                                                                                                | leak-free              |
| Death recap and combat report                                                                                   | leak-free              |
| Spectating widget ("switch player")                                                                             | leak-free              |
| Round-end banners (they show no player name)                                                                    | not boxed, as intended |

### Tested on variants of the real recordings (not verified natively)

These variants were made from the 1080p recordings with FFmpeg filters
and scaling. They are simulations: a real 1440p recording, a real
colour-blind mode or a real NVIDIA or AMD filter can differ.

| Case                  | Variants                                                                                                                                  | Result                                              |
| --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------- |
| Colour filters        | greyscale, saturation 0.1, hue rotated by 120Â°, gamma 0.7, 1.33 and 2.0, brightness âˆ’0.12 with contrast 0.85, sharpening, vibrance 0.6 | leak-free                                           |
| Colour-blind palettes | deuteranopia, protanopia and tritanopia simulated with colour matrices                                                                    | leak-free                                           |
| Other resolutions     | 720p, 900p, 1440p and 2160p, 720p in greyscale, 720p with gamma 2.0                                                                       | leak-free; also at native size on 8 recordings      |
| 4:3 stretched         | the 16:9 picture squeezed to 1440Ã—1080                                                                                                   | no readable name (excerpts of 3 recordings, full 1) |
| 4:3 native size       | the 16:9 picture scaled to 1280Ã—960 and 1024Ã—768                                                                                        | no readable name (excerpts of 3 recordings, full 2) |
| 4:3 pillarbox         | 1920Ã—1080 with black side bars                                                                                                           | no readable name (excerpts of 3 recordings, full 1) |
| 16:10                 | 1920Ã—1200 and 1680Ã—1050 (squeezed), 1920Ã—1200 with black bars at the top and bottom                                                    | no readable name (full recordings, 3)               |

The leak sweep brings each resolution back to 1080p before it detects; the
native-size runs feed the scaled frames to blurrd's own HUD mapping
instead. The 4:3 and 16:10 variants were run end to end through the
command-line tool with OCR (analysis, verification and export), and the
exports were read with OCR every fifth frame.

No real 4:3 or 16:10 recordings exist for v0.1: the variants above are
16:9 recordings squeezed, scaled or padded into the other frame. The
game's own 4:3 HUD layout was tested on synthetic frames only.

### Not verified

No recording of these cases was available. Where synthetic tests exist,
they run in CI, but they do not replace a real recording.

| Case                                                                               | Tested with                                  |
| ---------------------------------------------------------------------------------- | -------------------------------------------- |
| The game's colour-blind modes                                                      | simulated palettes only (see above)          |
| Native 1440p and 2160p recordings                                                  | rescaled variants only                       |
| Native 4:3 and 16:10 rendering, the game's own 4:3 HUD                             | synthetic frames                             |
| NVIDIA and AMD driver filters (Freestyle, vibrance, sharpening)                    | FFmpeg filters that imitate them             |
| HDR recordings                                                                     | unit tests of the colour handling            |
| OBS x264 recordings with B-frames                                                  | synthetic clips with B-frames and edit lists |
| ShadowPlay / NVIDIA App HEVC recordings                                            | none                                         |
| Variable frame rate recordings                                                     | synthetic clips                              |
| Client languages other than the one of the recordings                              | synthetic chat lines                         |
| Deathmatch and Team Deathmatch                                                     | none                                         |
| Spectator and tournament observer HUD                                              | none                                         |
| A full five-player party in voice chat                                             | synthetic fixtures with four speaker slots   |
| Six or more stacked kill-feed rows                                                 | synthetic fixtures                           |
| Long names and names in other scripts (Hangul, Japanese, Thai, Arabic, Devanagari) | synthetic fixtures                           |
| Match-end and MVP screens                                                          | synthetic frames                             |
| A "Spectating <name>" banner, if the game shows one                                | none                                         |
| Exports on AMD (AMF) and Intel (Quick Sync) encoders                               | none                                         |

## What blurrd stores and sends

blurrd works offline. It makes no network connections and sends no
telemetry. Inside the desktop app, the video player gets the recordings
from a small server that listens only on `127.0.0.1` and answers only
requests that carry a random token of the running app. Links in the About
dialog open in your browser only when you click them.

blurrd reads only the files you choose and writes:

| Where                                           | What                                                                                                                                                                                                                                                                                                                                                                                                   |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| The output file                                 | the blurred video; recorder metadata (titles, comments, creation times, account tags) is removed                                                                                                                                                                                                                                                                                                       |
| `%LOCALAPPDATA%\blurrd\projects\<fingerprint>\` | per recording: `project.json` (the recording's path and fingerprint, the boxes, your edits, the settings and the verification state; no text), the frame timestamp table `pts.bin`, the HUD structure record `structure.bin`, the reports `verify-*.json` and `audit-*.json`, and for the app's preview the timeline thumbnails `thumbs\*.jpg` and, for recordings the player cannot play, `proxy.mp4` |
| `%LOCALAPPDATA%\blurrd\caps\`                   | which decoders and encoders work on this computer                                                                                                                                                                                                                                                                                                                                                      |
| `%LOCALAPPDATA%\blurrd\session.json`            | the app's queue (file paths and their status)                                                                                                                                                                                                                                                                                                                                                          |
| `%APPDATA%\io.github.konbuku.blurrd\`           | the app's `settings.json` and your presets (`presets.json`)                                                                                                                                                                                                                                                                                                                                            |
| `%LOCALAPPDATA%\io.github.konbuku.blurrd\logs\` | the app's log: file names, frame numbers, timings and errors, never OCR text                                                                                                                                                                                                                                                                                                                           |

The thumbnails and the preview proxy are unblurred pictures of the
recording, like the recording itself. The small poster frame a queue row
shows is decoded on request and kept in memory only for the session. Delete `%LOCALAPPDATA%\blurrd` to
remove every cached project; blurrd creates the folder again when needed.
Names that you keep readable on purpose are stored in your settings and
presets; an exported preset file leaves them out unless you tick the box.
The report dialog builds leak crops from the recording when you open it
and shows the unblurred crop only when you click "Reveal"; crops are not
saved.
