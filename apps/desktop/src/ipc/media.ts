/**
 * Binary paths of the backend (plan 9.2): the `blurrd://` scheme serves
 * timeline thumbnails and the Exact patches (the export's own blur of the
 * two ROI regions around the current frame, JPEG from YCbCr). Both are
 * read with `fetch` and decoded to `ImageBitmap`s, so stale requests can
 * be aborted and images are ready for WebGL.
 */
import { convertFileSrc } from "@tauri-apps/api/core";

import type { RectI } from "@/bindings/RectI";

/** URL of `path` on the `blurrd` scheme (`http://blurrd.localhost/…` on Windows). */
export function schemeUrl(path: string): string {
  return convertFileSrc(path, "blurrd");
}

/** One Exact patch: a region of interest and where it goes, in source pixels. */
export interface ExactPatchInfo {
  /** Name of the ROI (for example `killfeed` or `left`). */
  readonly roi: string;
  readonly rect: RectI;
}

/** What `blurrd://exact/<project>/<frame>.json` answers. */
export interface ExactFrame {
  /** The presentation index the backend decoded. */
  readonly frame: number;
  /** Project revision the patches were blurred with. */
  readonly revision: number;
  readonly patches: readonly ExactPatchInfo[];
}

/** A decoded Exact patch. */
export interface ExactPatch {
  readonly rect: RectI;
  readonly image: ImageBitmap;
}

async function fetchOk(url: string, signal: AbortSignal): Promise<Response> {
  const response = await fetch(url, { signal });
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  return response;
}

/** Decodes the image at `url`. */
export async function loadImage(url: string, signal: AbortSignal): Promise<ImageBitmap> {
  const blob = await (await fetchOk(url, signal)).blob();
  return createImageBitmap(blob);
}

/**
 * The Exact patches of `frame`: the decoded frame index the backend
 * reports and its blurred ROI images.
 */
export async function fetchExact(
  projectId: string,
  frame: number,
  revision: number,
  signal: AbortSignal,
): Promise<{ readonly frame: number; readonly patches: readonly ExactPatch[] }> {
  const info = (await (
    await fetchOk(schemeUrl(`exact/${projectId}/${frame}.json`), signal)
  ).json()) as ExactFrame;
  const patches = await Promise.all(
    info.patches.map(async (patch) => ({
      rect: patch.rect,
      image: await loadImage(
        `${schemeUrl(`exact/${projectId}/${info.frame}/${patch.roi}.jpg`)}?rev=${revision}`,
        signal,
      ),
    })),
  );
  return { frame: info.frame, patches };
}

/** URL of timeline thumbnail `index` of a project. */
export function thumbnailUrl(projectId: string, index: number): string {
  return schemeUrl(`thumb/${projectId}/${index}.jpg`);
}
