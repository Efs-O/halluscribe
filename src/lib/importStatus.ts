// HalluScribe - wording for the chat-import path diagnostic (GROK_IMPORT_PLAN
// § 12.4.4). Pure functions so the wording is testable without a Tauri host.

import { formatBytes } from "./format";
import type { ImportPathStatus } from "./types";

/** Severity drives the colour: only a set-but-empty or missing path is a
 *  problem the user needs to act on. An unset provider is simply unused. */
export type ImportStatusTone = "quiet" | "ok" | "warn";

export function importStatusTone(status: ImportPathStatus): ImportStatusTone {
  switch (status.state) {
    case "found":
      return "ok";
    case "missing_folder":
    case "no_export_found":
      return "warn";
    default:
      return "quiet";
  }
}

/** The one-line summary shown under the path field.
 *
 *  The `no_export_found` wording names the two things that actually cause it -
 *  nothing unpacked there, or nesting too deep - because "no export found"
 *  alone leaves the user with no next move. */
export function importStatusMessage(status: ImportPathStatus): string {
  switch (status.state) {
    case "unset":
      return "No path set — this provider is skipped.";
    case "missing_folder":
      return "That folder does not exist. It may have been moved or renamed, or its drive is not connected.";
    case "no_export_found":
      return "No export found here. Unpack the provider's export into this folder — if it is already there, it is nested too deep to reach.";
    case "found": {
      const size = formatBytes(status.total_bytes);
      return status.file_count === 1
        ? `Found 1 export file (${size}).`
        : `Found ${status.file_count} export files (${size} total).`;
    }
  }
}

/** Which file(s) answered, shown only when something was found. Seeing the
 *  path matters because a "found" on a stale copy in an old subfolder looks
 *  identical to a healthy one until you read where it came from. */
export function importStatusDetail(status: ImportPathStatus): string {
  if (status.state !== "found") return "";
  return status.files.join("\n");
}

/** Headline for the whole CHAT IMPORTS section: the problems, if any. Returns
 *  an empty string when nothing needs attention, so the UI can skip the row. */
export function importStatusSummary(statuses: ImportPathStatus[]): string {
  const problems = statuses.filter(
    (status) => status.state === "no_export_found" || status.state === "missing_folder",
  );
  if (problems.length === 0) return "";
  const names = problems.map((status) => status.label).join(", ");
  return problems.length === 1
    ? `${names} has a path set but nothing to import.`
    : `${names} have paths set but nothing to import.`;
}
