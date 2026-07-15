// HalluScribe - shared callback types for settings section components.

export type SettingsNotify = (
  message: string,
  tone?: "ok" | "warn",
  duration?: number,
) => void;
