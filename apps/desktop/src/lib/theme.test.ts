import { describe, expect, it } from "vitest";

import { followSystemTheme } from "./theme";

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

describe("followSystemTheme", () => {
  it("applies the current system theme and follows changes", () => {
    const root = document.createElement("html");
    const media = new FakeMediaQueryList(true);
    const stop = followSystemTheme(root, media as unknown as MediaQueryList);
    expect(root.classList.contains("dark")).toBe(true);

    media.set(false);
    expect(root.classList.contains("dark")).toBe(false);

    stop();
    media.set(true);
    expect(root.classList.contains("dark")).toBe(false);
  });
});
