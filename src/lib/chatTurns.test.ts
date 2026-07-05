// HalluScribe - tests for assistant chat token routing helpers.

import { describe, expect, it } from "vitest";
import { appendAssistantToken } from "./chatTurns";
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
