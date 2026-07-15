// HalluScribe - durable briefing state and Tauri event orchestration.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { BriefingFilters, BriefingScope, TokenPayload } from "./types";

export class BriefingController {
  answer = $state("");
  header = $state("");
  streaming = $state(false);
  error = $state<string | null>(null);
  warning = $state<string | null>(null);
  scope = $state<BriefingScope>({ kind: "archive-wide" });

  async registerListeners(): Promise<UnlistenFn[]> {
    return Promise.all([
      listen<string>("briefing-header", (event) => {
        this.header = event.payload;
      }),
      listen<string | null>("briefing-warning", (event) => {
        this.warning = event.payload;
      }),
      listen<TokenPayload>("briefing-token", (event) => {
        this.answer += event.payload.text;
      }),
      listen("briefing-done", () => {
        this.streaming = false;
      }),
    ]);
  }

  scopeLabel(): string {
    if (this.scope.kind === "selected-session-ids") return this.scope.label;
    if (this.scope.kind === "filter-based") return "Scope: filtered briefing";
    return "Scope: all archive";
  }

  selectedScopeActive(): boolean {
    return this.scope.kind === "selected-session-ids";
  }

  async cancel(): Promise<void> {
    await invoke("cancel_briefing");
  }

  clearOutput(): void {
    this.answer = "";
    this.header = "";
    this.error = null;
    this.warning = null;
  }

  resetScope(nextScope: BriefingScope): void {
    void this.cancel();
    this.scope = nextScope;
    this.streaming = false;
    this.clearOutput();
  }

  async run(filters: BriefingFilters): Promise<void> {
    const nextScope: BriefingScope = this.scope.kind === "selected-session-ids"
      ? this.scope
      : this.hasActiveFilters(filters)
        ? { kind: "filter-based", filters }
        : { kind: "archive-wide" };

    this.scope = nextScope;
    this.clearOutput();
    this.streaming = true;
    try {
      await invoke("run_briefing", {
        dateFrom: filters.dateFrom || null,
        dateTo: filters.dateTo || null,
        fillMin: filters.fillMin ? parseFloat(filters.fillMin) : null,
        fillMax: filters.fillMax ? parseFloat(filters.fillMax) : null,
        keyword: filters.keyword || null,
        sessionIds: nextScope.kind === "selected-session-ids" ? nextScope.sessionIds : null,
      });
    } catch (error) {
      this.error = String(error);
      this.streaming = false;
    }
  }

  private hasActiveFilters(filters: BriefingFilters): boolean {
    return Boolean(
      filters.dateFrom
      || filters.dateTo
      || filters.fillMin
      || filters.fillMax
      || filters.keyword.trim()
    );
  }
}
