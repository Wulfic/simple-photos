/**
 * Split-screen Compare (#21) selection helpers.
 *
 * Compare shows exactly two photos side by side, so the gallery "Compare"
 * action is only offered when precisely two items are selected. Pure so it can
 * be unit-tested without a DOM.
 */
export const COMPARE_COUNT = 2;

/** True when the current selection can enter Compare (exactly two items). */
export function canCompare(count: number): boolean {
  return count === COMPARE_COUNT;
}

/**
 * Resolve a selection into the ordered pair Compare needs, or null when the
 * selection isn't exactly two items.
 */
export function compareTargets(ids: Iterable<string>): [string, string] | null {
  const arr = Array.from(ids);
  return arr.length === COMPARE_COUNT ? [arr[0], arr[1]] : null;
}
