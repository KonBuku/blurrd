/**
 * The `blurrd://` scheme of the mock backend. `fetch` calls to the scheme
 * (`http://blurrd.localhost/…`, as `convertFileSrc` builds it on Windows)
 * are answered here: Exact patches carry the frame index code of the
 * requested frame, drawn inverted exactly where the fixture video has it
 * (so a test sees both that the patch is drawn and where), thumbnails
 * are scaled fixture frames, and leak crops are the leak's box of its
 * first frame, block-averaged unless the clear view is asked for.
 */
import type { RectI } from "@/bindings/RectI";
import type { ExactFrame } from "@/ipc/media";

import { FIXTURE, INDEX_RECT, THUMB_EVERY, drawFrame, drawIndex } from "./fixture-media";

const PREFIX = "http://blurrd.localhost/";

/** The browser's own `fetch`, kept once so reinstalling never wraps twice. */
const realFetch = window.fetch.bind(window);

/** Thumbnail size (16:9). */
const THUMB = { width: 160, height: 90 } as const;

/** Margin around a leak's box in its crop, in source pixels. */
const CROP_MARGIN = 6;
/** Block size of the blurred crop view, in source pixels. */
const CROP_BLOCK = 8;

/** One mocked scheme request, for tests. */
export interface SchemeRequest {
  readonly path: string;
  /** The query string without `?` (for example `view=clear`). */
  readonly query: string;
}

/** What the scheme needs to know about the mock's projects. */
export interface SchemeProjects {
  /** Frame count of a project id. */
  readonly frames: (project: string) => number;
  /** First frame and box of leak `index` of a project's report, if any. */
  readonly leak: (project: string, index: number) => { frame: number; rect: RectI } | null;
}

/** Replaces each `CROP_BLOCK` square of a packed RGB image with its mean. */
function blockAverage(rgb: Uint8Array, width: number, height: number): void {
  for (let by = 0; by < height; by += CROP_BLOCK) {
    for (let bx = 0; bx < width; bx += CROP_BLOCK) {
      const sum = [0, 0, 0];
      let count = 0;
      const each = (visit: (i: number) => void) => {
        for (let y = by; y < Math.min(by + CROP_BLOCK, height); y += 1) {
          for (let x = bx; x < Math.min(bx + CROP_BLOCK, width); x += 1) visit((y * width + x) * 3);
        }
      };
      each((i) => {
        for (let c = 0; c < 3; c += 1) sum[c] = (sum[c] ?? 0) + (rgb[i + c] ?? 0);
        count += 1;
      });
      each((i) => {
        for (let c = 0; c < 3; c += 1) rgb[i + c] = Math.round((sum[c] ?? 0) / count);
      });
    }
  }
}

/** Packed RGB → a JPEG blob. */
async function jpeg(rgb: Uint8Array, width: number, height: number, scale = 1): Promise<Blob> {
  const rgba = new Uint8ClampedArray(width * height * 4);
  for (let i = 0; i < width * height; i += 1) {
    rgba.set(rgb.subarray(i * 3, i * 3 + 3), i * 4);
    rgba[i * 4 + 3] = 255;
  }
  const source = await createImageBitmap(new ImageData(rgba, width, height));
  const canvas = new OffscreenCanvas(Math.round(width * scale), Math.round(height * scale));
  const context = canvas.getContext("2d");
  context?.drawImage(source, 0, 0, canvas.width, canvas.height);
  return canvas.convertToBlob({ type: "image/jpeg", quality: 0.92 });
}

/** Answers one scheme path, or `null` for an unknown path (404). */
async function answer(
  path: string,
  query: string,
  projects: SchemeProjects,
): Promise<Response | null> {
  const { frames } = projects;
  const exactJson = /^exact\/([^/]+)\/(\d+)\.json$/.exec(path);
  if (exactJson) {
    const frame = Math.min(Number(exactJson[2]), frames(exactJson[1] ?? "") - 1);
    const body: ExactFrame = { frame, revision: 3, patches: [{ roi: "index", rect: INDEX_RECT }] };
    return Response.json(body);
  }
  const exactJpeg = /^exact\/([^/]+)\/(\d+)\/index\.jpg$/.exec(path);
  if (exactJpeg) {
    const [x0, y0, x1, y1] = INDEX_RECT;
    const rgb = new Uint8Array((x1 - x0) * (y1 - y0) * 3);
    drawIndex(rgb, x1 - x0, Number(exactJpeg[2]), [x0, y0], true);
    return new Response(await jpeg(rgb, x1 - x0, y1 - y0));
  }
  const thumb = /^thumb\/([^/]+)\/(\d+)\.jpg$/.exec(path);
  if (thumb) {
    const rgb = new Uint8Array(FIXTURE.width * FIXTURE.height * 3);
    drawFrame(rgb, Math.min(Number(thumb[2]) * THUMB_EVERY, FIXTURE.frames - 1));
    return new Response(
      await jpeg(rgb, FIXTURE.width, FIXTURE.height, THUMB.width / FIXTURE.width),
    );
  }
  const crop = /^crop\/([^/]+)\/(\d+)\.jpg$/.exec(path);
  const leak = crop ? projects.leak(crop[1] ?? "", Number(crop[2])) : null;
  if (leak) {
    const full = new Uint8Array(FIXTURE.width * FIXTURE.height * 3);
    drawFrame(full, leak.frame);
    const x0 = Math.max(0, leak.rect[0] - CROP_MARGIN);
    const y0 = Math.max(0, leak.rect[1] - CROP_MARGIN);
    const x1 = Math.min(FIXTURE.width, leak.rect[2] + CROP_MARGIN);
    const y1 = Math.min(FIXTURE.height, leak.rect[3] + CROP_MARGIN);
    const rgb = new Uint8Array((x1 - x0) * (y1 - y0) * 3);
    for (let y = y0; y < y1; y += 1) {
      rgb.set(
        full.subarray((y * FIXTURE.width + x0) * 3, (y * FIXTURE.width + x1) * 3),
        (y - y0) * (x1 - x0) * 3,
      );
    }
    if (new URLSearchParams(query).get("view") !== "clear") blockAverage(rgb, x1 - x0, y1 - y0);
    return new Response(await jpeg(rgb, x1 - x0, y1 - y0, 2));
  }
  return null;
}

/**
 * Routes `fetch` calls to the scheme through the mock and records them in
 * `requests`; other URLs go to the real `fetch`.
 */
export function installSchemeFetch(requests: SchemeRequest[], projects: SchemeProjects): void {
  window.fetch = async (input, init) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    if (!url.startsWith(PREFIX)) return realFetch(input, init);
    init?.signal?.throwIfAborted();
    const [rawPath = "", query = ""] = url.slice(PREFIX.length).split("?");
    const path = decodeURIComponent(rawPath);
    requests.push({ path, query });
    return (await answer(path, query, projects)) ?? new Response(null, { status: 404 });
  };
}
