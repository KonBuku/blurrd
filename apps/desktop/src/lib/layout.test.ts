import { describe, expect, it } from "vitest";

import {
  INSPECTOR,
  INSPECTOR_SHEET_BREAKPOINT,
  PREVIEW_MIN,
  SIDEBAR,
  clamp,
  resolveLayout,
  type LayoutPreferences,
} from "./layout";

const DEFAULTS: LayoutPreferences = {
  sidebarOpen: true,
  sidebarWidth: SIDEBAR.default,
  inspectorOpen: true,
  inspectorWidth: INSPECTOR.default,
};

/** Width left for the preview frame, gaps included. */
function previewWidth(windowWidth: number, prefs: LayoutPreferences): number {
  const layout = resolveLayout(windowWidth, prefs);
  return windowWidth - layout.sidebarWidth - (layout.inspectorAsSheet ? 0 : layout.inspectorWidth);
}

describe("clamp", () => {
  it("keeps values inside the range and lets the minimum win", () => {
    expect(clamp(5, 0, 10)).toBe(5);
    expect(clamp(-1, 0, 10)).toBe(0);
    expect(clamp(11, 0, 10)).toBe(10);
    expect(clamp(5, 8, 4)).toBe(8);
  });
});

describe("resolveLayout", () => {
  it("uses the default widths at the default window size", () => {
    expect(resolveLayout(1440, DEFAULTS)).toEqual({
      inspectorAsSheet: false,
      sidebarWidth: 280,
      sidebarMax: 576,
      inspectorWidth: 320,
      inspectorMax: 420,
    });
  });

  it("turns the inspector into a sheet below the breakpoint", () => {
    expect(resolveLayout(INSPECTOR_SHEET_BREAKPOINT - 1, DEFAULTS).inspectorAsSheet).toBe(true);
    expect(resolveLayout(INSPECTOR_SHEET_BREAKPOINT, DEFAULTS).inspectorAsSheet).toBe(false);
    // The sheet keeps the preferred width even when the frame is hidden.
    const sheet = resolveLayout(1024, { ...DEFAULTS, inspectorOpen: false, inspectorWidth: 360 });
    expect(sheet.inspectorWidth).toBe(360);
  });

  it("hides the sidebar and the inspector frame when they are closed", () => {
    const layout = resolveLayout(1440, { ...DEFAULTS, sidebarOpen: false, inspectorOpen: false });
    expect(layout.sidebarWidth).toBe(0);
    expect(layout.inspectorWidth).toBe(0);
    expect(layout.sidebarMax).toBe(576);
  });

  it("caps the sidebar at 40 % of the window", () => {
    const layout = resolveLayout(2560, { ...DEFAULTS, sidebarWidth: 5000 });
    expect(layout.sidebarWidth).toBe(1024);
  });

  it("clamps preferences outside the limits", () => {
    const narrow = resolveLayout(1440, { ...DEFAULTS, sidebarWidth: 10, inspectorWidth: 10 });
    expect(narrow.sidebarWidth).toBe(SIDEBAR.min);
    expect(narrow.inspectorWidth).toBe(INSPECTOR.min);
    const wide = resolveLayout(1440, { ...DEFAULTS, inspectorWidth: 9999 });
    expect(wide.inspectorWidth).toBe(INSPECTOR.max);
  });

  it("keeps the preview usable from 1024 to 2560 px with any preference", () => {
    const extremes = [0, 264, 300, 420, 800, 5000];
    for (let windowWidth = 1024; windowWidth <= 2560; windowWidth += 8) {
      for (const sidebarWidth of extremes) {
        for (const inspectorWidth of extremes) {
          const prefs = { ...DEFAULTS, sidebarWidth, inspectorWidth };
          const layout = resolveLayout(windowWidth, prefs);
          expect(layout.sidebarWidth).toBeGreaterThanOrEqual(SIDEBAR.min);
          expect(layout.inspectorWidth).toBeGreaterThanOrEqual(INSPECTOR.min);
          expect(layout.inspectorWidth).toBeLessThanOrEqual(INSPECTOR.max);
          expect(previewWidth(windowWidth, prefs)).toBeGreaterThanOrEqual(PREVIEW_MIN);
        }
      }
    }
  });
});
