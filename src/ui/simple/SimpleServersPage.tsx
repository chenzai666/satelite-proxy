import { useCallback, useEffect, useMemo, useState } from "react";
import {
  activateSubscription,
  addSubscriptionFile,
  addSubscriptionUrl,
  getProxyStatus,
  getSettings,
  listAllNodes,
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
import { GlassSeg } from "../../components/GlassSeg";
import { useImportIntent } from "../../ImportIntentContext";
import type { ProxyNode, SortMode, SubscriptionView } from "../../types";

const SORT_KEY = "simple.nodes.sortMode";
const SUBS_COLLAPSE_KEY = "simple.nodes.subsCollapsed";

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
  const { prefill, token, consume, dismiss } = useImportIntent();
  const [subs, setSubs] = useState<SubscriptionView[]>([]);
  const [nodes, setNodes] = useState<ProxyNode[]>([]);
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

  const reload = useCallback(async () => {
    try {
      const [s, n, settings] = await Promise.all([
        listSubscriptions(),
        listAllNodes(),
        getSettings(),
      ]);
      setSubs(s);
      setNodes(n);
      setCurrentId(settings.current_node_id ?? null);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

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

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    let list = nodes;
    if (q) {
      list = list.filter(
        (n) =>
          n.name.toLowerCase().includes(q) ||
          n.protocol.toLowerCase().includes(q) ||
          n.server.toLowerCase().includes(q),
      );
    }
    const sorted = [...list];
    if (sortMode === "name") {
      sorted.sort((a, b) =>
        a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
      );
    } else if (sortMode === "latency") {
      // Low latency first; timeout next; untested last.
      sorted.sort((a, b) => {
        const la = a.latency_ms;
        const lb = b.latency_ms;
        const sa = la != null ? la : a.latency_at != null ? 999999 : 9999999;
        const sb = lb != null ? lb : b.latency_at != null ? 999999 : 9999999;
        if (sa !== sb) return sa - sb;
        return a.name.localeCompare(b.name);
      });
    }
    return sorted;
  }, [nodes, query, sortMode]);

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
      const [n, settings, status] = await Promise.all([
        listAllNodes(),
        getSettings(),
        getProxyStatus().catch(() => null),
      ]);
      setNodes(n);
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
    if (testing || nodes.length === 0) return;
    const ids = nodes.map((n) => n.id);
    setTesting(true);
    setTestingIds(new Set(ids));
    setError(null);
    // Clear prior latency so UI shows spinner while probing.
    setNodes((prev) =>
      prev.map((n) =>
        ids.includes(n.id)
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
      } else {
        await addSubscriptionFile(
          payload.name || null,
          payload.path ?? "",
          !!payload.autoUpdate,
          payload.autoUpdateIntervalMin ?? 1440,
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
    <div className="simple-page simple-servers">
      <header className="simple-page-head">
        <div>
          <div className="simple-kicker muted">LIBRARY</div>
          <h1 className="simple-title">节点</h1>
        </div>
        <div className="simple-head-actions">
          <button
            type="button"
            className="btn-pill secondary"
            disabled={testing || nodes.length === 0}
            onClick={() => void onTestAll()}
          >
            {testing ? "测速中…" : "测速"}
          </button>
          <button
            type="button"
            className="btn-pill"
            onClick={() => {
              setModalError(null);
              setModalInitial(null);
              setModalOpen(true);
            }}
          >
            添加
          </button>
        </div>
      </header>

      <input
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
        className="search simple-search"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="搜索节点…"
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
              订阅配置
              {subsCollapsed && activeSubName
                ? ` · ${activeSubName}`
                : " · 点击切换"}
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
                  className={`simple-card simple-sub-row ${active ? "active" : ""}`}
                  disabled={busy}
                  onClick={() => void onSelectSub(s.id)}
                  aria-pressed={active}
                >
                  <span className="simple-radio" aria-hidden>
                    {active ? "●" : "○"}
                  </span>
                  <strong className="simple-sub-name">{s.name}</strong>
                  <span className="muted simple-sub-meta">
                    {s.node_count}节点
                    {s.auto_update ? " · 自动" : ""}
                    {active ? " · 使用中" : ""}
                  </span>
                  <button
                    type="button"
                    className="btn-pill secondary simple-sub-refresh"
                    disabled={busy}
                    onClick={(e) => void onRefreshSub(s.id, e)}
                  >
                    刷新
                  </button>
                </button>
              );
            })}
        </section>
      )}

      <section className="simple-section">
        <div className="simple-section-label muted">
          节点 · {filtered.length}
          {activeSubId
            ? ` · ${subs.find((s) => s.id === activeSubId)?.name ?? ""}`
            : ""}
        </div>
        <div className="simple-sort-row">
          <span className="muted simple-sort-label">排序</span>
          <GlassSeg
            value={sortMode}
            ariaLabel="节点排序"
            onChange={(v) => setSortMode(v as SortMode)}
            options={[
              { value: "latency", label: "延迟" },
              { value: "name", label: "名称" },
              { value: "default", label: "默认" },
            ]}
          />
        </div>
        {filtered.length === 0 ? (
          <div className="simple-card empty muted">
            {subs.length === 0
              ? "暂无节点，请先添加订阅"
              : "当前订阅无节点，点上方切换其他配置或刷新"}
          </div>
        ) : (
          <ul className="simple-node-list">
            {filtered.map((n) => {
              const active = n.id === currentId;
              return (
                <li key={n.id}>
                  <button
                    type="button"
                    className={`simple-card simple-node-item ${active ? "active" : ""}`}
                    disabled={busy}
                    onClick={() => void onSelectNode(n.id)}
                  >
                    <span className="simple-radio" aria-hidden>
                      {active ? "●" : "○"}
                    </span>
                    <span className="pill target-proxy">
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
          </ul>
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
