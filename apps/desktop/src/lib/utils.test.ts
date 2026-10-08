import { describe, expect, it } from "vitest";

import { cn } from "./utils";

describe("cn", () => {
  it("joins conditional class names", () => {
    const hidden = false;
    expect(cn("a", hidden && "b", { c: true, d: false }, ["e"], undefined)).toBe("a c e");
  });

  it("lets the last text-ui size win", () => {
    expect(cn("text-ui-base", "text-ui-sm")).toBe("text-ui-sm");
  });

  it("keeps a text colour next to a text-ui size", () => {
    expect(cn("text-ui-sm text-foreground", "text-foreground-subtle")).toBe(
      "text-ui-sm text-foreground-subtle",
    );
  });

  it("resolves ordinary tailwind conflicts", () => {
    expect(cn("h-7 px-2", "px-3")).toBe("h-7 px-3");
  });
});
