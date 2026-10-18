import { beforeEach, describe, expect, it } from "vitest";

import {
  DEFAULT_APPEARANCE,
  applyAppearance,
  cacheAppearance,
  cachedAppearance,
  parseAppearance,
} from "./appearance";

/** A media query list whose `matches` the test can flip. */
class FakeMediaQueryList extends EventTarget {
  matches: boolean;

  constructor(matches: boolean) {
    super();
    this.matches = matches;
  }

  set(matches: boolean) {
    this.matches = matches;
    this.dispatchEvent(new Event("change"));
  }
}

function setup(systemDark: boolean) {
  const root = document.createElement("html");
  const media = new FakeMediaQueryList(systemDark);
  return { root, media, list: media as unknown as MediaQueryList };
}

describe("parseAppearance", () => {
  it("defaults to the dark theme, matching the Rust default", () => {
    expect(DEFAULT_APPEARANCE.theme).toBe("Dark");
    expect(parseAppearance({}).theme).toBe("Dark");
    // A stored choice wins over the default.
    expect(parseAppearance({ theme: "System" }).theme).toBe("System");
    expect(parseAppearance({ theme: "Light" }).theme).toBe("Light");
  });

  it("keeps valid values", () => {
    const value = { theme: "Dark", ui_font_size: 16, reduce_motion: "On", timeline_thumbs: false };
    expect(parseAppearance(value)).toEqual(value);
  });

  it("replaces missing and invalid fields with the defaults", () => {
    expect(parseAppearance(undefined)).toEqual(DEFAULT_APPEARANCE);
    expect(parseAppearance("dark")).toEqual(DEFAULT_APPEARANCE);
    expect(
      parseAppearance({ theme: "Blue", ui_font_size: 40, reduce_motion: 1, timeline_thumbs: "no" }),
    ).toEqual(DEFAULT_APPEARANCE);
    expect(parseAppearance({ ui_font_size: 13.5 }).ui_font_size).toBe(14);
    expect(parseAppearance({ theme: "Light" })).toEqual({ ...DEFAULT_APPEARANCE, theme: "Light" });
  });
});

describe("applyAppearance", () => {
  it("follows the system theme in System mode until stopped", () => {
    const { root, media, list } = setup(true);
    const stop = applyAppearance({ ...DEFAULT_APPEARANCE, theme: "System" }, root, list);
    expect(root.classList.contains("dark")).toBe(true);
    media.set(false);
    expect(root.classList.contains("dark")).toBe(false);

    stop();
    media.set(true);
    expect(root.classList.contains("dark")).toBe(false);
  });

  it("ignores the system theme when a theme is chosen", () => {
    const { root, media, list } = setup(true);
    applyAppearance({ ...DEFAULT_APPEARANCE, theme: "Light" }, root, list);
    expect(root.classList.contains("dark")).toBe(false);
    media.set(false);
    applyAppearance({ ...DEFAULT_APPEARANCE, theme: "Dark" }, root, list);
    expect(root.classList.contains("dark")).toBe(true);
    media.set(true);
    expect(root.classList.contains("dark")).toBe(true);
  });

  it("sets the interface size and the motion preference", () => {
    const { root, list } = setup(false);
    applyAppearance({ ...DEFAULT_APPEARANCE, ui_font_size: 16, reduce_motion: "On" }, root, list);
    expect(root.style.getPropertyValue("--ui-font-size")).toBe("16px");
    expect(root.dataset.motion).toBe("reduced");

    applyAppearance({ ...DEFAULT_APPEARANCE, reduce_motion: "Off" }, root, list);
    expect(root.dataset.motion).toBe("full");
    expect(root.style.getPropertyValue("--ui-font-size")).toBe("14px");

    applyAppearance(DEFAULT_APPEARANCE, root, list);
    expect(root.dataset.motion).toBeUndefined();
  });
});

describe("appearance cache", () => {
  beforeEach(() => localStorage.clear());

  it("round-trips and falls back to the defaults", () => {
    expect(cachedAppearance()).toEqual(DEFAULT_APPEARANCE);
    const light = { ...DEFAULT_APPEARANCE, theme: "Light" as const };
    cacheAppearance(light);
    expect(cachedAppearance()).toEqual(light);

    localStorage.setItem("blurrd.appearance", "{not json");
    expect(cachedAppearance()).toEqual(DEFAULT_APPEARANCE);
  });
});
