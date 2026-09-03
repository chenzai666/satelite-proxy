import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  deleteNodes,
  generateSingboxConfig,
  getNodeShareUri,
  getProxyStatus,
  getSettings,
  listAllNodes,
  listCustomConfigNodes,
  listNodeIds,
  pingNodesLatency,
  setCurrentNode,
  testCustomNodesLatency,
  testNodesLatency,
} from "../api";
import { EditLocalNodesModal } from "../components/EditLocalNodesModal";
import { GlassButton } from "../components/GlassButton";
import { NodeContextMenu, type NodeContextMenuState } from "../components/NodeContextMenu";
import { NodeShareModal } from "../components/NodeShareModal";
import { ErrorModal } from "../components/ErrorModal";
import { useI18n } from "../i18n";
import { groupNodes, type GroupBy } from "../nodeGroups";
import { GlassSeg } from "../components/GlassSeg";
import { waitForCoreRestart } from "../coreBusy";
import { useVirtualRange } from "../hooks/useVirtualRange";
import { filterCustomNodes, applyCustomLatency, type CustomLatencyMap } from "../customNodes";
import { copyNodeShareText } from "../nodeShare";
import { createLatencyResultBuffer } from "../latencyStream";
import type { AutoSelectMode, ProxyNode, SortMode, ViewMode } from "../types";

const VIRTUALIZE_AFTER = 200;
const LIST_ROW_HEIGHT = 49;
const GRID_ROW_HEIGHT = 94;
const NODE_GROUP_H = 30;
const NODE_LIST_COLS = "40px minmax(0,1.44fr) 90px minmax(0,1fr) 70px 90px";
const GRID_GAP = 10;

/** Flat render items for the grouped list (headers share the row height so
 *  the fixed-size virtualizer math stays exact). */
type ListItem =
  | {
      type: "group";
      key: string;
      label: string;
      flag?: string;
      count: number;
      h: number;
    }
  | { type: "node"; n: ProxyNode; h: number };
/** Grid items are row-granular: one item is one visual row of cards. */
type GridItem =
  | {
      type: "group";
      key: string;
      label: string;
      flag?: string;
      count: number;
      h: number;
    }
  | { type: "row"; nodes: ProxyNode[]; h: number };

function gridColumns() {
  if (window.innerWidth <= 720) return 2;
  if (window.innerWidth <= 900) return 3;
  return 4;
}

/** Render latency cell: spinner / ms / timeout / needs-core / dash */
function LatencyDisplay({
  ms,
  latencyAt,
  testing,
  unsupported,
  unsupportedLabel,
}: {
  ms?: number | null;
  latencyAt?: number | null;
  testing: boolean;
  unsupported?: boolean;
  /** Overrides the default "start core" note — e.g. after a ping test the
      QUIC-only note applies instead (the core isn't involved at all). */
  unsupportedLabel?: string;
}) {
  const { t } = useI18n();
  if (testing) {
    return <span className="lat-spinner" aria-label="测试中" />;
  }
  if (unsupported) {
    const label = unsupportedLabel ?? t("nodes.latencyNeedsCore");
    return <span className="lat lat-none" title={label}>{label}</span>;
  }
  if (ms != null && ms >= 0) {
    return (
      <span className={`lat ${latencyClass(ms)}`}>{ms}ms</span>
    );
  }
  // tested but no value → timeout
  if (latencyAt != null) {
    return <span className="lat lat-timeout">timeout</span>;
  }
  return <span className="lat lat-none">—</span>;
}

function latencyClass(ms?: number | null) {
  if (ms == null || ms < 0) return "lat-none";
  if (ms < 200) return "lat-good";
  if (ms < 300) return "lat-ok";
  return "lat-slow";
}

