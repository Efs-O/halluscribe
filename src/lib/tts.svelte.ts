// HalluScribe - text-to-speech playback: markdown/thinking-block cleanup for
// speech, and a single shared SpeakController that synthesizes (via the
// tts_speak Tauri command) and plays one reply at a time through Web Audio.

import { invoke } from "@tauri-apps/api/core";

/** Playback lifecycle for the currently-active (if any) spoken turn. */
export type SpeakPhase = "idle" | "synthesizing" | "playing";

/**
 * Strip a reply down to speakable text: drop thinking/reasoning blocks,
 * fenced/inline code, markdown links (keep label), HTML tags, emojis, and
 * remaining markdown syntax characters; collapse whitespace.
 */
export function stripForSpeech(text: string): string {
  return text
    // thinking / reasoning blocks
    .replace(/<think(?:ing)?[^>]*>[\s\S]*?<\/think(?:ing)?>/gi, "")
    // fenced code blocks — replace with a short spoken cue
    .replace(/```[\s\S]*?```/g, " Code block omitted. ")
    // inline code — skip content, too noisy to read
    .replace(/`[^`]+`/g, "")
    // markdown links/images — keep label text
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    // HTML/XML tags
    .replace(/<[^>]+>/g, "")
    // emojis and pictographic symbols
    .replace(/[\u{1F000}-\u{1FFFF}\u{2600}-\u{27FF}\u{FE00}-\u{FE0F}]/gu, "")
    // remaining markdown syntax characters
    .replace(/[*_#>~|`]/g, "")
    // collapse whitespace
    .replace(/\s{2,}/g, " ")
    .trim();
}

/**
 * One shared controller for the whole chat surface (briefing / profile /
 * archive / semantic chat all reuse the same instance). Tracks which turn id
 * is currently synthesizing or playing so only that turn's button lights up,
 * and guards every async step with a monotonic sequence number so a new
 * speak() call — or a cancel() — always wins over a stale in-flight one.
 *
 * Backed by Svelte 5 `$state` class fields (this file uses the `.svelte.ts`
 * module extension so the compiler processes runes outside a component) so
 * any component reading `activeId()` / `phase()` re-renders when playback
 * state changes, without needing a separate store wrapper.
 */
export class SpeakController {
  private activeIdValue: string | null = $state(null);
  private phaseValue: SpeakPhase = $state("idle");

  private ctx: AudioContext | null = null;
  private source: AudioBufferSourceNode | null = null;
  private seq = 0;

  activeId(): string | null {
    return this.activeIdValue;
  }

  phase(): SpeakPhase {
    return this.phaseValue;
  }

  /** Stop any in-flight synthesis or playback and return to idle. */
  cancel(): void {
    this.seq += 1;
    if (this.source) {
      try {
        this.source.stop();
      } catch {
        // already stopped
      }
      this.source = null;
    }
    this.activeIdValue = null;
    this.phaseValue = "idle";
  }

  /**
   * Speak `rawText` for turn `id`. Clicking the same turn while it is
   * synthesizing or playing cancels it (toggle-to-stop). Clicking a
   * different turn cancels whatever else is active first, so only one
   * reply ever plays at a time.
   */
  async speak(id: string, rawText: string): Promise<void> {
    const alreadyActiveForThisTurn = this.activeIdValue === id && this.phaseValue !== "idle";
    if (alreadyActiveForThisTurn) {
      this.cancel();
      return;
    }

    this.cancel();
    const seq = this.seq;
    this.activeIdValue = id;
    this.phaseValue = "synthesizing";

    const clean = stripForSpeech(rawText);
    if (!clean) {
      if (this.seq === seq) this.cancel();
      return;
    }

    try {
      if (!this.ctx || this.ctx.state === "closed") {
        this.ctx = new AudioContext();
      }
      await this.ctx.resume();

      const buf = await invoke<ArrayBuffer>("tts_speak", { text: clean });
      if (this.seq !== seq) return;

      const audioBuffer = await this.ctx.decodeAudioData(buf);
      if (this.seq !== seq) return;

      const source = this.ctx.createBufferSource();
      source.buffer = audioBuffer;
      source.connect(this.ctx.destination);
      this.source = source;
      this.phaseValue = "playing";
      source.onended = () => {
        if (this.seq === seq) {
          this.activeIdValue = null;
          this.phaseValue = "idle";
        }
        this.source = null;
      };
      source.start();
    } catch (error) {
      if (this.seq === seq) {
        this.activeIdValue = null;
        this.phaseValue = "idle";
      }
      throw error;
    }
  }
}
