// HalluScribe - tests for when the settings form adopts parent settings.

import { describe, expect, it } from "vitest";
import { canAdoptIncomingSettings } from "./settingsSync";

describe("canAdoptIncomingSettings", () => {
  const adopted = { cc: "30", enabled: true };

  it("adopts on the first load", () => {
    expect(canAdoptIncomingSettings(null, null, adopted)).toBe(true);
  });

  it("adopts when the form has no unsaved edits", () => {
    const incoming = { cc: "30", enabled: false };
    expect(canAdoptIncomingSettings({ ...adopted }, adopted, incoming)).toBe(true);
  });

  it("keeps unsaved edits when the parent sends a different copy", () => {
    const local = { cc: "357", enabled: true };
    const incoming = { cc: "30", enabled: false };
    expect(canAdoptIncomingSettings(local, adopted, incoming)).toBe(false);
  });

  it("adopts the parent's echo of a save", () => {
    const local = { cc: "357", enabled: true };
    expect(canAdoptIncomingSettings(local, adopted, { ...local })).toBe(true);
  });
});
