// HalluScribe - decides when the settings form may adopt a fresh copy of the
// settings from its parent without clobbering the user's unsaved edits.

/**
 * `true` when the form can replace its local settings with `incoming`: on the
 * first load, when the local copy has no unsaved edits (it still matches what
 * was last adopted), or when `incoming` already equals the local copy (the
 * parent is echoing back a save).
 */
export function canAdoptIncomingSettings<T>(
  local: T | null,
  lastAdopted: T | null,
  incoming: T,
): boolean {
  if (local === null || lastAdopted === null) return true;
  const localJson = JSON.stringify(local);
  return localJson === JSON.stringify(lastAdopted) || localJson === JSON.stringify(incoming);
}
