import type { ConnectionView, LiveConnectionBatch } from "./types";

/** Hard cap on live rows held in the UI. Connections append at the tail of
 * the snapshot order, so the cap trims from the head (oldest) end. */
export const MAX_LIVE_ROWS = 1000;

export function applyConnectionChanges(
  current: ConnectionView[],
  batch: LiveConnectionBatch,
): ConnectionView[] {
  if (batch.unchanged) return current;
  if (batch.full || batch.order_ids) {
    const removed = new Set(batch.removed_ids);
    const byId = new Map(
      current.filter((row) => !removed.has(row.id)).map((row) => [row.id, row]),
    );
    for (const row of batch.rows) byId.set(row.id, row);
    const order = batch.order_ids ?? [...byId.keys()];
    return order
      .flatMap((id) => {
        const row = byId.get(id);
        return row ? [row] : [];
      })
      .slice(-MAX_LIVE_ROWS);
  }
  // Membership unchanged since our last batch (backend omitted order_ids) —
  // merge the updated rows in place instead of rebuilding the whole array.
  const removed = new Set(batch.removed_ids);
  const byId = new Map(batch.rows.map((row) => [row.id, row]));
  return current
    .filter((row) => !removed.has(row.id))
    .map((row) => byId.get(row.id) ?? row);
}
