import { describe, expect, it } from "vitest";

import { analysisRequest } from "./job-request";
import { DEFAULT_JOB_SETTINGS, orderedRegions, parseWordList } from "./job-settings";

describe("job requests", () => {
  it("carries the default settings unchanged", () => {
    const { regions, detector, verify } = DEFAULT_JOB_SETTINGS;
    expect(analysisRequest("p1", DEFAULT_JOB_SETTINGS)).toEqual({
      projectId: "p1",
      regions,
      detector,
      verify,
    });
  });

  it("normalises region order and word lists", () => {
    const request = analysisRequest("p1", {
      ...DEFAULT_JOB_SETTINGS,
      regions: {
        ...DEFAULT_JOB_SETTINGS.regions,
        enabled: ["MatchEnd", "KillFeed", "MatchEnd", "Banner"],
        keep_names: { own_row: false, names: [" Ally ", "Ally", ""] },
      },
      verify: { ...DEFAULT_JOB_SETTINGS.verify, ignore_words: ["ACE", "ACE", " Haven"] },
    });
    expect(request.regions.enabled).toEqual(["KillFeed", "Banner", "MatchEnd"]);
    expect(request.regions.keep_names.names).toEqual(["Ally"]);
    expect(request.verify.ignore_words).toEqual(["ACE", "Haven"]);
  });

  it("parses typed word lists and orders regions", () => {
    expect(parseWordList("a, b;c\n a ,,")).toEqual(["a", "b", "c"]);
    expect(orderedRegions(["Scoreboard", "VoiceChat", "Manual"])).toEqual([
      "VoiceChat",
      "Scoreboard",
    ]);
  });
});
