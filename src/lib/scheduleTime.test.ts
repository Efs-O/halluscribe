// HalluScribe - tests for scheduled sweep time normalization.

import { describe, expect, it } from "vitest";
import { normalizeScheduleTime } from "./scheduleTime";

describe("normalizeScheduleTime", () => {
  it("preserves valid padded 24-hour times", () => {
    expect(normalizeScheduleTime("18:30")).toBe("18:30");
    expect(normalizeScheduleTime(" 02:00 ")).toBe("02:00");
  });

  it("pads compact hour and minute values", () => {
    expect(normalizeScheduleTime("2:5")).toBe("02:05");
    expect(normalizeScheduleTime("0:0")).toBe("00:00");
  });

  it("rejects out-of-range or malformed values", () => {
    expect(normalizeScheduleTime("24:00")).toBeNull();
    expect(normalizeScheduleTime("12:60")).toBeNull();
    expect(normalizeScheduleTime("noon")).toBeNull();
  });
});
