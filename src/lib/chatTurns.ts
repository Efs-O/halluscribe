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
