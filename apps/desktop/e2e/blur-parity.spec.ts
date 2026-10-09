import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";

import { expect, test } from "@playwright/test";

import type { BlurProbeCase } from "../src/features/preview/gl/blur-probe";

/**
 * The WebGL preview blur against `CpuBlur` (acceptance M24). The reference
 * frames come from `cargo run -p blurrd-blur --example reference_frames`
 * (default folder `target/blur-reference`, or `BLURRD_BLUR_REFERENCE`).
 * Without them the test is skipped, unless `BLURRD_REQUIRE_BLUR_REFERENCE`
 * is set (CI).
 *
 * The CPU blur works on YUV 4:2:0; the preview blurs luma at full
 * resolution and chroma on the half-resolution grid with the max-pooled
 * chroma mask (see `blur-renderer.ts`), so the two differ only by
 * rounding, plus a few pixels where the YUV result lies outside the RGB
 * gamut and the browser's RGB input was already clipped. Tolerance (8-bit
 * RGB code values per channel, pixels of the padded boxes): mean ≤
 * {@link MEAN_LIMIT}, 99th percentile ≤ {@link P99_LIMIT}, maximum ≤
 * {@link MAX_LIMIT}; pixels outside the boxes are identical. Measured
 * (SwiftShader and ANGLE/D3D11): mean ≤ 0.24, p99 ≤ 2, max 15.
 */
const MEAN_LIMIT = 0.5;
const P99_LIMIT = 2;
const MAX_LIMIT = 16;

const folder = resolve(
  process.env.BLURRD_BLUR_REFERENCE ?? join(import.meta.dirname, "../../../target/blur-reference"),
);
const required = Boolean(process.env.BLURRD_REQUIRE_BLUR_REFERENCE);

interface ReferenceCase extends BlurProbeCase {
  readonly name: string;
  readonly input: string;
  readonly expected: string;
}

/** Differences of one case. */
interface Stats {
  readonly mean: number;
  readonly p99: number;
  readonly max: number;
  readonly outside: number;
}

function compare(c: ReferenceCase, actual: Buffer, expected: Buffer): Stats {
  const inBox = new Uint8Array(c.width * c.height);
  const pad = Math.ceil(16 * c.sy) + 2;
  for (const b of c.boxes) {
    const [x0, y0, x1, y1] = b.rect;
    for (let y = Math.max(0, y0 - pad); y < Math.min(c.height, y1 + pad); y += 1) {
      inBox.fill(1, y * c.width + Math.max(0, x0 - pad), y * c.width + Math.min(c.width, x1 + pad));
    }
  }
  const histogram = Array.from({ length: 256 }, () => 0);
  let sum = 0;
  let count = 0;
  let outside = 0;
  for (let i = 0; i < c.width * c.height * 3; i += 1) {
    const diff = Math.abs((actual[i] ?? 0) - (expected[i] ?? 0));
    if (inBox[Math.floor(i / 3)]) {
      histogram[diff] = (histogram[diff] ?? 0) + 1;
      sum += diff;
      count += 1;
    } else {
      outside = Math.max(outside, diff);
    }
  }
  let seen = 0;
  let p99 = 0;
  for (const [value, n] of histogram.entries()) {
    seen += n;
    if (seen >= 0.99 * count) {
      p99 = value;
      break;
    }
  }
  const max = histogram.findLastIndex((n) => n > 0);
  return { mean: sum / count, p99, max, outside };
}

test("the webgl blur matches CpuBlur on the reference frames", async ({ page }) => {
  const index = join(folder, "cases.json");
  if (!existsSync(index)) {
    if (required) throw new Error(`reference frames missing in ${folder}`);
    test.skip(true, `no reference frames in ${folder}`);
    return;
  }
  const { cases } = JSON.parse(readFileSync(index, "utf8")) as { cases: ReferenceCase[] };
  expect(cases.length).toBeGreaterThan(0);
  await page.goto("/");
  await page.waitForFunction(() => window.blurrdBlurProbe !== undefined);
  const report: string[] = [];
  for (const c of cases) {
    const input = readFileSync(join(folder, c.input)).toString("base64");
    // One case at a time: each one is a full frame in the page.
    // oxlint-disable-next-line no-await-in-loop
    const output = await page.evaluate(
      ([probe, data]) => {
        if (!window.blurrdBlurProbe) throw new Error("blur probe missing");
        return window.blurrdBlurProbe(probe, data);
      },
      [c, input] as const,
    );
    const stats = compare(c, Buffer.from(output, "base64"), readFileSync(join(folder, c.expected)));
    report.push(
      `${c.name}: mean ${stats.mean.toFixed(3)} p99 ${stats.p99} max ${stats.max} outside ${stats.outside}`,
    );
    expect.soft(stats.outside, `${c.name}: pixels outside the boxes`).toBe(0);
    expect.soft(stats.mean, `${c.name}: mean difference`).toBeLessThanOrEqual(MEAN_LIMIT);
    expect.soft(stats.p99, `${c.name}: 99th percentile`).toBeLessThanOrEqual(P99_LIMIT);
    expect.soft(stats.max, `${c.name}: largest difference`).toBeLessThanOrEqual(MAX_LIMIT);
  }
  test.info().annotations.push({ type: "parity", description: report.join("\n") });
  console.warn(report.join("\n"));
});