export function NodesPage() {
  const { t, locale } = useI18n();
  const [nodes, setNodes] = useState<ProxyNode[]>([]);
  const [currentId, setCurrentId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(true);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [autoSelect, setAutoSelect] = useState<AutoSelectMode>("off");
  // Manual click in kernel-auto mode: urltest → selector rebuild restarts the core.
  const [switching, setSwitching] = useState(false);
  const [viewMode, setViewMode] = useState<ViewMode>(() => {
    return (localStorage.getItem("nodes.viewMode") as ViewMode) || "list";
  });
  const [sortMode, setSortMode] = useState<SortMode>(() => {
    return (localStorage.getItem("nodes.sortMode") as SortMode) || "default";
  });
  // Click-test mode: node clicks probe latency instead of selecting.
  const [clickTest, setClickTest] = useState<boolean>(
    () => localStorage.getItem("nodes.clickTest") === "1",
  );

  const [customRuntime, setCustomRuntime] = useState(false);
  // Session-only latency results for custom-mode nodes (not persisted backend-side).
  const [customLatency, setCustomLatency] = useState<CustomLatencyMap>(new Map());
  const [testing, setTesting] = useState(false);
  const [testingIds, setTestingIds] = useState<Set<string>>(new Set());
  // Which probe the current/last run used — "real" rides the kernel's proxy
  // path, "ping" is direct TCP; drives button labels and the unsupported note.
  const [testKind, setTestKind] = useState<"real" | "ping">("real");
  // Node ids whose last test used method "unsupported" (UDP-only protocol,
  // core not running) — shown as "start core to test" instead of "timeout".
  const [unsupportedIds, setUnsupportedIds] = useState<Set<string>>(new Set());
  // Protocols delegated to the companion Xray sidecar (from settings) —
  // surfaced as a small badge so the egress path is visible per node.
  const [delegatedProtocols, setDelegatedProtocols] = useState<Set<string>>(
    new Set(),
  );
  // Keep the chunk size in sync with the CSS grid breakpoints so virtualized
  // rows remain aligned after a window resize.
  const [gridCols, setGridCols] = useState(gridColumns);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [batchBusy, setBatchBusy] = useState(false);
  const [contextMenu, setContextMenu] = useState<NodeContextMenuState | null>(null);
  const [shareNode, setShareNode] = useState<ProxyNode | null>(null);
  const [editNode, setEditNode] = useState<ProxyNode | null>(null);
  const [shareNotice, setShareNotice] = useState<string | null>(null);

  useEffect(() => {
    const update = () => setGridCols(gridColumns());
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, []);
  // Batch-test streaming: the rAF buffer between channel messages and state
  // (see latencyStream.ts); stopped on unmount so no flush lands post-dismount.
  const latencyBufferRef = useRef<ReturnType<
    typeof createLatencyResultBuffer
  > | null>(null);
  useEffect(
    () => () => latencyBufferRef.current?.stop(),
    [],
  );

  // Grouping: default (flat) / subscription / protocol / country, persisted
  // like viewMode. v2 key: the first iteration persisted "sub" as its
  // default — the feature is unreleased, so bump the key to let every
  // profile start on the new "default = flat" preference.
  const [groupBy, setGroupBy] = useState<GroupBy>(
    () =>
      (localStorage.getItem("nodes.groupBy.v2") as GroupBy | null) || "default",
  );
  useEffect(() => {
    localStorage.setItem("nodes.groupBy.v2", groupBy);
  }, [groupBy]);

  const reload = useCallback(async () => {
    setError(null);
    try {
      const settings = await getSettings();
      const custom = (settings.runtime_source ?? "generated").startsWith("singbox:");
      setCustomRuntime(custom);
      setCurrentId(settings.current_node_id ?? null);
      setAutoSelect((settings.auto_select as AutoSelectMode) ?? "off");
      setDelegatedProtocols(
        settings.multi_core_enabled
          ? new Set(
              (settings.protocol_cores ?? [])
                .filter((e) => e.core === "xray")
                .map((e) => e.protocol),
            )
          : new Set(),
      );
      // Always load the full node set — grouping needs to see everything to
      // classify correctly, and pagination made "load more" ambiguous once
      // grouped (unclear which group new items would land in).
      const all = custom
        ? applyCustomLatency(await listCustomConfigNodes(), customLatency)
        : await listAllNodes();
      const filtered = filterCustomNodes(all, query, sortMode, 0, Number.MAX_SAFE_INTEGER);
      setNodes(filtered.nodes);
      setTotal(filtered.total);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setLoading(false);
    }
  }, [query, sortMode, customLatency]);

  useEffect(() => {
    setLoading(true);
    setSelectedIds(new Set());
    const timer = window.setTimeout(() => void reload(), 150);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, sortMode]);

  useEffect(() => {
    localStorage.setItem("nodes.viewMode", viewMode);
  }, [viewMode]);

  useEffect(() => {
    localStorage.setItem("nodes.sortMode", sortMode);
  }, [sortMode]);

  useEffect(() => {
    localStorage.setItem("nodes.clickTest", clickTest ? "1" : "0");
  }, [clickTest]);

  const displayed = nodes;

  // Flat render items: group headers interleave with nodes at the same fixed
  // heights the virtualizer assumes (headers in the grid span the full row,
  // padded with filler cells to keep the per-cell math exact).
  const groups = useMemo(
    () =>
      groupNodes(displayed, groupBy, locale, {
        other: t("nodes.groupOther"),
        noSub: t("nodes.groupNoSub"),
      }),
    [displayed, groupBy, locale, t],
  );

  // Collapse state is kept per grouping dimension. Changing from protocol to
  // country must not reuse keys from the previous dimension, and reopening a
  // page should preserve the user's browsing choice.
  function collapsedStorageKey(by: GroupBy) {
    return `nodes.collapsedGroups.${by}`;
  }
  function loadCollapsed(by: GroupBy): Set<string> {
    if (by === "default") return new Set();
    try {
      const raw = localStorage.getItem(collapsedStorageKey(by));
      return raw ? new Set(JSON.parse(raw) as string[]) : new Set();
    } catch {
      return new Set();
    }
  }
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(() =>
    loadCollapsed(groupBy),
  );
  const previousGroupBy = useRef<GroupBy>(groupBy);
  useEffect(() => {
    if (previousGroupBy.current === groupBy) return;
    previousGroupBy.current = groupBy;
    setCollapsedGroups(loadCollapsed(groupBy));
  }, [groupBy]);
  useEffect(() => {
    if (groupBy === "default") return;
    localStorage.setItem(
      collapsedStorageKey(groupBy),
      JSON.stringify([...collapsedGroups]),
    );
  }, [groupBy, collapsedGroups]);
  function toggleGroup(key: string) {
    setCollapsedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }
  function collapseAll() {
    setCollapsedGroups(new Set(groups.map((group) => group.key)));
  }
  function expandAll() {
    setCollapsedGroups(new Set());
  }

  const listItems = useMemo(() => {
    const out: ListItem[] = [];
    for (const g of groups) {
      const grouped = groupBy !== "default";
      if (grouped) {
        out.push({
          type: "group",
          key: g.key,
          label: g.label,
          flag: g.flag,
          count: g.nodes.length,
          h: NODE_GROUP_H,
        });
      }
      if (!grouped || !collapsedGroups.has(g.key)) {
        for (const n of g.nodes) out.push({ type: "node", n, h: LIST_ROW_HEIGHT });
      }
    }
    return out;
  }, [groups, groupBy, collapsedGroups]);

  const gridItems = useMemo(() => {
    const out: GridItem[] = [];
    const pushRows = (list: ProxyNode[]) => {
      for (let i = 0; i < list.length; i += gridCols) {
        out.push({
          type: "row",
          nodes: list.slice(i, i + gridCols),
          h: GRID_ROW_HEIGHT,
        });
      }
    };
    for (const g of groups) {
      const grouped = groupBy !== "default";
      if (grouped) {
        out.push({
          type: "group",
          key: g.key,
          label: g.label,
          flag: g.flag,
          count: g.nodes.length,
          h: NODE_GROUP_H + GRID_GAP,
        });
      }
      if (!grouped || !collapsedGroups.has(g.key)) {
        pushRows(g.nodes);
      }
    }
    return out;
  }, [groups, groupBy, gridCols, collapsedGroups]);

  const virtualized = displayed.length > VIRTUALIZE_AFTER;
  // Pixel-space windows keep the 30px group headers and 49px node rows
  // exact. The hook's itemSize=1 turns its range into a pixel interval; the
  // prefix offsets below map that interval back to item indexes.
  function offsetsOf(items: { h: number }[]) {
    const offsets = new Array<number>(items.length + 1);
    offsets[0] = 0;
    for (let i = 0; i < items.length; i++) offsets[i + 1] = offsets[i] + items[i].h;
    return offsets;
  }
  function visibleWindow<T extends { h: number }>(
    items: T[],
    offsets: number[],
    startPx: number,
    endPx: number,
  ) {
    let low = 0;
    let high = items.length;
    while (low < high) {
      const mid = (low + high) >> 1;
      if (offsets[mid + 1] <= startPx) low = mid + 1;
      else high = mid;
    }
    const first = low;
    let last = first;
    while (last < items.length && offsets[last] < endPx) last++;
    const total = offsets[items.length] ?? 0;
    const bottom = offsets[last] ?? total;
    return {
      first,
      last,
      top: offsets[first] ?? 0,
      bottom,
      bottomPad: Math.max(0, total - bottom),
    };
  }
  const listOffsets = useMemo(() => offsetsOf(listItems), [listItems]);
  const gridOffsets = useMemo(() => offsetsOf(gridItems), [gridItems]);
  const listPixels = useVirtualRange({
    itemCount: Math.max(1, listOffsets[listOffsets.length - 1] ?? 0),
    itemSize: 1,
    enabled: virtualized,
    overscanRows: 400,
  });
  const gridPixels = useVirtualRange({
    itemCount: Math.max(1, gridOffsets[gridOffsets.length - 1] ?? 0),
    itemSize: 1,
    enabled: virtualized,
    overscanRows: 400,
  });
  const listWindow = useMemo(
    () =>
      visibleWindow(
        listItems,
        listOffsets,
        Math.max(0, listPixels.start),
        Math.min(listPixels.end, listOffsets[listOffsets.length - 1] ?? 0),
      ),
    [listItems, listOffsets, listPixels],
  );
  const gridWindow = useMemo(
    () =>
      visibleWindow(
        gridItems,
        gridOffsets,
        Math.max(0, gridPixels.start),
        Math.min(gridPixels.end, gridOffsets[gridOffsets.length - 1] ?? 0),
      ),
    [gridItems, gridOffsets, gridPixels],
  );

  async function onSelect(id: string) {
    if (busyId || switching || batchBusy) return;
    setBusyId(id);
    setError(null);
    try {
      const leavingKernel = autoSelect === "kernel";
      const before = await getSettings();
      const selected = await setCurrentNode(id);
      const coreSwitched = selected.core_type !== before.core_type;
      setCurrentId(id);
      setAutoSelect("off");
      // Running: Clash API hot-switch is immediate. Leaving kernel auto or
      // selecting an Xray-incompatible node rebuilds/restarts the core; the
      // latter is automatically handed to bundled sing-box by the backend.
      // Stopped: write the config for the selected (possibly new) core.
      const status = await getProxyStatus().catch(() => null);
      if (!status?.running) {
        await generateSingboxConfig();
      } else if (leavingKernel || coreSwitched) {
        // Main group rebuilds urltest → selector: hold the busy feedback
        // or wait for the automatic Xray → sing-box handoff to finish.
        setSwitching(true);
        await waitForCoreRestart();
      }
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setSwitching(false);
      setBusyId(null);
    }
  }

  async function onTestNodes(ids: string[], kind: "real" | "ping" = "real") {
    if (testing || ids.length === 0) return;
    setTesting(true);
    setTestKind(kind);
    setError(null);
    // Custom mode probes the extracted (unsaved) nodes — ids come from the
    // loaded list because they are not in the node store.
    const idSet = new Set(ids);
    setTestingIds(idSet);

    // clear prior latency so only spinner shows while testing
    setNodes((prev) =>
      prev.map((n) =>
        idSet.has(n.id)
          ? { ...n, latency_ms: undefined, latency_at: undefined }
          : n,
      ),
    );

    // Per-node streaming: the backend pushes each result over an IPC channel
    // the moment its probe completes; the buffer applies them per animation
    // frame (see latencyStream.ts).
    const buffer = createLatencyResultBuffer((batch) => {
      setUnsupportedIds((prev) => {
        const next = new Set(prev);
        for (const r of batch.values())
          if (r.method === "unsupported") next.add(r.id);
        return next;
      });
      if (customRuntime) {
        // Session-only — remember results across filter / sort / page reloads.
        setCustomLatency((prev) => {
          const next = new Map(prev);
          for (const [id, r] of batch) {
            next.set(id, { ms: r.latency_ms ?? null, at: r.tested_at });
          }
          return next;
        });
      }
      // Retire the finished spinners as their results land.
      setTestingIds((prev) => {
        const next = new Set(prev);
        for (const id of batch.keys()) next.delete(id);
        return next;
      });
      setNodes((prev) =>
        prev.map((n) => {
          const r = batch.get(n.id);
          if (!r) return n;
          return {
            ...n,
            // null = failed → show timeout; number = success
            latency_ms: r.latency_ms ?? null,
            latency_at: r.tested_at,
          };
        }),
      );
    });
    latencyBufferRef.current = buffer;

    try {
      // Custom mode can't map into the running config, so both probes are
      // the same direct-TCP path there.
      const batch = customRuntime
        ? await testCustomNodesLatency(3000, buffer.push)
        : kind === "ping"
          ? await pingNodesLatency(ids, 3000, buffer.push)
          : await testNodesLatency(ids, 3000, buffer.push);
      buffer.flushNow();
      setUnsupportedIds(
        new Set(batch.results.filter((r) => r.method === "unsupported").map((r) => r.id)),
      );
    } catch (e) {
      buffer.flushNow();
      setError(typeof e === "string" ? e : String(e));
      if (!customRuntime) await reload();
    } finally {
      setTesting(false);
      setTestingIds(new Set());
      // Custom results are session-only — keep the merged values instead of
      // re-reading the latency-less extracted list.
      if (!customRuntime) await reload();
    }
  }

  async function onTestLatency(kind: "real" | "ping" = "real") {
    const ids = customRuntime
      ? nodes.map((n) => n.id)
      : await listNodeIds(query, sortMode);
    await onTestNodes(ids, kind);
  }

  function toggleSelected(id: string) {
    if (customRuntime || batchBusy) return;
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  const selectAllMatching = useCallback(async () => {
    if (customRuntime || batchBusy) return;
    try {
      setSelectedIds(new Set(await listNodeIds(query)));
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }, [batchBusy, customRuntime, query]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== "a") return;
      const target = event.target as HTMLElement | null;
      if (target?.closest("input, textarea, [contenteditable='true']")) return;
      event.preventDefault();
      void selectAllMatching();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [selectAllMatching]);

  async function deleteSelected(ids: string[]) {
    if (batchBusy || ids.length === 0) return;
    const confirmed = window.confirm(
      ids.length === 1
        ? t("nodes.deleteConfirm", { name: nodes.find((node) => node.id === ids[0])?.name ?? ids[0] })
        : t("nodes.deleteSelectedConfirm", { n: ids.length }),
    );
    if (!confirmed) return;
    setBatchBusy(true);
    setError(null);
    try {
      await deleteNodes(ids);
      setSelectedIds((current) => {
        const next = new Set(current);
        ids.forEach((id) => next.delete(id));
        return next;
      });
      await reload();
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBatchBusy(false);
    }
  }

  async function copyShareLink(node: ProxyNode) {
    try {
      await copyNodeShareText(await getNodeShareUri(node.id));
      setShareNotice(t("nodes.shareCopied"));
      window.setTimeout(() => setShareNotice(null), 2200);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }

  function openNodeEditor(node: ProxyNode) {
    if (!node.subscription_id) {
      setError("节点来源未知，无法编辑");
      return;
    }
    setEditNode(node);
  }
  // After a ping run, "unsupported" means QUIC-only (unpingable), not "core
  // stopped" — swap the cell note accordingly.
  const pingNote = testKind === "ping" ? t("nodes.pingUnsupported") : undefined;

  // Click-test mode: probe one node with the real-latency path (Clash delay
  // API through the core; TCP fallback when the core is stopped). The backend
  // persists the result, same as the batch run.
  async function onTestOne(id: string) {
    if (testing || testingIds.size > 0 || busyId || switching) return;
    setTestKind("real");
    setError(null);
    setTestingIds(new Set([id]));
    setNodes((prev) =>
      prev.map((n) =>
        n.id === id ? { ...n, latency_ms: undefined, latency_at: undefined } : n,
      ),
    );
    try {
      const batch = await testNodesLatency([id], 3000);
      const r = batch.results.find((x) => x.id === id);
      setUnsupportedIds((prev) => {
        const next = new Set(prev);
        if (r?.method === "unsupported") next.add(id);
        else next.delete(id);
        return next;
      });
      if (r) {
        setNodes((prev) =>
          prev.map((n) =>
            n.id === id
              ? { ...n, latency_ms: r.latency_ms ?? null, latency_at: r.tested_at }
              : n,
          ),
        );
      }
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setTestingIds((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }
  }

  /** Group header row (list): click to expand or collapse its node set. */
  function renderGroupRow(item: Extract<ListItem, { type: "group" }>) {
    const open = !collapsedGroups.has(item.key);
    return (
      <div
        key={item.key}
        className="node-list-group-row"
        style={{ height: NODE_GROUP_H }}
        onClick={() => toggleGroup(item.key)}
        title={t("nodes.groupToggleHint")}
      >
        <span className={`node-group-caret${open ? "" : " closed"}`} />
        <span className="node-group-label">
          {item.flag ? <span className="node-group-flag">{item.flag}</span> : null}
          {item.label}
        </span>
        <span className="node-group-count mono">{item.count}</span>
      </div>
    );
  }

  /** Group header band (grid): spans all columns and toggles its node set. */
  function renderGroupHead(item: Extract<GridItem, { type: "group" }>) {
    const open = !collapsedGroups.has(item.key);
    return (
      <div
        key={item.key}
        className="node-group-head"
        style={{ height: NODE_GROUP_H }}
        onClick={() => toggleGroup(item.key)}
        title={t("nodes.groupToggleHint")}
      >
        <span className={`node-group-caret${open ? "" : " closed"}`} />
        <span className="node-group-label">
          {item.flag ? <span className="node-group-flag">{item.flag}</span> : null}
          {item.label}
        </span>
        <span className="node-group-count mono">{item.count}</span>
      </div>
    );
  }

  function renderNodeRow(n: ProxyNode) {
    const active = n.id === currentId;
    const isTesting = testingIds.has(n.id);
    const selected = selectedIds.has(n.id);
    return (
      <div
        key={n.id}
        className={`node-list-row node-virtual-row ${active ? "row-active" : ""} ${selected ? "row-selected" : ""}`}
        style={{
          gridTemplateColumns: NODE_LIST_COLS,
          cursor: customRuntime ? "default" : "pointer",
        }}
        onClick={(event) => {
          if (customRuntime) return;
          if (event.ctrlKey || event.metaKey) {
            toggleSelected(n.id);
          } else if (clickTest) {
            void onTestOne(n.id);
          }
        }}
        title={!customRuntime && clickTest ? t("nodes.clickTestLatency") : undefined}
        onContextMenu={(event) => {
          if (customRuntime) return;
          event.preventDefault();
          setContextMenu({ node: n, x: event.clientX, y: event.clientY });
        }}
      >
        <span>
          <button
            type="button"
            className="node-select-dot"
            aria-label={`切换到 ${n.name}`}
            aria-pressed={active}
            disabled={customRuntime || busyId === n.id || batchBusy}
            onClick={(event) => {
              event.stopPropagation();
              void onSelect(n.id);
            }}
          >
            {active ? "●" : "○"}
          </button>
        </span>
        <span>
          <div className="node-list-name">{n.name}</div>
          {n.subscription_name ? (
            <div className="node-sub-label" title={n.subscription_name}>
              {n.subscription_name}
            </div>
          ) : null}
        </span>
        <span>
          <code>{n.protocol}</code>
          {delegatedProtocols.has(n.protocol) ? (
            <span className="pill sidecar-tag">Xray</span>
          ) : null}
        </span>
        <span>{n.server}</span>
        <span>{n.port}</span>
        <span className="node-list-latency">
          <button
            type="button"
            className="node-latency-action"
            title={t("nodes.testOneLatency")}
            aria-label={`${t("nodes.testOneLatency")}：${n.name}`}
            disabled={testing || customRuntime || batchBusy}
            onClick={(event) => {
              event.stopPropagation();
              void onTestNodes([n.id], "real");
            }}
          >
            <LatencyDisplay
              ms={n.latency_ms}
              latencyAt={n.latency_at}
              testing={isTesting}
              unsupported={unsupportedIds.has(n.id)}
              unsupportedLabel={pingNote}
            />
          </button>
        </span>
      </div>
    );
  }

  function renderNodeCard(n: ProxyNode) {
    const active = n.id === currentId;
    const isTesting = testingIds.has(n.id);
    const selected = selectedIds.has(n.id);
    return (
      <div
        key={n.id}
        className={`node-card ${active ? "active" : ""} ${selected ? "selected" : ""}`}
        onClick={(event) => {
          if (customRuntime) return;
          if (event.ctrlKey || event.metaKey) {
            toggleSelected(n.id);
          } else if (clickTest) {
            void onTestOne(n.id);
          }
        }}
        style={{ cursor: customRuntime ? "default" : clickTest ? "pointer" : undefined }}
        title={!customRuntime && clickTest ? t("nodes.clickTestLatency") : undefined}
        onContextMenu={(event) => {
          if (customRuntime) return;
          event.preventDefault();
          setContextMenu({ node: n, x: event.clientX, y: event.clientY });
        }}
      >
        <div className="node-card-top">
          <button
            type="button"
            className="node-select-dot"
            aria-label={`切换到 ${n.name}`}
            aria-pressed={active}
            disabled={customRuntime || busyId === n.id || batchBusy}
            onClick={(event) => {
              event.stopPropagation();
              void onSelect(n.id);
            }}
          >
            {active ? "●" : "○"}
          </button>
          <div className="node-card-meta">
            <code>{n.protocol}</code>
            {delegatedProtocols.has(n.protocol) ? (
              <span className="pill sidecar-tag">Xray</span>
            ) : null}
          </div>
        </div>
        <div className="node-card-name" title={n.name}>{n.name}</div>
        <div className="node-card-footer">
          <span className="node-sub-label" title={n.subscription_name ?? ""}>
            {n.subscription_name}
          </span>
          <button
            type="button"
            className="node-latency-action node-card-latency"
            title={t("nodes.testOneLatency")}
            aria-label={`${t("nodes.testOneLatency")}：${n.name}`}
            disabled={testing || customRuntime || batchBusy}
            onClick={(event) => {
              event.stopPropagation();
              void onTestNodes([n.id], "real");
            }}
          >
            <LatencyDisplay
              ms={n.latency_ms}
              latencyAt={n.latency_at}
              testing={isTesting}
              unsupported={unsupportedIds.has(n.id)}
              unsupportedLabel={pingNote}
            />
          </button>
        </div>
      </div>
    );
  }

  return (

    <div className="page nodes-page">
      {customRuntime && (
        <div className="banner" role="status">
          {t("nodes.customReadOnly")}
        </div>
      )}
      <header className="page-header">
        <div>
          <h1>{t("nodes.title")}</h1>
          <p className="page-desc">
            {t("nodes.desc")}
            {" · "}
            <span className="mono">
              {query.trim()
                ? t("nodes.countFiltered", {
                    shown: displayed.length,
                    total,
                  })
                : t("nodes.count", { n: total })}
            </span>
          </p>
        </div>
        <div className="header-actions nodes-toolbar">
          <input
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            className="search"
            placeholder={t("nodes.search")}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />

          <GlassSeg
            value={sortMode}
            ariaLabel="sort"
            onChange={(v) => setSortMode(v as SortMode)}
            options={[
              { value: "default", label: t("nodes.sortDefault") },
              { value: "name", label: t("nodes.sortName") },
              { value: "latency", label: t("nodes.sortLatency") },
            ]}
          />

          {/* Monochrome text glyphs (same family as ↻ / + elsewhere) — they
              follow the button color instead of rendering as color emoji. */}
          <GlassButton
            icon="◉"
            disabled={testing || displayed.length === 0}
            onClick={() => void onTestLatency("real")}
            title={t("nodes.testRealLatencyHint")}
          >
            {testing && testKind === "real" ? t("nodes.testing") : t("nodes.testRealLatency")}
          </GlassButton>
          {/* Hidden in custom mode — there both probes take the same
              direct-TCP path (extracted nodes have no kernel mapping). */}
          {!customRuntime && (
            <GlassButton
              icon="∿"
              disabled={testing || displayed.length === 0}
              onClick={() => void onTestLatency("ping")}
              title={t("nodes.pingTestHint")}
            >
              {testing && testKind === "ping" ? t("nodes.pinging") : t("nodes.pingTest")}
            </GlassButton>
          )}
          {/* 单点测试 toggle: state reads from the LED dot alone — gray
              while off, green while armed (same LED language as the logs
              page kernel tabs). Label stays constant in both states.
              Meaningless in custom mode (rows are not clickable there) —
              hidden with ping. */}
          {!customRuntime && (
            <GlassButton
              icon={
                <span
                  className={`seg-dot${clickTest ? " on" : ""}`}
                  aria-hidden
                />
              }
              onClick={() => setClickTest((v) => !v)}
              title={t("nodes.clickTestHint")}
            >
              {t("nodes.clickTest")}
            </GlassButton>
          )}

          <div className="nodes-view-segs">
            {!customRuntime && clickTest && (
              <span className="nodes-clicktest-active">
                {t("nodes.clickTestActive")}
              </span>
            )}
            <GlassSeg
              value={groupBy}
              ariaLabel={t("nodes.groupBy")}
              onChange={(v) => setGroupBy(v as GroupBy)}
              options={[
                { value: "default", label: t("nodes.groupDefault") },
                { value: "sub", label: t("nodes.groupSub") },
                { value: "proto", label: t("nodes.groupProto") },
                { value: "country", label: t("nodes.groupCountry") },
              ]}
            />
            <div className="node-group-fold" role="group" aria-label={t("nodes.groupBy")}>
              <span
                className={`node-group-fold-label minus${groupBy === "default" ? " disabled" : ""}`}
                onClick={groupBy === "default" ? undefined : collapseAll}
                title={t("nodes.collapseAll")}
              />
              <span
                className={`node-group-fold-label plus${groupBy === "default" ? " disabled" : ""}`}
                onClick={groupBy === "default" ? undefined : expandAll}
                title={t("nodes.expandAll")}
              />
            </div>
          </div>

          <GlassSeg
            value={viewMode}
            ariaLabel="视图"
            onChange={(v) => setViewMode(v as ViewMode)}
            options={[
              { value: "list", label: "列表" },
              { value: "grid", label: "网格" },
            ]}
          />
        </div>
      </header>

      {!customRuntime && selectedIds.size > 0 && (
        <div className="node-batch-toolbar" role="status">
          <span>{t("nodes.selectedCount", { n: selectedIds.size })}</span>
          <GlassButton
            disabled={testing || batchBusy}
            onClick={() => void onTestNodes([...selectedIds])}
          >
            {t("nodes.testSelected")}
          </GlassButton>
          <GlassButton
            variant="danger"
            disabled={batchBusy}
            onClick={() => void deleteSelected([...selectedIds])}
          >
            {t("nodes.deleteSelected")}
          </GlassButton>
          <GlassButton disabled={batchBusy} onClick={() => setSelectedIds(new Set())}>
            {t("nodes.clearSelection")}
          </GlassButton>
        </div>
      )}

      {shareNotice && <div className="banner" role="status">{shareNotice}</div>}

      {error && (
        <ErrorModal message={error} onClose={() => setError(null)} />
      )}

      {switching && (
        <div className="banner busy" role="status">
          <span className="lat-spinner" aria-hidden />
          {t("nodes.switchingManual")}
        </div>
      )}

      {loading ? (
        <div className="empty">{t("common.loading")}</div>
      ) : displayed.length === 0 ? (
        <div className="empty card muted">
          {nodes.length === 0
            ? customRuntime
              ? t("nodes.customEmpty")
              : t("nodes.empty")
            : "—"}
        </div>
      ) : viewMode === "list" ? (
        <div className={`card table-wrap${clickTest ? " spot-armed" : ""}`}>
          <div className="node-list">
            <div className="node-list-head" style={{ gridTemplateColumns: NODE_LIST_COLS }}>
              <span />
              <span>{t("nodes.sortName")}</span>
              <span>proto</span>
              <span>host</span>
              <span>port</span>
              <span>{t("nodes.sortLatency")}</span>
            </div>
            <div ref={listPixels.containerRef as React.RefObject<HTMLDivElement>}>
              {listWindow.top > 0 && (
                <div
                  className="node-virtual-spacer"
                  aria-hidden="true"
                  style={{ height: listWindow.top }}
                />
              )}
              {listItems
                .slice(listWindow.first, listWindow.last)
                .map((item) =>
                  item.type === "group" ? renderGroupRow(item) : renderNodeRow(item.n),
                )}
              {listWindow.bottomPad > 0 && (
                <div
                  className="node-virtual-spacer"
                  aria-hidden="true"
                  style={{ height: listWindow.bottomPad }}
                />
              )}
            </div>
          </div>
        </div>
      ) : (
        <div
          className={virtualized ? "node-grid-window" : undefined}
          ref={gridPixels.containerRef as React.RefObject<HTMLDivElement>}
        >
          {gridWindow.top > 0 && (
            <div style={{ height: gridWindow.top }} aria-hidden="true" />
          )}
          <div
            className={`node-grid ${virtualized ? "node-grid-virtual" : ""}${clickTest ? " spot-armed" : ""}`}
          >
            {gridItems
              .slice(gridWindow.first, gridWindow.last)
              .map((item) => {
                if (item.type === "group") return renderGroupHead(item);
                return item.nodes.map((node) => renderNodeCard(node));
              })}
          </div>
          {gridWindow.bottomPad > 0 && (
            <div style={{ height: gridWindow.bottomPad }} aria-hidden="true" />
          )}
        </div>
      )}
      <NodeContextMenu
        state={contextMenu}
        onClose={() => setContextMenu(null)}
        onEdit={openNodeEditor}
        onCopyLink={(node) => void copyShareLink(node)}
        onShowQr={setShareNode}
        onDelete={(node) => void deleteSelected([node.id])}
      />
      <NodeShareModal node={shareNode} onClose={() => setShareNode(null)} />
      <EditLocalNodesModal
        open={!!editNode}
        profileId={editNode?.subscription_id ?? null}
        profileName={editNode?.subscription_name ?? ""}
        initialNodeId={editNode?.id ?? null}
        onClose={() => setEditNode(null)}
        onNodesChanged={() => void reload()}
      />
    </div>
  );
}
