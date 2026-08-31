import { useCallback, useEffect, useState } from "react";
import {
  addSubscriptionFile,
  addSubscriptionNode,
  addSubscriptionSingbox,
  addSubscriptionText,
  addSubscriptionUrl,
  deleteNodes,
  getNodeShareUri,
  getProxyStatus,
  getSettings,
  listCustomConfigNodes,
  listNodeIds,
  listNodesPage,
  setCurrentNode,
  testCustomNodesLatency,
  testNodesLatency,
} from "../../api";
import {
  applyCustomLatency,
  filterCustomNodes,
  type CustomLatencyMap,
} from "../../customNodes";
import {
  AddConfigModal,
  type ConfigFormValues,
} from "../../components/AddConfigModal";
import { GlassButton } from "../../components/GlassButton";
import { GlassSeg } from "../../components/GlassSeg";
import { EditLocalNodesModal } from "../../components/EditLocalNodesModal";
import { NodeContextMenu, type NodeContextMenuState } from "../../components/NodeContextMenu";
import { NodeShareModal } from "../../components/NodeShareModal";
import { useImportIntent } from "../../ImportIntentContext";
import { useI18n } from "../../i18n";
import { ErrorModal } from "../../components/ErrorModal";
import { waitForCoreRestart } from "../../coreBusy";
import { useVirtualRange } from "../../hooks/useVirtualRange";
import { copyNodeShareText } from "../../nodeShare";
import type { AutoSelectMode, ProxyNode, SortMode } from "../../types";

const SORT_KEY = "simple.nodes.sortMode";
const VIRTUALIZE_AFTER = 200;
const NODE_ROW_HEIGHT = 48;
const PAGE_SIZE = 200;

function readSortMode(): SortMode {
  try {
    const v = localStorage.getItem(SORT_KEY);
    if (v === "latency" || v === "name" || v === "default") return v;
  } catch {
    /* ignore */
  }
  return "latency";
}

/** Latency colors: green <200 · yellow <300 · red ≥300 (same as Nodes / Connect). */
function latencyClass(ms?: number | null) {
  if (ms == null || ms < 0) return "lat-none";
  if (ms < 200) return "lat-good";
  if (ms < 300) return "lat-ok";
  return "lat-slow";
}

function LatencyLabel({
  ms,
  testedAt,
  testing,
}: {
  ms?: number | null;
  testedAt?: number | null;
  testing?: boolean;
}) {
  if (testing) {
    return <span className="lat lat-spinner" aria-label="测速中" />;
  }
  if (ms != null && ms >= 0) {
    return <span className={`lat mono ${latencyClass(ms)}`}>{ms}ms</span>;
  }
  if (testedAt != null) {
    return <span className="lat lat-timeout mono">timeout</span>;
  }
  return <span className="lat lat-none mono">—</span>;
}

