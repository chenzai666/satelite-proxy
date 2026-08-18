import type { ConnectionView, LiveConnectionBatch } from "./types";

export function applyConnectionChanges(
  current: ConnectionView[],
  batch: LiveConnectionBatch,
): ConnectionView[] {
  if (batch.unchanged) return current;
  if (batch.full) return batch.rows;
  const removed = new Set(batch.removed_ids);
  const byId = new Map(
    current.filter((row) => !removed.has(row.id)).map((row) => [row.id, row]),
  );
  for (const row of batch.rows) byId.set(row.id, row);
  return batch.order_ids.flatMap((id) => {
    const row = byId.get(id);
    return row ? [row] : [];
  });
}
