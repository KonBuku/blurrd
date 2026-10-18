import { describe, expect, it } from "vitest";

import { analysisRequest, runFile } from "./job-request";
import { DEFAULT_JOB_SETTINGS, orderedRegions, type JobSettings } from "./job-settings";

describe("job requests", () => {
  it("carries the default settings unchanged", () => {
    const { regions, verify } = DEFAULT_JOB_SETTINGS;
    expect(analysisRequest("p1", DEFAULT_JOB_SETTINGS)).toEqual({
      projectId: "p1",
      regions,
      verify,
    });
  });

  it("normalises the region order and carries box fit and coverage", () => {
    const settings: JobSettings = {
      ...DEFAULT_JOB_SETTINGS,
      regions: { enabled: ["MatchEnd", "KillFeed", "MatchEnd", "Banner"], mode: "Safe" },
      verify: { enabled: true, coverage: "EveryFrame" },
    };
    const request = analysisRequest("p1", settings);
    expect(request.regions).toEqual({ enabled: ["KillFeed", "Banner", "MatchEnd"], mode: "Safe" });
    expect(request.verify).toEqual({ enabled: true, coverage: "EveryFrame" });
    const file = runFile("p1", settings);
    expect(file.regions.mode).toBe("Safe");
    expect(file.verify.coverage).toBe("EveryFrame");
  });

  it("orders regions", () => {
    expect(orderedRegions(["Scoreboard", "VoiceChat", "Manual"])).toEqual([
      "VoiceChat",
      "Scoreboard",
    ]);
  });
});
