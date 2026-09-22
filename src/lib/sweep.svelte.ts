// HalluScribe - durable sweep state and Tauri event orchestration.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { SweepProgress } from "./types";

export class SweepController {
  running = $state(false);
  progress = $state<SweepProgress | null>(null);
  toast = $state<{ msg: string; ok: boolean } | null>(null);

  /**
   * `onDone` runs after every `sweep-done`. A business import writes its
   * last-import / last-error into settings.json when it finishes, so the owner
   * reloads settings here — otherwise the open Settings form keeps (and on its
   * next save writes back) the stale values.
   */
  constructor(private readonly onDone?: () => void) {}

  async registerListeners(): Promise<UnlistenFn[]> {
    return Promise.all([
      listen<SweepProgress>("sweep-progress", (event) => {
        this.running = true;
        this.progress = event.payload;
        if (!this.toast) {
          this.toast = { msg: "Sweep running...", ok: true };
        }
      }),
      listen<string>("sweep-done", (event) => {
        this.progress = null;
        this.toast = { msg: event.payload, ok: true };
        this.running = false;
        this.onDone?.();
      }),
    ]);
  }

  async runNow(): Promise<void> {
    await this.start("trigger_sweep");
  }

  /**
   * Start a business import (the same sweep behind its consent gate). State
   * lives here rather than in the settings section, so leaving and reopening
   * Settings mid-import still shows it running. Resolves to the start error,
   * or null once the import is running.
   */
  async runBusinessImport(): Promise<string | null> {
    return this.start("trigger_business_import");
  }

  private async start(command: string): Promise<string | null> {
    if (this.running) return "A sweep is already running.";
    this.running = true;
    this.progress = null;
    this.toast = null;
    try {
      await invoke(command);
      this.toast = { msg: "Sweep running...", ok: true };
      return null;
    } catch (error) {
      this.toast = { msg: String(error), ok: false };
      this.running = false;
      return String(error);
    }
  }

  async cancel(): Promise<void> {
    await invoke("cancel_sweep");
    this.toast = { msg: "Stopping - waiting for current session to finish...", ok: true };
  }

  dismissToast(): void { this.toast = null; }

  dispose(): void {}
}