export function SimpleServersPage() {
  const { t } = useI18n();
  const { prefill, token, consume, dismiss } = useImportIntent();
  const [nodes, setNodes] = useState<ProxyNode[]>([]);
  const [nodeTotal, setNodeTotal] = useState(0);
  const [loadingMore, setLoadingMore] = useState(false);
  const [currentId, setCurrentId] = useState<string | null>(null);
  const [runtimeSource, setRuntimeSource] = useState("generated");
  // Session-only latency results for custom-mode nodes (not persisted backend-side).
  const [customLatency, setCustomLatency] = useState<CustomLatencyMap>(new Map());
  const [sortMode, setSortMode] = useState<SortMode>(() => readSortMode());
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [autoSelect, setAutoSelect] = useState<AutoSelectMode>("off");
  // Manual click in kernel-auto mode: urltest → selector rebuild restarts the core.
  const [switching, setSwitching] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testingIds, setTestingIds] = useState<Set<string>>(new Set());
  const [modalOpen, setModalOpen] = useState(false);
  const [modalBusy, setModalBusy] = useState(false);
  const [modalError, setModalError] = useState<string | null>(null);
  const [modalInitial, setModalInitial] = useState<ConfigFormValues | null>(
    null,
  );
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [batchBusy, setBatchBusy] = useState(false);
  const [contextMenu, setContextMenu] = useState<NodeContextMenuState | null>(null);
  const [shareNode, setShareNode] = useState<ProxyNode | null>(null);
  const [editNode, setEditNode] = useState<ProxyNode | null>(null);
  const [shareNotice, setShareNotice] = useState<string | null>(null);


  const reload = useCallback(async (append = false) => {
    try {
      if (append) setLoadingMore(true);
      const settings = await getSettings();
      setCurrentId(settings.current_node_id ?? null);
      setRuntimeSource(settings.runtime_source || "generated");
      setAutoSelect((settings.auto_select as AutoSelectMode) ?? "off");
      const offset = append ? nodes.length : 0;
      if ((settings.runtime_source || "generated").startsWith("singbox:")) {
        // Custom mode: read-only nodes extracted from the sing-box config,
        // overlaid with this session's latency results.
        const all = applyCustomLatency(await listCustomConfigNodes(), customLatency);
        const filtered = filterCustomNodes(all, "", sortMode, offset, PAGE_SIZE);
        setNodes((prev) => (append ? [...prev, ...filtered.nodes] : filtered.nodes));
        setNodeTotal(filtered.total);
      } else {
        const page = await listNodesPage("", sortMode, offset, PAGE_SIZE);
        setNodes((prev) => (append ? [...prev, ...page.nodes] : page.nodes));
        setNodeTotal(page.total);
      }
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setLoadingMore(false);
    }
  }, [nodes.length, sortMode, customLatency]);

  useEffect(() => {
    setSelectedIds(new Set());
    const timer = window.setTimeout(() => void reload(false), 150);
    return () => window.clearTimeout(timer);
    // nodes.length changes when appending and must not reset pagination.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sortMode]);

  // One-click subscribe deep link → open add modal prefilled.
  useEffect(() => {
    if (!token || !prefill) return;
    setModalError(null);
    setModalInitial({
      name: prefill.name ?? "",
      kind: "url",
      url: prefill.url,
      autoUpdate: true,
      autoUpdateIntervalMin: 1440,
    });
    setModalOpen(true);
    consume();
  }, [token, prefill, consume]);

  useEffect(() => {
    try {
      localStorage.setItem(SORT_KEY, sortMode);
    } catch {
      /* ignore */
    }
  }, [sortMode]);

  const customRuntime = runtimeSource.startsWith("singbox:");

  const filtered = nodes;
  const virtualized = filtered.length > VIRTUALIZE_AFTER;
  const nodeRange = useVirtualRange({
    itemCount: filtered.length,
    itemSize: NODE_ROW_HEIGHT,
    enabled: virtualized,
  });

  async function onSelectNode(id: string) {
    if (busy || switching || batchBusy || id === currentId) return;
    setBusy(true);
    setError(null);
    try {
      const leavingKernel = autoSelect === "kernel";
      await setCurrentNode(id);
      setCurrentId(id);
      setAutoSelect("off");
      if (leavingKernel) {
        // urltest → selector rebuild: when running, hold the busy state until
        // the core restart finishes (stopped cores just persist the pick).
        const status = await getProxyStatus().catch(() => null);
        if (status?.running) {
          setSwitching(true);
          await waitForCoreRestart();
        }
      }
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setSwitching(false);
      setBusy(false);
    }
  }

  async function onTestNodes(ids: string[]) {
    if (testing || ids.length === 0) return;
    const idSet = new Set(ids);
    setTesting(true);
    setTestingIds(idSet);
    setError(null);
    // Clear prior latency so UI shows spinner while probing.
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

  async function onTestAll() {
    const ids = customRuntime ? nodes.map((n) => n.id) : await listNodeIds();
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

  const selectAllNodes = useCallback(async () => {
    if (customRuntime || batchBusy) return;
    try {
      setSelectedIds(new Set(await listNodeIds()));
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }, [batchBusy, customRuntime]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== "a") return;
      const target = event.target as HTMLElement | null;
      if (target?.closest("input, textarea, [contenteditable='true']")) return;
      event.preventDefault();
      void selectAllNodes();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [selectAllNodes]);

  async function deleteSelected(ids: string[]) {
    if (batchBusy || ids.length === 0) return;
    const name = nodes.find((node) => node.id === ids[0])?.name ?? ids[0];
    const confirmed = window.confirm(
      ids.length === 1
        ? t("nodes.deleteConfirm", { name })
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

  async function onAdd(payload: ConfigFormValues) {
    setModalBusy(true);
    setModalError(null);
    try {
      if (payload.kind === "url") {
        await addSubscriptionUrl(
          payload.name || null,
          payload.url ?? "",
          !!payload.viaProxy,
          !!payload.autoUpdate,
          payload.autoUpdateIntervalMin ?? 1440,
        );
      } else if (payload.kind === "file") {
        await addSubscriptionFile(
          payload.name || null,
          payload.path ?? "",
          !!payload.autoUpdate,
          payload.autoUpdateIntervalMin ?? 1440,
        );
      } else if (payload.kind === "text") {
        await addSubscriptionText(payload.name || null, payload.content ?? "");
      } else if (payload.kind === "singbox") {
        await addSubscriptionSingbox(
          payload.name || null,
          payload.content ?? "",
          null,
        );
      } else {
        await addSubscriptionNode(
          payload.name || null,
          payload.uri ?? null,
          payload.node ?? null,
        );
      }
      setModalOpen(false);
      setModalInitial(null);
      dismiss();
      await reload();
    } catch (e) {
      setModalError(typeof e === "string" ? e : String(e));
    } finally {
      setModalBusy(false);
    }
  }

  return (
    <div className="page simple-page simple-servers">
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
            <span className="mono">{t("nodes.count", { n: nodeTotal })}</span>
          </p>
        </div>
        <div className="header-actions simple-head-actions">
          <GlassButton
            variant="primary"
            icon="⚡"
            disabled={testing || nodeTotal === 0}
            onClick={() => void onTestAll()}
            title={t("nodes.testRealLatency")}
          >
            {testing ? t("nodes.testing") : t("nodes.testRealLatency")}
          </GlassButton>
        </div>
      </header>

      {!customRuntime && selectedIds.size > 0 && (
        <div className="node-batch-toolbar" role="status">
          <span>{t("nodes.selectedCount", { n: selectedIds.size })}</span>
          <GlassButton disabled={testing || batchBusy} onClick={() => void onTestNodes([...selectedIds])}>
            {t("nodes.testSelected")}
          </GlassButton>
          <GlassButton variant="danger" disabled={batchBusy} onClick={() => void deleteSelected([...selectedIds])}>
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

      <section className="simple-section">
        <div className="simple-sort-row">
          <span className="dash-inline-label">{t("nodes.sortLatency")}</span>
          <GlassSeg
            value={sortMode}
            ariaLabel={t("nodes.sortLatency")}
            onChange={(v) => setSortMode(v as SortMode)}
            options={[
              { value: "latency", label: t("nodes.sortLatency") },
              { value: "name", label: t("nodes.sortName") },
              { value: "default", label: t("nodes.sortDefault") },
            ]}
          />
        </div>
        {filtered.length === 0 ? (
          <div className="empty card muted">
            {customRuntime ? t("nodes.customEmpty") : t("nodes.empty")}
          </div>
        ) : (
          <ul
            className={`simple-node-list ${virtualized ? "virtualized" : ""}`}
            ref={nodeRange.containerRef as React.RefObject<HTMLUListElement>}
          >
            {nodeRange.paddingTop > 0 && (
              <li
                className="node-virtual-spacer"
                style={{ height: nodeRange.paddingTop }}
                aria-hidden="true"
              />
            )}
            {filtered.slice(nodeRange.start, nodeRange.end).map((n) => {
              const active = n.id === currentId;
              const selected = selectedIds.has(n.id);
              return (
                <li key={n.id}>
                  <div
                    className={`simple-node-item ${active ? "active" : ""} ${selected ? "selected" : ""}`}
                    onClick={(event) => {
                      if (!customRuntime && (event.ctrlKey || event.metaKey)) toggleSelected(n.id);
                    }}
                    onContextMenu={(event) => {
                      if (customRuntime) return;
                      event.preventDefault();
                      setContextMenu({ node: n, x: event.clientX, y: event.clientY });
                    }}
                  >
                    <button
                      type="button"
                      className="node-select-dot simple-radio"
                      aria-label={`切换到 ${n.name}`}
                      aria-pressed={active}
                      disabled={busy || customRuntime || batchBusy}
                      onClick={(event) => {
                        event.stopPropagation();
                        void onSelectNode(n.id);
                      }}
                    >
                      {active ? "●" : "○"}
                    </button>
                    <span className="simple-node-proto mono">
                      {n.protocol.toUpperCase()}
                    </span>
                    <span className="simple-node-item-name">{n.name}</span>
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
                      <LatencyLabel
                        ms={n.latency_ms}
                        testedAt={n.latency_at}
                        testing={testingIds.has(n.id)}
                      />
                    </button>
                  </div>
                </li>
              );
            })}
            {nodeRange.paddingBottom > 0 && (
              <li
                className="node-virtual-spacer"
                style={{ height: nodeRange.paddingBottom }}
                aria-hidden="true"
              />
            )}
          </ul>
        )}
        {nodes.length < nodeTotal && (
          <GlassButton
            disabled={loadingMore}
            onClick={() => void reload(true)}
            className="simple-load-more"
          >
            {loadingMore
              ? t("common.loading")
              : t("simple.loadMore", {
                  shown: nodes.length,
                  total: nodeTotal,
                })}
          </GlassButton>
        )}
      </section>

      <AddConfigModal
        open={modalOpen}
        busy={modalBusy}
        error={modalError}
        onDismissError={() => setModalError(null)}
        isEdit={false}
        initial={modalInitial}
        onClose={() => {
          if (modalBusy) return;
          setModalOpen(false);
          setModalInitial(null);
          dismiss();
        }}
        onSubmit={(p) => void onAdd(p)}
      />
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
