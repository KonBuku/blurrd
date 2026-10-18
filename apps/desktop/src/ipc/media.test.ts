import { QueryClient } from "@tanstack/react-query";
import { clearMocks, mockConvertFileSrc } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it, vi } from "vitest";

import { busyRetryDelay, HttpError, loadObjectUrl, posterUrl, retryWhenBusy } from "./media";

/** Loads `url` the way the timeline loads a thumbnail. */
function loadThumbnail(url: string): Promise<string> {
  const client = new QueryClient();
  return client.fetchQuery({
    queryKey: ["thumbnail", url],
    queryFn: ({ signal }) => loadObjectUrl(url, signal),
    retry: retryWhenBusy,
    retryDelay: (failures) => busyRetryDelay(failures) / 50,
  });
}

/** Answers the fetches with `statuses` in turn. */
function answer(...statuses: number[]) {
  const fetch = vi.fn<() => Promise<Response>>(async () => {
    const status = statuses.shift() ?? 500;
    return new Response(status === 200 ? new Blob(["jpeg"]) : "busy", { status });
  });
  vi.stubGlobal("fetch", fetch);
  return fetch;
}

describe("scheme request retries", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("retries only a busy backend, twice at most", () => {
    const busy = new HttpError("u", 503);
    expect(retryWhenBusy(0, busy)).toBe(true);
    expect(retryWhenBusy(1, busy)).toBe(true);
    expect(retryWhenBusy(2, busy)).toBe(false);
    expect(retryWhenBusy(0, new HttpError("u", 404))).toBe(false);
    expect(retryWhenBusy(0, new Error("aborted"))).toBe(false);
    expect([0, 1].map(busyRetryDelay)).toEqual([250, 500]);
  });

  it("loads a thumbnail that was busy twice", async () => {
    const fetch = answer(503, 503, 200);
    await expect(loadThumbnail("thumb/a")).resolves.toMatch(/^blob:/);
    expect(fetch).toHaveBeenCalledTimes(3);
  });

  it("gives up after two retries and at once on other errors", async () => {
    const busy = answer(503, 503, 503, 200);
    await expect(loadThumbnail("thumb/b")).rejects.toMatchObject({ status: 503 });
    expect(busy).toHaveBeenCalledTimes(3);

    const missing = answer(404, 200);
    await expect(loadThumbnail("thumb/c")).rejects.toMatchObject({ status: 404 });
    expect(missing).toHaveBeenCalledTimes(1);
  });
});

describe("poster URLs", () => {
  afterEach(() => clearMocks());

  // The backend route (`scheme::Route::Poster`) decodes the query value.
  it("carry the encoded file path", () => {
    mockConvertFileSrc("windows");
    expect(posterUrl(String.raw`C:\clips\a b#1&2.mp4`)).toBe(
      "http://blurrd.localhost/poster.jpg?path=C%3A%5Cclips%5Ca%20b%231%262.mp4",
    );
  });
});
