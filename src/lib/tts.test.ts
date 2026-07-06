// HalluScribe - tests for the speech-cleanup helper used by the Speak button.

import { describe, expect, it } from "vitest";
import { stripForSpeech } from "./tts.svelte.ts";

describe("stripForSpeech", () => {
  it("strips thinking blocks", () => {
    expect(stripForSpeech("before <thinking>secret plan</thinking> after")).toBe("before after");
    expect(stripForSpeech("<think>hmm</think>hello")).toBe("hello");
  });

  it("replaces fenced code blocks with a spoken cue", () => {
    expect(stripForSpeech("see ```js\nconst x = 1;\n``` done")).toBe("see Code block omitted. done");
  });

  it("drops inline code content", () => {
    expect(stripForSpeech("run `npm test` now")).toBe("run now");
  });

  it("keeps the label text of markdown links", () => {
    expect(stripForSpeech("read [the docs](https://example.com/docs) today")).toBe(
      "read the docs today",
    );
  });

  it("keeps the label text of markdown images", () => {
    expect(stripForSpeech("![a cat](https://example.com/cat.png)")).toBe("a cat");
  });

  it("strips HTML tags", () => {
    expect(stripForSpeech("<b>bold</b> and <i>italic</i>")).toBe("bold and italic");
  });

  it("strips emojis and pictographic symbols", () => {
    expect(stripForSpeech("great job! \u{1F389}\u{1F600}")).toBe("great job!");
  });

  it("strips remaining markdown syntax characters", () => {
    expect(stripForSpeech("# Title **bold** _italic_ >quote ~strike| pipe")).toBe(
      "Title bold italic quote strike pipe",
    );
  });

  it("collapses whitespace", () => {
    expect(stripForSpeech("a   b\n\n\nc")).toBe("a b c");
  });

  it("returns '' for empty input", () => {
    expect(stripForSpeech("")).toBe("");
  });

  it("returns '' for whitespace-only input", () => {
    expect(stripForSpeech("   \n\t  ")).toBe("");
  });
});
