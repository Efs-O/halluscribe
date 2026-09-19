// HalluScribe - pure logic for the "Business messages" settings section (Phase
// 9b-UI). Kept free of Tauri/Svelte so the wording and gating are testable
// without a host. Mirrors the Rust `business_*` settings fields.

/** The fixed sensitive-data warning shown above the business controls. It is
 *  copy, not state, so it lives here and is asserted in a test. */
export function sensitiveDataWarning(): string {
  return (
    "Business chats are customer data: names, phone numbers, prices and orders. " +
    "They are read from your iPhone backup, summarised locally, and kept in this " +
    "workspace only — they never reach the MCP or your personal archive."
  );
}

/** Whether the "Run import now" button is enabled. It is the UI half of the
 *  one-consent gate: disabled while the toggle is off (the Rust command
 *  refuses too, so the gate cannot be bypassed). */
export function runImportEnabled(businessIngestionEnabled: boolean): boolean {
  return businessIngestionEnabled;
}

/** Format an ISO-8601 last-import timestamp for display, or "never" when empty
 *  or unparseable. Rendered in the viewer's local time. */
export function formatLastImport(iso: string): string {
  const trimmed = iso.trim();
  if (trimmed === "") return "never";
  const date = new Date(trimmed);
  if (Number.isNaN(date.getTime())) return "never";
  return date.toLocaleString("en-GB", {
    day: "2-digit",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

/** The one status line under the business controls. A failed import (or an
 *  unsupported-schema note) is the thing to surface first, in a warning tone;
 *  otherwise the last successful import is shown, or "never" if there is none. */
export function businessImportStatus(
  lastImport: string,
  lastError: string,
): { text: string; tone: "warn" | "ok" | "quiet" } {
  const error = lastError.trim();
  if (error !== "") {
    return { text: `Last import failed: ${error}`, tone: "warn" };
  }
  const importTime = formatLastImport(lastImport);
  if (importTime === "never") {
    return { text: "Not imported yet.", tone: "quiet" };
  }
  return { text: `Last import: ${importTime}`, tone: "ok" };
}

// --- Tests -------------------------------------------------------------------

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest;

  describe("sensitiveDataWarning", () => {
    it("names the data and where it stays", () => {
      const text = sensitiveDataWarning();
      expect(text).toContain("customer data");
      expect(text).toContain("MCP");
      expect(text).toContain("personal archive");
    });
  });

  describe("runImportEnabled", () => {
    it("is enabled only when the toggle is on", () => {
      expect(runImportEnabled(true)).toBe(true);
      expect(runImportEnabled(false)).toBe(false);
    });
  });

  describe("formatLastImport", () => {
    it("returns 'never' for an empty value", () => {
      expect(formatLastImport("")).toBe("never");
      expect(formatLastImport("   ")).toBe("never");
    });
    it("returns 'never' for an unparseable value", () => {
      expect(formatLastImport("not-a-date")).toBe("never");
    });
    it("formats a real ISO timestamp", () => {
      expect(formatLastImport("2026-09-19T06:00:00+00:00")).toMatch(/2026/);
    });
  });

  describe("businessImportStatus", () => {
    it("shows a failed import in a warning tone", () => {
      const status = businessImportStatus("2026-09-19T06:00:00+00:00", "unsupported schema");
      expect(status.tone).toBe("warn");
      expect(status.text).toContain("unsupported schema");
    });
    it("shows the last import when there is no error", () => {
      const status = businessImportStatus("2026-09-19T06:00:00+00:00", "");
      expect(status.tone).toBe("ok");
      expect(status.text).toContain("Last import:");
    });
    it("shows 'not imported yet' when nothing has run", () => {
      const status = businessImportStatus("", "");
      expect(status.tone).toBe("quiet");
      expect(status.text).toBe("Not imported yet.");
    });
  });
}
