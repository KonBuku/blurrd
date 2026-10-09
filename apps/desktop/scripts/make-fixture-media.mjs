// Encodes the synthetic fixture videos of the mock backend (see
// src/ipc/mock/fixture-media.ts) with FFmpeg into e2e/fixtures/media.
// Runs before the end-to-end tests (Playwright global setup) and by hand
// with `pnpm fixtures` for `pnpm dev:mock`. The files are generated, never
// committed (.gitignore excludes video files), and are rebuilt only when
// missing or older than the frame generator.
//
// Requires FFmpeg with libvpx-vp9 and libx265 on PATH (or FFMPEG).
import { spawn } from "node:child_process";
import { once } from "node:events";
import { existsSync, mkdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { FIXTURE, FIXTURE_URLS, drawFrame } from "../src/ipc/mock/fixture-media.ts";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const generator = join(root, "src/ipc/mock/fixture-media.ts");
const ffmpeg = process.env.FFMPEG ?? "ffmpeg";

/** Encoder settings per fixture: output URL, frame count and FFmpeg arguments. */
const OUTPUTS = [
  {
    url: FIXTURE_URLS.video,
    frames: FIXTURE.frames,
    args: ["-c:v", "libvpx-vp9", "-crf", "28", "-b:v", "0", "-g", "30", "-row-mt", "1"],
  },
  {
    url: FIXTURE_URLS.hevc,
    frames: 60,
    args: ["-c:v", "libx265", "-crf", "26", "-tag:v", "hvc1", "-x265-params", "log-level=error"],
  },
  {
    url: FIXTURE_URLS.proxy,
    frames: 60,
    args: ["-vf", "scale=320:180", "-c:v", "libvpx-vp9", "-crf", "28", "-b:v", "0", "-g", "30"],
  },
];

/** Whether `file` exists and is newer than the frame generator. */
function fresh(file) {
  return existsSync(file) && statSync(file).mtimeMs >= statSync(generator).mtimeMs;
}

/** Pipes `frames` generated frames through FFmpeg into `file`. */
async function encode(file, frames, args) {
  mkdirSync(dirname(file), { recursive: true });
  const { width, height, fps } = FIXTURE;
  const child = spawn(
    ffmpeg,
    [
      "-hide_banner",
      "-loglevel",
      "error",
      "-y",
      "-f",
      "rawvideo",
      "-pix_fmt",
      "rgb24",
      "-s",
      `${width}x${height}`,
      "-r",
      String(fps),
      "-i",
      "-",
      "-pix_fmt",
      "yuv420p",
      ...args,
      file,
    ],
    { stdio: ["pipe", "inherit", "inherit"] },
  );
  const done = new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("close", (code) =>
      code === 0 ? resolve() : reject(new Error(`ffmpeg exited with ${code} for ${file}`)),
    );
  });
  const rgb = new Uint8Array(width * height * 3);
  for (let f = 0; f < frames; f += 1) {
    drawFrame(rgb, f);
    // Frames are written in order; wait whenever FFmpeg's pipe is full.
    // oxlint-disable-next-line no-await-in-loop
    if (!child.stdin.write(Buffer.from(rgb))) await once(child.stdin, "drain");
  }
  child.stdin.end();
  await done;
}

for (const output of OUTPUTS) {
  const file = join(root, output.url);
  if (fresh(file)) continue;
  // One encode at a time keeps the CPU free for the rest of the machine.
  // oxlint-disable-next-line no-await-in-loop
  await encode(file, output.frames, output.args);
  console.warn(`fixture written: ${output.url}`);
}
