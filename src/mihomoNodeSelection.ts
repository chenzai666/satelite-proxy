import type { MihomoProxyGroup, ProxyNode } from "./types";

/** Resolve each group's live `now` chain, never display-name matching.
 * The node tag contract matches config::outbound_tag; duplicate tags are
 * deliberately ambiguous rather than marking the wrong subscription node. */
export function mihomoNodeSelections(
  groups: MihomoProxyGroup[],
  nodes: Pick<ProxyNode, "id">[],
): Map<string, string[]> {
  const byName = new Map(groups.map(group => [group.name, group]));
  const nodeIds = new Map<string, string | null>();
  for (const node of nodes) {
    const tag = `node-${node.id.slice(0, 16)}`;
    nodeIds.set(tag, nodeIds.has(tag) ? null : node.id);
  }
  const selected = new Map<string, string[]>();
  for (const group of groups) {
    let current: MihomoProxyGroup | undefined = group;
    const visited = new Set<string>();
    while (current && !visited.has(current.name)) {
      visited.add(current.name);
      // A stale or malformed `now` must not invent a selection.
      if (!current.now || !current.all.includes(current.now)) break;
      const child = byName.get(current.now);
      if (child) { current = child; continue; }
      const id = nodeIds.get(current.now);
      if (id) {
        const labels = selected.get(id) ?? [];
        labels.push(group.labels?.[group.name] ?? group.name);
        selected.set(id, labels);
      }
      break;
    }
  }
  return selected;
}
