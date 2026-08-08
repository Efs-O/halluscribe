// HalluScribe - chat turn helpers for assistant token routing.

import type { TokenPayload, Turn } from "./types";

export function appendAssistantToken(turn: Turn | undefined, payload: TokenPayload): Turn | undefined {
  if (!turn || turn.role !== "assistant") return turn;
  if (payload.is_thinking) {
    return {
      ...turn,
      thinkingText: `${turn.thinkingText}${payload.text}`,
      toolActivity: null,
    };
  }
  return {
    ...turn,
    answerText: `${turn.answerText}${payload.text}`,
    toolActivity: null,
  };
}

/// Context-meter percentage for a reported usage sample. Reported at full 1%
/// resolution so the bar drifts with the conversation instead of snapping
/// between coarse buckets.
export function ctxUsedPercent(usedTokens: number, ctxSize: number): number {
  if (ctxSize <= 0) return 0;
  return Math.min(100, Math.max(0, Math.round((usedTokens / ctxSize) * 100)));
}

export function formatToolActivity(tool: string, args: Record<string, unknown>): string {
  if (tool === "web_search") {
    const query = typeof args.query === "string" ? args.query : "";
    return query ? `searching the web: ${query}` : "searching the web...";
  }
  if (tool === "web_fetch") {
    const url = typeof args.url === "string" ? args.url : "";
    return url ? `fetching page: ${url}` : "fetching page...";
  }
  if (tool === "search_sessions") {
    const query = typeof args.query === "string" ? args.query : "";
    return query ? `searching archives: ${query}` : "searching archives...";
  }
  if (tool === "search_sessions_semantic") {
    const query = typeof args.query === "string" ? args.query : "";
    return query ? `semantic search: ${query}` : "semantic search...";
  }
  if (tool === "read_session") {
    const sessionId = typeof args.session_id === "string" ? args.session_id : "";
    return sessionId ? `reading session: ${sessionId}` : "reading session...";
  }
  return `running tool: ${tool}`;
}
