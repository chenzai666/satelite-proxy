import { useCallback, useEffect, useState } from "react";
import {
  deleteNodes,
  generateSingboxConfig,
  getNodeShareUri,
  getProxyStatus,
  getSettings,
  listCustomConfigNodes,
  listNodeIds,
  listNodesPage,
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
import { GlassSeg } from "../components/GlassSeg";
import { waitForCoreRestart } from "../coreBusy";
import { useVirtualRange } from "../hooks/useVirtualRange";
import { filterCustomNodes, applyCustomLatency, type CustomLatencyMap } from "../customNodes";
import { copyNodeShareText } from "../nodeShare";
import type { AutoSelectMode, ProxyNode, SortMode, ViewMode } from "../types";

const VIRTUALIZE_AFTER = 200;
const LIST_ROW_HEIGHT = 49;
const GRID_ROW_HEIGHT = 94;
const PAGE_SIZE = 200;

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
}: {
  ms?: number | null;
  latencyAt?: number | null;
  testing: boolean;
  unsupported?: boolean;
}) {
  const { t } = useI18n();
  if (testing) {
    return <span className="lat-spinner" aria-label="测试中" />;
  }
  if (unsupported) {
    return <span className="lat lat-none" title={t("nodes.latencyNeedsCore")}>{t("nodes.latencyNeedsCore")}</span>;
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
  const { t } = useI18n();
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

  const [customRuntime, setCustomRuntime] = useState(false);
  // Session-only latency results for custom-mode nodes (not persisted backend-side).
  const [customLatency, setCustomLatency] = useState<CustomLatencyMap>(new Map());
  const [testing, setTesting] = useState(false);
  const [testingIds, setTestingIds] = useState<Set<string>>(new Set());
  // Node ids whose last test used method "unsupported" (UDP-only protocol,
  // core not running) — shown as "start core to test" instead of "timeout".
  const [unsupportedIds, setUnsupportedIds] = useState<Set<string>>(new Set());
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

  const reload = useCallback(async (append = false) => {
    setError(null);
    if (append) setLoadingMore(true);
    try {
      const settings = await getSettings();
      const custom = (settings.runtime_source ?? "generated").startsWith("singbox:");
      setCustomRuntime(custom);
      setCurrentId(settings.current_node_id ?? null);
      setAutoSelect((settings.auto_select as AutoSelectMode) ?? "off");
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

  const displayed = nodes;
  const virtualized = displayed.length > VIRTUALIZE_AFTER;
  const listRange = useVirtualRange({
    itemCount: displayed.length,
    itemSize: LIST_ROW_HEIGHT,
    enabled: virtualized,
  });
  const gridRange = useVirtualRange({
    itemCount: displayed.length,
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

  async function onTestNodes(ids: string[]) {
    if (testing || ids.length === 0) return;
    setTesting(true);
    setError(null);
    // no top banner / completion message
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

    try {
      const batch = customRuntime
        ? await testCustomNodesLatency(3000)
        : await testNodesLatency(ids, 3000);
      const map = new Map(batch.results.map((r) => [r.id, r]));
      setUnsupportedIds(
        new Set(batch.results.filter((r) => r.method === "unsupported").map((r) => r.id)),
      );
      if (customRuntime) {
        // Session-only — remember results across filter / sort / page reloads.
        setCustomLatency((prev) => {
          const next = new Map(prev);
          for (const r of batch.results) {
            next.set(r.id, { ms: r.latency_ms ?? null, at: r.tested_at });
          }
          return next;
        });
      }
      setNodes((prev) =>
        prev.map((n) => {
          const r = map.get(n.id);
          if (!r) return n;
          return {
            ...n,
            // null = failed → show timeout; number = success
            latency_ms: r.latency_ms ?? null,
            latency_at: r.tested_at,
          };
        }),
      );
    } catch (e) {
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

  async function onTestLatency() {
    const ids = customRuntime ? nodes.map((n) => n.id) : await listNodeIds(query);
    await onTestNodes(ids);
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

          <GlassButton
            variant="primary"
            icon="⚡"
            disabled={testing || displayed.length === 0}
            onClick={() => void onTestLatency()}
            title={t("nodes.testLatency")}
          >
            {testing ? t("nodes.testing") : t("nodes.testLatency")}
          </GlassButton>

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
        <div className="card table-wrap">
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
              {displayed.slice(listRange.start, listRange.end).map((n) => {
                const active = n.id === currentId;
                const isTesting = testingIds.has(n.id);
                const selected = selectedIds.has(n.id);
                return (
                  <tr
                    key={n.id}
                    className={`node-virtual-row ${active ? "row-active" : ""} ${selected ? "row-selected" : ""}`}
                    onClick={(event) => {
                      if (!customRuntime && (event.ctrlKey || event.metaKey)) toggleSelected(n.id);
                    }}
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
                          void onTestNodes([n.id]);
                        }}
                      >
                        <LatencyDisplay
                          ms={n.latency_ms}
                          latencyAt={n.latency_at}
                          testing={isTesting}
                          unsupported={unsupportedIds.has(n.id)}
                        />
                      </button>
                    </td>
                  </tr>
                );
              })}
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
          <div className={`node-grid ${virtualized ? "node-grid-virtual" : ""}`}>
            {displayed.slice(gridRange.start, gridRange.end).map((n) => {
              const active = n.id === currentId;
              const isTesting = testingIds.has(n.id);
              const selected = selectedIds.has(n.id);
              return (
                <div
                  key={n.id}
                  className={`node-card ${active ? "active" : ""} ${selected ? "selected" : ""}`}
                  onClick={(event) => {
                    if (!customRuntime && (event.ctrlKey || event.metaKey)) toggleSelected(n.id);
                  }}
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
                    </div>
                  </div>
                  <div className="node-card-name" title={n.name}>
                    {n.name}
                  </div>
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
                        void onTestNodes([n.id]);
                      }}
                    >
                      <LatencyDisplay
                        ms={n.latency_ms}
                        latencyAt={n.latency_at}
                        testing={isTesting}
                        unsupported={unsupportedIds.has(n.id)}
                      />
                    </button>
                  </div>
                </div>
              );
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
