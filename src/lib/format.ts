// HalluScribe — display formatting helpers.
/// <reference types="vitest/importMeta" />

/** Byte count as a short human-readable size, e.g. `182.3 MB`. */
export function formatBytes(bytes: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = Math.max(0, bytes);
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

/**
 * Returns the CSS class name for the fill % colour band.
 * green < 40 %, amber 40–70 %, red > 70 % (mirrors HalluMeter ring states).
 */
export function fillClass(pct: number): string {
  if (pct > 70) return "fill-red";
  if (pct >= 40) return "fill-amber";
  return "fill-green";
}

/** Format fill % for display, prefixing imported heuristic values with "~". */
export function displayFillPct(pct: number, estimated = false): string {
  const rounded = `${pct.toFixed(0)}%`;
  return estimated ? `~${rounded}` : rounded;
}

/**
 * Output tokens for the session table: compact, and marked when inferred.
 *
 * The tilde is not cosmetic. A measured count includes hidden thinking tokens
 * and tool arguments the server counted; an estimate derived from characters
 * runs several-fold low, and a chat-export estimate cannot see thinking at all.
 * An em dash means the session predates the column and has no value yet.
 */
export function displayTokens(tokens: number | undefined, estimated = true): string {
  if (!tokens) return "—";
  const prefix = estimated ? "~" : "";
  if (tokens >= 1_000_000) return `${prefix}${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 1_000) return `${prefix}${(tokens / 1_000).toFixed(1)}K`;
  return `${prefix}${tokens}`;
}

/** Extract HH:MM from an archive_path filename like "13-47-56-claudecode-sweep.md". */
export function timeFromPath(archivePath: string): string {
  const file = archivePath.split("/").pop() ?? "";
  const parts = file.split("-");
  if (parts.length >= 3) return `${parts[0]}:${parts[1]}`;
  return "";
}

export function timeFromSession(
  sessionTimestamp?: string,
  archivePath?: string,
): string {
  if (sessionTimestamp) {
    const date = new Date(sessionTimestamp);
    if (!Number.isNaN(date.getTime())) {
      return date.toLocaleTimeString("en-GB", {
        hour: "2-digit",
        minute: "2-digit",
        hour12: false,
        timeZone: "UTC",
      });
    }
  }
  return timeFromPath(archivePath ?? "");
}

/** Format a YYYY-MM-DD date string as "15 Apr 2026". */
export function shortDate(iso: string): string {
  const [y, m, d] = iso.split("-").map(Number);
  const date = new Date(y ?? 2026, (m ?? 1) - 1, d ?? 1);
  return date.toLocaleDateString("en-GB", { day: "2-digit", month: "short", year: "numeric" });
}

// --- Tests -------------------------------------------------------------------

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest;

  describe("formatBytes", () => {
    it("keeps whole bytes",        () => expect(formatBytes(512)).toBe("512 B"));
    it("steps up at 1024",         () => expect(formatBytes(1024)).toBe("1.0 KB"));
    it("formats megabytes",        () => expect(formatBytes(191_150_161)).toBe("182.3 MB"));
    it("formats gigabytes",        () => expect(formatBytes(2_545_341_726)).toBe("2.4 GB"));
    it("clamps negatives to zero", () => expect(formatBytes(-5)).toBe("0 B"));
  });

  describe("displayTokens", () => {
    it("marks a measured count plainly",  () => expect(displayTokens(884, false)).toBe("884"));
    it("marks an estimate with a tilde",  () => expect(displayTokens(884, true)).toBe("~884"));
    it("compacts thousands",              () => expect(displayTokens(209_440, false)).toBe("209.4K"));
    it("compacts millions",               () => expect(displayTokens(2_450_000, false)).toBe("2.5M"));
    it("dashes a session with no value",  () => expect(displayTokens(undefined)).toBe("—"));
    // Archived before the column existed: zero is absence, not a measured zero.
    it("dashes a zero rather than showing 0", () => expect(displayTokens(0, false)).toBe("—"));
    it("defaults to estimated",           () => expect(displayTokens(100)).toBe("~100"));
  });

  describe("fillClass", () => {
    it("green below 40", () => expect(fillClass(39)).toBe("fill-green"));
    it("amber at 40",    () => expect(fillClass(40)).toBe("fill-amber"));
    it("amber at 70",    () => expect(fillClass(70)).toBe("fill-amber"));
    it("red above 70",   () => expect(fillClass(71)).toBe("fill-red"));
  });

  describe("displayFillPct", () => {
    it("formats exact fill without prefix", () => {
      expect(displayFillPct(42.4, false)).toBe("42%");
    });

    it("formats estimated fill with tilde prefix", () => {
      expect(displayFillPct(42.4, true)).toBe("~42%");
    });
  });

  describe("shortDate", () => {
    it("formats correctly", () => expect(shortDate("2026-04-15")).toMatch(/15/));
  });

  describe("timeFromSession", () => {
    it("uses session timestamp when present", () => {
      expect(timeFromSession("2026-04-15T14:30:00Z", "09-00-00-x.md")).toBe("14:30");
    });

    it("falls back to archive path", () => {
      expect(timeFromSession(undefined, "sessions/p/2026-04-15/09-07-00-codex-sweep.md")).toBe("09:07");
    });
  });
}
