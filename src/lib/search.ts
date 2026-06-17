// HalluScribe — client-side session filter (SESSION SUMMARY search bar).
/// <reference types="vitest/importMeta" />
// Pure functions — no Tauri calls. All filtering done in-memory on the loaded
// IndexEntry list so the search bar is instant with no round-trips.

import type { IndexEntry } from "./types";

export type SessionSortKey = "date" | "title" | "project" | "tool" | "fill" | "tags";
export type SessionSortDirection = "asc" | "desc";

/**
 * Filter a list of sessions by query text and optional fill% range.
 * Query matches (case-insensitive) against: title, project, tool, provider, error_tags, topic_tags.
 * fillMin / fillMax are inclusive bounds; omit either to leave that end open.
 */
export function filterSessions(
  sessions: IndexEntry[],
  query: string,
  fillMin?: number,
  fillMax?: number,
): IndexEntry[] {
  const q = query.trim().toLowerCase();
  return sessions.filter((s) => {
    if (q) {
      const match =
        s.title.toLowerCase().includes(q) ||
        s.project.toLowerCase().includes(q) ||
        s.tool.toLowerCase().includes(q) ||
        (s.provider ?? "").toLowerCase().includes(q) ||
        s.error_tags.some((t) => t.toLowerCase().includes(q)) ||
        s.topic_tags.some((t) => t.toLowerCase().includes(q));
      if (!match) return false;
    }
    if (fillMin !== undefined && s.fill_pct < fillMin) return false;
    if (fillMax !== undefined && s.fill_pct > fillMax) return false;
    return true;
  });
}

export function sortSessions(
  sessions: IndexEntry[],
  key: SessionSortKey,
  direction: SessionSortDirection,
): IndexEntry[] {
  const sorted = [...sessions].sort((a, b) => compareSessions(a, b, key));
  return direction === "asc" ? sorted : sorted.reverse();
}

function compareSessions(a: IndexEntry, b: IndexEntry, key: SessionSortKey): number {
  switch (key) {
    case "date":
      return compareText(dateSortValue(a), dateSortValue(b));
    case "title":
      return compareText(a.title, b.title);
    case "project":
      return compareText(a.project, b.project);
    case "tool":
      return compareText(a.tool, b.tool);
    case "fill":
      return a.fill_pct - b.fill_pct || compareText(a.title, b.title);
    case "tags":
      return compareText(joinTags(a), joinTags(b));
  }
}

function dateSortValue(session: IndexEntry): string {
  return session.updated_at || session.session_timestamp || session.date;
}

function joinTags(session: IndexEntry): string {
  return [...session.error_tags, ...session.topic_tags].join(" ");
}

function compareText(a: string, b: string): number {
  return a.localeCompare(b, undefined, { sensitivity: "base", numeric: true });
}

// --- Tests -------------------------------------------------------------------

if (import.meta.vitest) {
  const { describe, it, expect } = import.meta.vitest;

  const s = (title: string, project = "proj", tags: string[] = []): IndexEntry => ({
    id: title,
    project,
    date: "2026-04-15",
    title,
    tool: "Claude Code",
    fill_pct: 60,
    session_type: "debugging",
    error_tags: tags,
    topic_tags: [],
    archive_path: "",
    source_jsonl: "",
  });

  describe("filterSessions", () => {
    it("empty query returns all", () => {
      const sessions = [s("Auth fix"), s("Codex run")];
      expect(filterSessions(sessions, "")).toHaveLength(2);
    });

    it("matches title case-insensitively", () => {
      const sessions = [s("Auth fix"), s("Codex run")];
      expect(filterSessions(sessions, "AUTH")).toHaveLength(1);
    });

    it("matches project", () => {
      const sessions = [s("A", "halluscribe"), s("B", "other")];
      expect(filterSessions(sessions, "hallus")).toHaveLength(1);
    });

    it("matches error tags", () => {
      const sessions = [s("A", "p", ["borrow-checker"]), s("B", "p", [])];
      expect(filterSessions(sessions, "borrow")).toHaveLength(1);
    });

    it("matches tool", () => {
      const sessions = [
        { ...s("A"), tool: "ChatGPT" },
        { ...s("B"), tool: "Claude Code" },
      ];
      expect(filterSessions(sessions, "chatgpt")).toHaveLength(1);
    });

    it("matches provider key", () => {
      const sessions = [
        { ...s("A"), provider: "gemini" },
        { ...s("B"), provider: "claude_code" },
      ];
      expect(filterSessions(sessions, "gemini")).toHaveLength(1);
    });

    it("no match returns empty", () => {
      const sessions = [s("Auth fix")];
      expect(filterSessions(sessions, "xyz-not-found")).toHaveLength(0);
    });
  });

  describe("filterSessions fill% range", () => {
    const withFill = (fill: number) => ({ ...s("T"), fill_pct: fill });

    it("fillMin excludes sessions below", () => {
      const sessions = [withFill(30), withFill(60), withFill(90)];
      expect(filterSessions(sessions, "", 50)).toHaveLength(2);
    });

    it("fillMax excludes sessions above", () => {
      const sessions = [withFill(30), withFill(60), withFill(90)];
      expect(filterSessions(sessions, "", undefined, 70)).toHaveLength(2);
    });

    it("fillMin + fillMax narrows range inclusively", () => {
      const sessions = [withFill(30), withFill(60), withFill(90)];
      expect(filterSessions(sessions, "", 50, 70)).toHaveLength(1);
    });

    it("no fill filter returns all", () => {
      const sessions = [withFill(10), withFill(99)];
      expect(filterSessions(sessions, "")).toHaveLength(2);
    });
  });

  describe("sortSessions", () => {
    it("sorts by title ascending", () => {
      const sessions = [s("Beta"), s("alpha"), s("Gamma")];
      expect(sortSessions(sessions, "title", "asc").map((session) => session.title))
        .toEqual(["alpha", "Beta", "Gamma"]);
    });

    it("sorts by fill descending", () => {
      const sessions = [{ ...s("A"), fill_pct: 10 }, { ...s("B"), fill_pct: 80 }];
      expect(sortSessions(sessions, "fill", "desc").map((session) => session.title))
        .toEqual(["B", "A"]);
    });

    it("sorts by updated/session date descending", () => {
      const sessions = [
        { ...s("Older"), date: "2026-04-15", session_timestamp: "2026-04-15T10:00:00Z" },
        { ...s("Newer"), date: "2026-04-14", updated_at: "2026-04-16T09:00:00Z" },
      ];
      expect(sortSessions(sessions, "date", "desc").map((session) => session.title))
        .toEqual(["Newer", "Older"]);
    });
  });
}
