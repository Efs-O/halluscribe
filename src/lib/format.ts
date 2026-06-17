// HalluScribe — display formatting helpers.
/// <reference types="vitest/importMeta" />

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
