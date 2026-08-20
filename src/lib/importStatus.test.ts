// HalluScribe - tests for the import-path diagnostic wording.

import { describe, it, expect } from "vitest";
import {
  importStatusTone,
  importStatusMessage,
  importStatusDetail,
  importStatusSummary,
} from "./importStatus";
import type { ImportPathStatus, ImportPathState } from "./types";

function status(state: ImportPathState, extra: Partial<ImportPathStatus> = {}): ImportPathStatus {
  return {
    provider: "grok",
    label: "Grok",
    path: "C:/archive/imports/grok",
    state,
    file_count: 0,
    total_bytes: 0,
    files: [],
    ...extra,
  };
}

describe("importStatusTone", () => {
  it("treats an unused provider as quiet, not as a problem", () => {
    expect(importStatusTone(status("unset"))).toBe("quiet");
  });

  it("warns only when a path is set and yields nothing", () => {
    expect(importStatusTone(status("no_export_found"))).toBe("warn");
    expect(importStatusTone(status("missing_folder"))).toBe("warn");
    expect(importStatusTone(status("found", { file_count: 1 }))).toBe("ok");
  });
});

describe("importStatusMessage", () => {
  it("tells the user what to do about an empty folder, not just that it is empty", () => {
    const message = importStatusMessage(status("no_export_found"));
    expect(message).toContain("Unpack");
    expect(message).toContain("nested too deep");
  });

  it("distinguishes a missing folder from an empty one", () => {
    expect(importStatusMessage(status("missing_folder"))).toContain("does not exist");
  });

  it("reports a single export in the singular, with its size", () => {
    const message = importStatusMessage(
      status("found", { file_count: 1, total_bytes: 120050488 }),
    );
    expect(message).toBe("Found 1 export file (114.5 MB).");
  });

  it("reports split exports in the plural with a combined size", () => {
    const message = importStatusMessage(status("found", { file_count: 5, total_bytes: 2048 }));
    expect(message).toContain("5 export files");
    expect(message).toContain("total");
  });

  it("says a blank path is skipped rather than broken", () => {
    expect(importStatusMessage(status("unset"))).toContain("skipped");
  });
});

describe("importStatusDetail", () => {
  it("shows which file answered, so a stale copy is visible", () => {
    const detail = importStatusDetail(
      status("found", { file_count: 1, files: ["C:/archive/old-backup/conversations.json"] }),
    );
    expect(detail).toBe("C:/archive/old-backup/conversations.json");
  });

  it("shows nothing when nothing was found", () => {
    expect(importStatusDetail(status("no_export_found"))).toBe("");
  });
});

describe("importStatusSummary", () => {
  it("is empty when every provider is healthy or unused", () => {
    expect(importStatusSummary([status("found", { file_count: 1 }), status("unset")])).toBe("");
  });

  it("names the single provider that needs attention", () => {
    const summary = importStatusSummary([
      status("found", { provider: "chatgpt", label: "ChatGPT", file_count: 1 }),
      status("no_export_found"),
    ]);
    expect(summary).toBe("Grok has a path set but nothing to import.");
  });

  it("names every failing provider when there are several", () => {
    const summary = importStatusSummary([
      status("no_export_found", { provider: "chatgpt", label: "ChatGPT" }),
      status("missing_folder"),
    ]);
    expect(summary).toBe("ChatGPT, Grok have paths set but nothing to import.");
  });
});
