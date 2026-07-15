// HalluScribe - pure validation and normalization for scheduled sweep times.

const SCHEDULE_TIME_PATTERN = /^([01]\d|2[0-3]):([0-5]\d)$/;

export function normalizeScheduleTime(value: string): string | null {
  const trimmed = value.trim();
  if (SCHEDULE_TIME_PATTERN.test(trimmed)) return trimmed;
  const compact = /^(\d{1,2}):(\d{1,2})$/.exec(trimmed);
  if (!compact) return null;
  const hour = Number(compact[1]);
  const minute = Number(compact[2]);
  if (!Number.isInteger(hour) || !Number.isInteger(minute)) return null;
  if (hour < 0 || hour > 23 || minute < 0 || minute > 59) return null;
  return `${String(hour).padStart(2, "0")}:${String(minute).padStart(2, "0")}`;
}
