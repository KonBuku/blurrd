import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it } from "vitest";

import { installMockBackend, type MockBackend } from "@/ipc/mock";

import { AboutContent, BUILD, TRADEMARKS } from "./AboutContent";
import { AboutDialogs } from "./AboutDialogs";
import {
  filterComponents,
  ffmpegSource,
  loadLicenses,
  parseLicenseData,
  type LicensedComponent,
} from "./licenses";
import { ISSUES_URL, REPOSITORY_URL } from "./links";
import { useAboutStore } from "./store";

function withQuery(children: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

function component(name: string, license: string, version = "1.0.0"): LicensedComponent {
  return { kind: "npm", name, version, license, url: `https://example.org/${name}`, texts: [0] };
}

describe("licence data", () => {
  it("lists every kind of shipped component with its licence texts", async () => {
    const data = await loadLicenses();
    const kinds = new Set(data.packages.map((item) => item.kind));
    expect(kinds).toContain("binary");
    expect(kinds).toContain("crate");
    expect(kinds).toContain("npm");
    // Every component points at at least one non-empty licence text.
    for (const item of data.packages) {
      expect(item.texts.length).toBeGreaterThan(0);
      for (const index of item.texts) expect(data.texts[index]?.text.length).toBeGreaterThan(0);
    }
    const names = data.packages.map((item) => item.name);
    for (const expected of ["FFmpeg", "tauri", "react", "cmdk", "tailwindcss"]) {
      expect(names).toContain(expected);
    }
    expect(names.some((name) => name.startsWith("@radix-ui/"))).toBe(true);
    expect(ffmpegSource(data)).toMatch(
      /^https:\/\/github\.com\/KonBuku\/blurrd\/releases\/.+-source\.zip$/,
    );
  });

  it("rejects data of another format", () => {
    expect(() => parseLicenseData({ schema: "other" })).toThrow(/blurrd-licenses\/1/);
    expect(() =>
      parseLicenseData({
        schema: "blurrd-licenses/1",
        texts: [],
        packages: [component("a", "MIT")],
      }),
    ).toThrow(/malformed/);
  });

  it("filters by every word of the query in name, version and licence", () => {
    const items = [
      component("react", "MIT", "19.3.0"),
      component("zlib", "Zlib"),
      component("serde", "MIT OR Apache-2.0"),
    ];
    expect(filterComponents(items, "")).toBe(items);
    expect(filterComponents(items, "mit").map((item) => item.name)).toEqual(["react", "serde"]);
    expect(filterComponents(items, "MIT apache").map((item) => item.name)).toEqual(["serde"]);
    expect(filterComponents(items, "19.3").map((item) => item.name)).toEqual(["react"]);
    expect(filterComponents(items, "nothing")).toEqual([]);
  });
});

describe("about", () => {
  let backend: MockBackend;

  beforeEach(() => {
    backend = installMockBackend();
    useAboutStore.setState({ open: null });
  });

  it("shows version, commit, FFmpeg and the trademark statements", async () => {
    render(withQuery(<AboutContent heading={<h2>blurrd</h2>} />));
    expect(BUILD.version).toMatch(/^\d+\.\d+\.\d+/);
    expect(screen.getByText(BUILD.version)).toBeTruthy();
    expect(screen.getByText(BUILD.commit)).toBeTruthy();
    expect(screen.getByText(TRADEMARKS)).toBeTruthy();
    expect(TRADEMARKS).toContain("not affiliated with");
    expect(await screen.findByText(backend.capabilities.ffmpeg_version)).toBeTruthy();
    fireEvent.click(await screen.findByRole("button", { name: /Source code/ }));
    fireEvent.click(screen.getByRole("button", { name: /GitHub/ }));
    fireEvent.click(screen.getByRole("button", { name: /Report an issue/ }));
    await waitFor(() => expect(backend.opened).toHaveLength(3));
    expect(backend.opened[0]).toMatch(/-source\.zip$/);
    expect(backend.opened.slice(1)).toEqual([REPOSITORY_URL, ISSUES_URL]);
  });

  it("opens the licences, searches them and shows a licence text", async () => {
    render(withQuery(<AboutContent heading={<h2>blurrd</h2>} />));
    fireEvent.click(screen.getByRole("button", { name: "Open-source licenses" }));
    const dialog = await screen.findByRole("dialog", { name: "Open-source licenses" });
    const search = await within(dialog).findByRole("searchbox", { name: "Search the components" });
    expect(within(dialog).getByRole("region", { name: "Rust crates" })).toBeTruthy();
    fireEvent.change(search, { target: { value: "ffmpeg lgpl" } });
    const row = await within(dialog).findByRole("button", { name: /^FFmpeg/ });
    expect(within(dialog).queryByRole("region", { name: "Rust crates" })).toBeNull();
    expect(row.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("true");
    expect(within(dialog).getByText(/GNU LESSER GENERAL PUBLIC LICENSE/)).toBeTruthy();
    expect(
      within(dialog).getByRole("button", { name: /Download the corresponding source code/ }),
    ).toBeTruthy();
  });

  it("opens from the store without a trigger", async () => {
    render(withQuery(<AboutDialogs />));
    act(() => useAboutStore.getState().show("about"));
    expect(await screen.findByRole("dialog", { name: "blurrd" })).toBeTruthy();
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    await waitFor(() => expect(useAboutStore.getState().open).toBeNull());
    act(() => useAboutStore.getState().show("licenses"));
    expect(await screen.findByRole("dialog", { name: "Open-source licenses" })).toBeTruthy();
  });
});
