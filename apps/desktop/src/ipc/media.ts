/**
 * Binary paths of the backend (plan 9.2): the `blurrd://` scheme serves
 * timeline thumbnails, the queue's poster frames and the Exact patches (the export's own blur of the
 * two ROI regions around the current frame, JPEG from YCbCr). Both are
 * read with `fetch` and decoded to `ImageBitmap`s, so stale requests can
 * be aborted and images are ready for WebGL.
 */
import { convertFileSrc } from "@tauri-apps/api/core";

import type { ExactFrame } from "@/bindings/ExactFrame";
import type { ExactSettings } from "@/bindings/ExactSettings";
import type { RectI } from "@/bindings/RectI";

/** URL of `path` on the `blurrd` scheme (`http://blurrd.localhost/…` on Windows). */
export function schemeUrl(path: string): string {
  return convertFileSrc(path, "blurrd");
}

/** A decoded Exact patch. */
export interface ExactPatch {
  readonly rect: RectI;
  readonly image: ImageBitmap;
}

/** A failed request to the backend and its HTTP status. */
export class HttpError extends Error {
  readonly status: number;

  constructor(url: string, status: number) {
    super(`${url}: HTTP ${status}`);
    this.name = "HttpError";
    this.status = status;
  }
}

async function fetchOk(url: string, signal: AbortSignal): Promise<Response> {
  const response = await fetch(url, { signal });
  if (!response.ok) throw new HttpError(url, response.status);
  return response;
}

/**
 * Whether a failed scheme request is tried again: only when the backend
 * answered 503 (its request queue was full during a burst), at most
 * twice. Use with `busyRetryDelay` as a query's `retry`.
 */
export function retryWhenBusy(failures: number, error: unknown): boolean {
  return failures < 2 && error instanceof HttpError && error.status === 503;
}

/** The pause before retry `failures + 1` of a busy request: 250 ms, then 500 ms. */
export function busyRetryDelay(failures: number): number {
  return 250 * 2 ** failures;
}

/** Decodes the image at `url`. */
export async function loadImage(url: string, signal: AbortSignal): Promise<ImageBitmap> {
  const blob = await (await fetchOk(url, signal)).blob();
  return createImageBitmap(blob);
}

/** The `settings` query of the Exact URLs. */
export function exactQuery(settings: ExactSettings): string {
  return `settings=${encodeURIComponent(JSON.stringify(settings))}`;
}

/**
 * The Exact patches of `frame` at project `revision`: the decoded frame
 * index the backend reports and its blurred ROI images. `settings` are
 * the blur and the enabled regions the export would use (the
 * inspector's, which can differ from the analysis), so the patches show
 * the export's pixels.
 */
export async function fetchExact(
  projectId: string,
  frame: number,
  revision: number,
  settings: ExactSettings,
  signal: AbortSignal,
): Promise<{ readonly frame: number; readonly patches: readonly ExactPatch[] }> {
  const query = `rev=${revision}&${exactQuery(settings)}`;
  const info = (await (
    await fetchOk(`${schemeUrl(`exact/${projectId}/${frame}.json`)}?${query}`, signal)
  ).json()) as ExactFrame;
  const patches = await Promise.all(
    info.patches.map(async (patch) => ({
      rect: patch.rect,
      image: await loadImage(
        `${schemeUrl(`exact/${projectId}/${info.frame}/${patch.roi}.jpg`)}?${query}`,
        signal,
      ),
    })),
  );
  return { frame: info.frame, patches };
}

/** Loads the image at `url` into an object URL. */
export async function loadObjectUrl(url: string, signal: AbortSignal): Promise<string> {
  return URL.createObjectURL(await (await fetchOk(url, signal)).blob());
}

/**
 * URL of the poster frame of a queued video (a small JPEG of a frame a
 * tenth into it), available before any project is open.
 */
export function posterUrl(path: string): string {
  return `${schemeUrl("poster.jpg")}?path=${encodeURIComponent(path)}`;
}

/** URL of timeline thumbnail `index` of a project. */
export function thumbnailUrl(projectId: string, index: number): string {
  return schemeUrl(`thumb/${projectId}/${index}.jpg`);
}

/**
 * URL of the image of leak `index` of a project's report: `blurred` is
 * the crop as the backend blurs it (shown by default, it never carries
 * readable text), `clear` re-decodes the frame for the user's "reveal".
 */
export function leakCropUrl(projectId: string, index: number, view: "blurred" | "clear"): string {
  return `${schemeUrl(`crop/${projectId}/${index}.jpg`)}?view=${view}`;
}
