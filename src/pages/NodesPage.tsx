import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  deleteNodes,
  generateSingboxConfig,
  getNodeShareUri,
  getProxyStatus,
  getSettings,
  listCustomConfigNodes,
  listNodeIds,
  listNodesPage,
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
const PAGE_SIZE = 200;

/** Flat render items for the grouped list (headers share the row height so
 *  the fixed-size virtualizer math stays exact). */
type ListItem =
  | { type: "group"; label: string; flag?: string; count: number }
  | { type: "node"; n: ProxyNode };
/** Grid variant: a full-width header occupies one cell slot and pads the rest
 *  of its row with fillers so subsequent cards stay aligned. */
type GridItem = ListItem | { type: "filler" };

function gridColumns() {
  if (window.innerWidth <= 720) return 2;
  if (window.innerWidth <= 960) return 3;
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
  const [loadingMore, setLoadingMore] = useState(false);
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
  const [columnCount, setColumnCount] = useState(gridColumns);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [batchBusy, setBatchBusy] = useState(false);
  const [contextMenu, setContextMenu] = useState<NodeContextMenuState | null>(null);
  const [shareNode, setShareNode] = useState<ProxyNode | null>(null);
  const [editNode, setEditNode] = useState<ProxyNode | null>(null);
  const [shareNotice, setShareNotice] = useState<string | null>(null);

  useEffect(() => {
    const update = () => setColumnCount(gridColumns());
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
  const reload = useCallback(async (append = false) => {
    setError(null);
    if (append) setLoadingMore(true);
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
      const offset = append ? nodes.length : 0;
      if (custom) {
        // Custom mode: read-only nodes extracted from the sing-box config,
        // overlaid with this session's latency results.
        const all = applyCustomLatency(await listCustomConfigNodes(), customLatency);
        const filtered = filterCustomNodes(all, query, sortMode, offset, PAGE_SIZE);
        setNodes((prev) => (append ? [...prev, ...filtered.nodes] : filtered.nodes));
        setTotal(filtered.total);
      } else {
        const page = await listNodesPage(query, sortMode, offset, PAGE_SIZE);
        setNodes((prev) => (append ? [...prev, ...page.nodes] : page.nodes));
        setTotal(page.total);
      }
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setLoading(false);
      setLoadingMore(false);
    }
  }, [nodes.length, query, sortMode, customLatency]);

  useEffect(() => {
    setLoading(true);
    setSelectedIds(new Set());
    const timer = window.setTimeout(() => void reload(false), 150);
    return () => window.clearTimeout(timer);
    // nodes.length changes as pages append and must not restart the first page.
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

  // Grouping: subscription / protocol / country (persisted like viewMode).
  const [groupBy, setGroupBy] = useState<GroupBy>(
    () => (localStorage.getItem("nodes.groupBy") as GroupBy) || "default",
  );
  useEffect(() => {
    localStorage.setItem("nodes.groupBy", groupBy);
  }, [groupBy]);

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

  const listItems = useMemo(() => {
    const out: ListItem[] = [];
    for (const g of groups) {
      if (groupBy !== "default") {
        out.push({
          type: "group",
          label: g.label,
          flag: g.flag,
          count: g.nodes.length,
        });
      }
      for (const n of g.nodes) out.push({ type: "node", n });
    }
    return out;
  }, [groups, groupBy]);

  const gridItems = useMemo(() => {
    const out: GridItem[] = [];
    for (const g of groups) {
      if (groupBy !== "default") {
        out.push({
          type: "group",
          label: g.label,
          flag: g.flag,
          count: g.nodes.length,
        });
        for (let i = 1; i < columnCount; i++) out.push({ type: "filler" });
      }
      for (const n of g.nodes) out.push({ type: "node", n });
    }
    return out;
  }, [groups, groupBy, columnCount]);

  const virtualized = displayed.length > VIRTUALIZE_AFTER;
  const listRange = useVirtualRange({
    itemCount: listItems.length,
    itemSize: LIST_ROW_HEIGHT,
    enabled: virtualized,
  });
  const gridRange = useVirtualRange({
    itemCount: gridItems.length,
    itemSize: GRID_ROW_HEIGHT,
    itemsPerRow: columnCount,
    enabled: virtualized,
  });

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
      if (!customRuntime) await reload(false);
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
      await reload(false);
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

  /** Group header row (list): full-width cell at ROW_H for the virtualizer. */
  function renderGroupRow(key: string, label: string, flag: string | undefined, count: number) {
    return (
      <tr key={key} className="node-group-row">
        <td colSpan={6}>
          <span className="node-group-label">
            {flag ? <span className="node-group-flag">{flag}</span> : null}
            {label}
          </span>
          <span className="node-group-count mono">{count}</span>
        </td>
      </tr>
    );
  }

  /** Group header band (grid): spans all columns at GRID_ROW_HEIGHT. */
  function renderGroupHead(key: string, label: string, flag: string | undefined, count: number) {
    return (
      <div key={key} className="node-group-head" style={{ height: GRID_ROW_HEIGHT }}>
        <span className="node-group-label">
          {flag ? <span className="node-group-flag">{flag}</span> : null}
          {label}
        </span>
        <span className="node-group-count mono">{count}</span>
      </div>
    );
  }

  function renderNodeRow(n: ProxyNode) {
    const active = n.id === currentId;
    const isTesting = testingIds.has(n.id);
    const selected = selectedIds.has(n.id);
    return (
      <tr
        key={n.id}
        className={`node-virtual-row ${active ? "row-active" : ""} ${selected ? "row-selected" : ""}`}
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
        <td>
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
        </td>
        <td>
          <div className="node-list-name">{n.name}</div>
          {n.subscription_name ? (
            <div className="node-sub-label" title={n.subscription_name}>
              {n.subscription_name}
            </div>
          ) : null}
        </td>
        <td>
          <code>{n.protocol}</code>
          {delegatedProtocols.has(n.protocol) ? (
            <span className="pill sidecar-tag">Xray</span>
          ) : null}
        </td>
        <td>{n.server}</td>
        <td>{n.port}</td>
        <td className="node-list-latency">
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
        </td>
      </tr>
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
          <table>
            <thead>
              <tr>
                <th style={{ width: 40 }}></th>
                <th>{t("nodes.sortName")}</th>
                <th>proto</th>
                <th>host</th>
                <th>port</th>
                <th style={{ width: 90 }}>{t("nodes.sortLatency")}</th>
              </tr>
            </thead>
            <tbody ref={listRange.containerRef as React.RefObject<HTMLTableSectionElement>}>
              {listRange.paddingTop > 0 && (
                <tr className="node-virtual-spacer" aria-hidden="true">
                  <td colSpan={6} style={{ height: listRange.paddingTop }} />
                </tr>
              )}
              {listItems
                .slice(listRange.start, listRange.end)
                .map((item, i) =>
                  item.type === "group" ? (
                    renderGroupRow(
                      `g-${listRange.start + i}`,
                      item.label,
                      item.flag,
                      item.count,
                    )
                  ) : (
                    renderNodeRow(item.n)
                  ),
                )}
              {listRange.paddingBottom > 0 && (
                <tr className="node-virtual-spacer" aria-hidden="true">
                  <td colSpan={6} style={{ height: listRange.paddingBottom }} />
                </tr>
              )}
            </tbody>
          </table>
        </div>
      ) : (
        <div
          className={virtualized ? "node-grid-window" : undefined}
          ref={gridRange.containerRef as React.RefObject<HTMLDivElement>}
        >
          {gridRange.paddingTop > 0 && (
            <div style={{ height: gridRange.paddingTop }} aria-hidden="true" />
          )}
          <div
            className={`node-grid ${virtualized ? "node-grid-virtual" : ""}${clickTest ? " spot-armed" : ""}`}
          >
            {gridItems
              .slice(gridRange.start, gridRange.end)
              .map((item, i) => {
                if (item.type === "group")
                  return renderGroupHead(
                    `g-${gridRange.start + i}`,
                    item.label,
                    item.flag,
                    item.count,
                  );
                if (item.type === "filler")
                  return <div key={`f-${gridRange.start + i}`} aria-hidden />;
                return renderNodeCard(item.n);
              })}
          </div>
          {gridRange.paddingBottom > 0 && (
            <div style={{ height: gridRange.paddingBottom }} aria-hidden="true" />
          )}
        </div>
      )}
      {!loading && nodes.length < total && (
        <div style={{ display: "flex", justifyContent: "center", padding: 12 }}>
          <GlassButton disabled={loadingMore} onClick={() => void reload(true)}>
            {loadingMore ? t("common.loading") : `加载更多（${nodes.length}/${total}）`}
          </GlassButton>
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
        onNodesChanged={() => void reload(false)}
      />
    </div>
  );
}
