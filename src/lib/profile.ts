// HalluScribe — pure helpers for the PROFILE panel (profile.md rendering).
/// <reference types="vitest/importMeta" />

import type { ProfileProgressPayload } from "./types";

/** Human-readable label for the current refresh stage/progress. */
export function stageLabel(p: ProfileProgressPayload): string {
  if (p.stage === "writing") return "Writing…";
  // The merge is one model call per consolidate chunk and per section — dozens
  // of them on a full archive — so report the step. Small archives merge in a
  // single call and keep the bare label rather than saying "step 1 of 1".
  if (p.stage === "merging") {
    return p.total > 1 ? `Merging step ${p.current} of ${p.total}…` : "Merging…";
  }
  return `Mapping batch ${p.current} of ${p.total}…`;
}

/** The generated-by header line of profile.md (its first line). */
export function firstLine(text: string): string {
  return text.split("\n", 1)[0] ?? "";
}

/** profile.md's body, with the header line and any blank lines after it stripped. */
export function restOfFile(text: string): string {
  const idx = text.indexOf("\n");
  return idx === -1 ? "" : text.slice(idx + 1).replace(/^\n+/, "");
}

// --- Tests -------------------------------------------------------------------

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest;

  describe("stageLabel", () => {
    it("labels the merging stage", () => {
      expect(stageLabel({ current: 1, total: 1, stage: "merging", scope: "work" })).toBe(
        "Merging…",
      );
    });

    it("labels merging with the step when the reduce takes several calls", () => {
      expect(stageLabel({ current: 7, total: 45, stage: "merging", scope: "work" })).toBe(
        "Merging step 7 of 45…",
      );
    });

    it("labels the writing stage", () => {
      expect(stageLabel({ current: 1, total: 1, stage: "writing", scope: "work" })).toBe(
        "Writing…",
      );
    });

    it("labels mapping with batch progress", () => {
      expect(stageLabel({ current: 3, total: 10, stage: "mapping", scope: "personal" })).toBe(
        "Mapping batch 3 of 10…",
      );
    });
  });

  describe("firstLine", () => {
    it("returns the header line", () => {
      expect(firstLine("# User Profile — v1\n\n## Identity\n\ntext")).toBe(
        "# User Profile — v1",
      );
    });

    it("returns the whole string when there is no newline", () => {
      expect(firstLine("solo line")).toBe("solo line");
    });
  });

  describe("restOfFile", () => {
    it("strips the header line and leading blank lines", () => {
      expect(restOfFile("# header\n\n\n## Identity\n\ntext")).toBe("## Identity\n\ntext");
    });

    it("returns empty string when there is no newline", () => {
      expect(restOfFile("solo line")).toBe("");
    });
  });
}
