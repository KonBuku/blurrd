import { describe, expect, it } from "vitest";

import { DEFAULT_GENERAL, parseGeneral } from "./general";

describe("parseGeneral", () => {
  it("fills missing and invalid fields with defaults", () => {
    expect(parseGeneral(undefined)).toEqual(DEFAULT_GENERAL);
    expect(parseGeneral({ restore_queue: false, notify: "yes" })).toEqual({
      ...DEFAULT_GENERAL,
      restore_queue: false,
    });
  });
});
