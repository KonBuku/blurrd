/**
 * The `blurrd://` scheme of the mock backend. `fetch` calls to the scheme
 * (`http://blurrd.localhost/…`, as `convertFileSrc` builds it on Windows)
 * are answered here: Exact patches carry the frame index code of the
 * requested frame, drawn inverted exactly where the fixture video has it
 * (so a test sees both that the patch is drawn and where), and
 * thumbnails are scaled fixture frames.
 */
import type { ExactFrame } from "@/ipc/media";

import { FIXTURE, INDEX_RECT, THUMB_EVERY, drawFrame, drawIndex } from "./fixture-media";

const PREFIX = "http://blurrd.localhost/";

/** The browser's own `fetch`, kept once so reinstalling never wraps twice. */
const realFetch = window.fetch.bind(window);

/** Thumbnail size (16:9). */
const THUMB = { width: 160, height: 90 } as const;

/** One mocked scheme request, for tests. */
export interface SchemeRequest {
  readonly path: string;
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
async function answer(path: string, frames: (project: string) => number): Promise<Response | null> {
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
  return null;
}

/**
 * Routes `fetch` calls to the scheme through the mock and records them in
 * `requests`; other URLs go to the real `fetch`. `frames` gives the frame
 * count of a project id.
 */
export function installSchemeFetch(
  requests: SchemeRequest[],
  frames: (project: string) => number,
): void {
  window.fetch = async (input, init) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    if (!url.startsWith(PREFIX)) return realFetch(input, init);
    init?.signal?.throwIfAborted();
    const path = decodeURIComponent(url.slice(PREFIX.length).split("?")[0] ?? "");
    requests.push({ path });
    return (await answer(path, frames)) ?? new Response(null, { status: 404 });
  };
}
