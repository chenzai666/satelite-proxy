import { useEffect, useMemo, useRef, useState } from "react";
import { getProxyStatus, listMihomoProxyGroups, onProxySnapshot } from "../api";
import { mihomoNodeSelections } from "../mihomoNodeSelection";
import type { MihomoProxyGroup, ProxyNode, ProxyStatus } from "../types";
import { useVisibleInterval } from "./useVisibleInterval";

/** Read-only policy selection: never mutate the legacy current_node_id or
 * bulk-operation selectedIds. Poll visible pages for url-test/fallback moves. */
export function useMihomoNodeSelection(enabled: boolean, nodes: ProxyNode[]) {
  const [groups, setGroups] = useState<MihomoProxyGroup[]>([]);
  const refreshRef = useRef<() => Promise<void>>(async () => {});
  useEffect(() => {
    setGroups([]);
    if (!enabled) return;
    let disposed = false;
    let pending = false;
    let epoch = 0;
    let statusKey = "";
    const applyStatus = (status: ProxyStatus) => {
      const key = `${status.running}:${status.core_type}:${status.outbound_mode}:${status.runtime_source}`;
      if (key !== statusKey) {
        statusKey = key;
        ++epoch;
        setGroups([]);
      }
      return status.running && status.core_type === "mihomo"
        && status.outbound_mode?.toLowerCase() !== "direct";
    };
    const refresh = async () => {
      if (disposed || pending) return;
      pending = true;
      let request = epoch;
      try {
        const status = await getProxyStatus();
        if (disposed) return;
        if (!applyStatus(status)) return;
        request = epoch;
        const next = await listMihomoProxyGroups();
        if (!disposed && request === epoch) setGroups(next);
      } catch {
        // Unknown/transitioning is not the old independent node selection.
        if (!disposed && request === epoch) setGroups([]);
      } finally {
        pending = false;
        if (!disposed && request !== epoch) void refresh();
      }
    };
    refreshRef.current = refresh;
    const unsubscribe = onProxySnapshot(status => {
      if (disposed) return;
      const previous = epoch;
      applyStatus(status);
      if (previous !== epoch) void refresh();
    });
    void refresh();
    return () => {
      disposed = true;
      unsubscribe();
      refreshRef.current = async () => {};
    };
  }, [enabled]);
  useVisibleInterval(() => refreshRef.current(), enabled ? 2000 : null);
  return useMemo(() => mihomoNodeSelections(groups, nodes), [groups, nodes]);
}
