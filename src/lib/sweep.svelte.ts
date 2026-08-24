// HalluScribe - durable sweep state and Tauri event orchestration.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { SweepProgress } from "./types";

export class SweepController {
  running = $state(false);
  progress = $state<SweepProgress | null>(null);
  toast = $state<{ msg: string; ok: boolean } | null>(null);

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
      }),
    ]);
  }

  async runNow(): Promise<void> {
    if (this.running) return;
    this.running = true;
    this.progress = null;
    this.toast = null;
    try {
      await invoke("trigger_sweep");
      this.toast = { msg: "Sweep running...", ok: true };
    } catch (error) {
      this.toast = { msg: String(error), ok: false };
      this.running = false;
    }
  }

  async cancel(): Promise<void> {
    await invoke("cancel_sweep");
    this.toast = { msg: "Stopping - waiting for current session to finish...", ok: true };
  }

  dismissToast(): void { this.toast = null; }

  dispose(): void {}
}
