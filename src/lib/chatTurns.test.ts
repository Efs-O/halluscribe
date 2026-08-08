// HalluScribe - tests for assistant chat token routing helpers.

import { describe, expect, it } from "vitest";
import { appendAssistantToken, ctxUsedPercent, formatToolActivity } from "./chatTurns";
import type { Turn } from "./types";

function assistantTurn(): Turn {
  return {
    id: "t1",
    role: "assistant",
    thinkingText: "",
    answerText: "",
    toolActivity: "searching archives...",
    streaming: true,
  };
}

describe("appendAssistantToken", () => {
  it("routes thinking tokens into thinkingText", () => {
    const next = appendAssistantToken(assistantTurn(), { text: "plan", is_thinking: true });
    expect(next?.thinkingText).toBe("plan");
    expect(next?.answerText).toBe("");
    expect(next?.toolActivity).toBeNull();
  });

  it("routes answer tokens into answerText", () => {
    const next = appendAssistantToken(assistantTurn(), { text: "reply", is_thinking: false });
    expect(next?.thinkingText).toBe("");
    expect(next?.answerText).toBe("reply");
    expect(next?.toolActivity).toBeNull();
  });

  it("ignores non-assistant turns", () => {
    const next = appendAssistantToken(
      {
        id: "t2",
        role: "user",
        thinkingText: "",
        answerText: "hello",
        toolActivity: null,
        streaming: false,
      },
      { text: "ignored", is_thinking: true },
    );
    expect(next?.answerText).toBe("hello");
    expect(next?.thinkingText).toBe("");
  });
});

describe("formatToolActivity", () => {
  it("includes the useful argument for known tools", () => {
    expect(formatToolActivity("web_search", { query: "Gemma 4" }))
      .toBe("searching the web: Gemma 4");
    expect(formatToolActivity("web_fetch", { url: "https://example.com" }))
      .toBe("fetching page: https://example.com");
    expect(formatToolActivity("search_sessions", { query: "scheduler" }))
      .toBe("searching archives: scheduler");
    expect(formatToolActivity("search_sessions_semantic", { query: "nightly sweep" }))
      .toBe("semantic search: nightly sweep");
    expect(formatToolActivity("read_session", { session_id: "session-1" }))
      .toBe("reading session: session-1");
  });

  it("uses stable fallback text for missing and unknown arguments", () => {
    expect(formatToolActivity("web_search", {})).toBe("searching the web...");
    expect(formatToolActivity("custom_tool", {})).toBe("running tool: custom_tool");
  });
});

describe("ctxUsedPercent", () => {
  it("reports whole-percent resolution instead of ten-point buckets", () => {
    expect(ctxUsedPercent(3400, 10_000)).toBe(34);
    expect(ctxUsedPercent(3500, 10_000)).toBe(35);
    expect(ctxUsedPercent(3549, 10_000)).toBe(35);
  });

  it("clamps to the window and treats an unknown window as empty", () => {
    expect(ctxUsedPercent(12_000, 10_000)).toBe(100);
    expect(ctxUsedPercent(-5, 10_000)).toBe(0);
    expect(ctxUsedPercent(4_000, 0)).toBe(0);
  });
});
