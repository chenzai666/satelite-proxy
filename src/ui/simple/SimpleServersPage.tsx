import { useCallback, useEffect, useMemo, useState } from "react";
import {
  activateSubscription,
  addSubscriptionFile,
  addSubscriptionNode,
  addSubscriptionSingbox,
  addSubscriptionText,
  addSubscriptionUrl,
  getProxyStatus,
  getSettings,
  listNodeIds,
  listNodesPage,
  listSubscriptions,
  refreshSubscription,
  restartProxy,
  setCurrentNode,
  testNodesLatency,
} from "../../api";
import {
  AddConfigModal,
  type ConfigFormValues,
} from "../../components/AddConfigModal";
import { GlassButton } from "../../components/GlassButton";
import { GlassSeg } from "../../components/GlassSeg";
import { useImportIntent } from "../../ImportIntentContext";
import { useI18n } from "../../i18n";
import { useVirtualRange } from "../../hooks/useVirtualRange";
import type { ProxyNode, SortMode, SubscriptionView } from "../../types";

const SORT_KEY = "simple.nodes.sortMode";
const SUBS_COLLAPSE_KEY = "simple.nodes.subsCollapsed";
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

function readSubsCollapsed(): boolean {
  try {
    return localStorage.getItem(SUBS_COLLAPSE_KEY) === "1";
  } catch {
    return false;
  }
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
  const [subs, setSubs] = useState<SubscriptionView[]>([]);
  const [nodes, setNodes] = useState<ProxyNode[]>([]);
  const [nodeTotal, setNodeTotal] = useState(0);
  const [loadingMore, setLoadingMore] = useState(false);
  const [currentId, setCurrentId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [sortMode, setSortMode] = useState<SortMode>(() => readSortMode());
  const [subsCollapsed, setSubsCollapsed] = useState(() => readSubsCollapsed());
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testingIds, setTestingIds] = useState<Set<string>>(new Set());
  const [modalOpen, setModalOpen] = useState(false);
  const [modalBusy, setModalBusy] = useState(false);
  const [modalError, setModalError] = useState<string | null>(null);
  const [modalInitial, setModalInitial] = useState<ConfigFormValues | null>(
    null,
  );


  const reload = useCallback(async (append = false) => {
    try {
      if (append) setLoadingMore(true);
      const [s, page, settings] = await Promise.all([
        listSubscriptions(),
        listNodesPage(query, sortMode, append ? nodes.length : 0, PAGE_SIZE),
        getSettings(),
      ]);
      setSubs(s);
      setNodes((prev) => (append ? [...prev, ...page.nodes] : page.nodes));
      setNodeTotal(page.total);
      setCurrentId(settings.current_node_id ?? null);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setLoadingMore(false);
    }
  }, [nodes.length, query, sortMode]);

  useEffect(() => {
    const timer = window.setTimeout(() => void reload(false), 150);
    return () => window.clearTimeout(timer);
    // nodes.length changes when appending and must not reset pagination.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, sortMode]);

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

  useEffect(() => {
    try {
      localStorage.setItem(SUBS_COLLAPSE_KEY, subsCollapsed ? "1" : "0");
    } catch {
      /* ignore */
    }
  }, [subsCollapsed]);

  const activeSubId = useMemo(
    () => subs.find((s) => s.enabled)?.id ?? null,
    [subs],
  );

  const activeSubName = useMemo(
    () => subs.find((s) => s.id === activeSubId)?.name ?? null,
    [subs, activeSubId],
  );

  const filtered = nodes;
  const virtualized = filtered.length > VIRTUALIZE_AFTER;
  const nodeRange = useVirtualRange({
    itemCount: filtered.length,
    itemSize: NODE_ROW_HEIGHT,
    enabled: virtualized,
  });

  async function onSelectNode(id: string) {
    if (busy || id === currentId) return;
    setBusy(true);
    setError(null);
    try {
      await setCurrentNode(id);
      setCurrentId(id);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
  }

  /** Switch which subscription is active (exclusive); rebuild core if running. */
  async function onSelectSub(id: string) {
    if (busy) return;
    if (activeSubId === id) return;
    setBusy(true);
    setError(null);
    try {
      const list = await activateSubscription(id);
      setSubs(list);
      // Reload nodes for the newly enabled profile(s).
      const [page, settings, status] = await Promise.all([
        listNodesPage(query, sortMode, 0, PAGE_SIZE),
        getSettings(),
        getProxyStatus().catch(() => null),
      ]);
      setNodes(page.nodes);
      setNodeTotal(page.total);
      setCurrentId(settings.current_node_id ?? null);
      // Apply new node pool if core is running.
      if (status?.running) {
        await restartProxy();
      }
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function onTestAll() {
    if (testing || nodeTotal === 0) return;
    const ids = await listNodeIds(query);
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
      const batch = await testNodesLatency(ids, 3000);
      const map = new Map(batch.results.map((r) => [r.id, r]));
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
      await reload();
    } finally {
      setTesting(false);
      setTestingIds(new Set());
      await reload(false);
    }
  }

  async function onRefreshSub(id: string, e: React.MouseEvent) {
    e.stopPropagation();
    e.preventDefault();
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await refreshSubscription(id);
      await reload();
    } catch (err) {
      setError(typeof err === "string" ? err : String(err));
    } finally {
      setBusy(false);
    }
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
      <header className="page-header">
        <div>
          <h1>{t("nodes.title")}</h1>
          <p className="page-desc">
            {t("nodes.desc")}
            {" · "}
            <span className="mono">
              {query.trim()
                ? t("nodes.countFiltered", {
                    shown: filtered.length,
                    total: nodeTotal,
                  })
                : t("nodes.count", { n: nodeTotal })}
            </span>
          </p>
        </div>
        <div className="header-actions simple-head-actions">
          <GlassButton
            variant="primary"
            icon="⚡"
            disabled={testing || nodeTotal === 0}
            onClick={() => void onTestAll()}
            title={t("nodes.testLatency")}
          >
            {testing ? t("nodes.testing") : t("nodes.testLatency")}
          </GlassButton>
          <GlassButton
            onClick={() => {
              setModalError(null);
              setModalInitial(null);
              setModalOpen(true);
            }}
          >
            {t("config.add")}
          </GlassButton>
        </div>
      </header>

      <input
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
        className="search simple-search"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder={t("nodes.search")}
      />

      {error && <div className="banner error">{error}</div>}

      {subs.length > 0 && (
        <section className="simple-section">
          <button
            type="button"
            className="simple-section-toggle"
            aria-expanded={!subsCollapsed}
            onClick={() => setSubsCollapsed((v) => !v)}
          >
            <span className="simple-section-label muted">
              {t("simple.subs")}
              {subsCollapsed && activeSubName
                ? ` · ${activeSubName}`
                : ` · ${t("config.clickUse")}`}
            </span>
            <span
              className={`simple-collapse-caret muted ${subsCollapsed ? "collapsed" : ""}`}
              aria-hidden
            />
          </button>
          {!subsCollapsed &&
            subs.map((s) => {
              const active = s.enabled;
              return (
                <button
                  key={s.id}
                  type="button"
                  className={`card simple-sub-row ${active ? "active" : ""}`}
                  disabled={busy}
                  onClick={() => void onSelectSub(s.id)}
                  aria-pressed={active}
                >
                  <span className="simple-radio node-dot" aria-hidden>
                    {active ? "●" : "○"}
                  </span>
                  <strong className="simple-sub-name">{s.name}</strong>
                  <span className="muted simple-sub-meta">
                    {t("config.nodes", { n: s.node_count })}
                    {s.auto_update ? ` · ${t("common.enabled")}` : ""}
                    {active ? ` · ${t("config.using")}` : ""}
                  </span>
                  <GlassButton
                    className="simple-sub-refresh"
                    disabled={busy}
                    onClick={(e) => void onRefreshSub(s.id, e)}
                  >
                    {t("common.refresh")}
                  </GlassButton>
                </button>
              );
            })}
        </section>
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
            {subs.length === 0 ? t("nodes.empty") : t("nodes.empty")}
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
              return (
                <li key={n.id}>
                  <button
                    type="button"
                    className={`simple-node-item ${active ? "active" : ""}`}
                    disabled={busy}
                    onClick={() => void onSelectNode(n.id)}
                  >
                    <span className="simple-radio node-dot" aria-hidden>
                      {active ? "●" : "○"}
                    </span>
                    <span className="simple-node-proto mono">
                      {n.protocol.toUpperCase()}
                    </span>
                    <span className="simple-node-item-name">{n.name}</span>
                    <LatencyLabel
                      ms={n.latency_ms}
                      testedAt={n.latency_at}
                      testing={testingIds.has(n.id)}
                    />
                  </button>
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
    </div>
  );
}
