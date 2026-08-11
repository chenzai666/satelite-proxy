import { useCallback, useEffect, useMemo, useState } from "react";
import {
  generateSingboxConfig,
  getProxyStatus,
  getSettings,
  listAllNodes,
  setCurrentNode,
  testNodesLatency,
} from "../api";
import { GlassButton } from "../components/GlassButton";
import { useI18n } from "../i18n";
import { GlassSeg } from "../components/GlassSeg";
import type { ProxyNode, SortMode, ViewMode } from "../types";

/** Render latency cell: spinner / ms / timeout / dash */
function LatencyDisplay({
  ms,
  latencyAt,
  testing,
}: {
  ms?: number | null;
  latencyAt?: number | null;
  testing: boolean;
}) {
  if (testing) {
    return <span className="lat-spinner" aria-label="测试中" />;
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
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>(() => {
    return (localStorage.getItem("nodes.viewMode") as ViewMode) || "list";
  });
  const [sortMode, setSortMode] = useState<SortMode>(() => {
    return (localStorage.getItem("nodes.sortMode") as SortMode) || "default";
  });

  const [testing, setTesting] = useState(false);
  const [testingIds, setTestingIds] = useState<Set<string>>(new Set());

  const reload = useCallback(async () => {
    setError(null);
    try {
      const [list, settings] = await Promise.all([listAllNodes(), getSettings()]);
      setNodes(list);
      setCurrentId(settings.current_node_id ?? null);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  useEffect(() => {
    localStorage.setItem("nodes.viewMode", viewMode);
  }, [viewMode]);

  useEffect(() => {
    localStorage.setItem("nodes.sortMode", sortMode);
  }, [sortMode]);

  const displayed = useMemo(() => {
    const q = query.trim().toLowerCase();
    let list = nodes;
    if (q) {
      list = list.filter(
        (n) =>
          n.name.toLowerCase().includes(q) ||
          n.server.toLowerCase().includes(q) ||
          n.protocol.toLowerCase().includes(q) ||
          (n.subscription_name?.toLowerCase().includes(q) ?? false),
      );
    }

    const sorted = [...list];
    if (sortMode === "name") {
      sorted.sort((a, b) =>
        a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
      );
    } else if (sortMode === "latency") {
      sorted.sort((a, b) => {
        const la = a.latency_ms;
        const lb = b.latency_ms;
        // tested timeout (latency_at set, no ms) sort last among tested
        const sa = la != null ? la : a.latency_at != null ? 999999 : 9999999;
        const sb = lb != null ? lb : b.latency_at != null ? 999999 : 9999999;
        if (sa !== sb) return sa - sb;
        return a.name.localeCompare(b.name);
      });
    }
    return sorted;
  }, [nodes, query, sortMode]);

  async function onSelect(id: string) {
    setBusyId(id);
    setError(null);
    try {
      await setCurrentNode(id);
      setCurrentId(id);
      // Running: Clash API hot-switch — UI selection is enough feedback.
      // Stopped: write active.json so next start uses the new node.
      const status = await getProxyStatus().catch(() => null);
      if (!status?.running) {
        await generateSingboxConfig();
      }
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusyId(null);
    }
  }

  async function onTestLatency() {
    if (testing || displayed.length === 0) return;
    setTesting(true);
    setError(null);
    // no top banner / completion message
    const ids = displayed.map((n) => n.id);
    setTestingIds(new Set(ids));

    // clear prior latency so only spinner shows while testing
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
            // null = failed → show timeout; number = success
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

  return (
    <div className="page nodes-page">
      <header className="page-header">
        <div>
          <h1>{t("nodes.title")}</h1>
          <p className="page-desc">
            {t("nodes.desc")}
            {" · "}
            <span className="mono">
              {query.trim() && displayed.length !== nodes.length
                ? t("nodes.countFiltered", {
                    shown: displayed.length,
                    total: nodes.length,
                  })
                : t("nodes.count", { n: nodes.length })}
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

      {error && <div className="banner error">{error}</div>}

      {loading ? (
        <div className="empty">{t("common.loading")}</div>
      ) : displayed.length === 0 ? (
        <div className="empty card muted">
          {nodes.length === 0 ? t("nodes.empty") : "—"}
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
            <tbody>
              {displayed.map((n) => {
                const active = n.id === currentId;
                const isTesting = testingIds.has(n.id);
                return (
                  <tr
                    key={n.id}
                    className={active ? "row-active" : undefined}
                    onClick={() => void onSelect(n.id)}
                    style={{ cursor: "pointer" }}
                  >
                    <td>{active ? "●" : "○"}</td>
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
                      <LatencyDisplay
                        ms={n.latency_ms}
                        latencyAt={n.latency_at}
                        testing={isTesting}
                      />
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      ) : (
        <div className="node-grid">
          {displayed.map((n) => {
            const active = n.id === currentId;
            const isTesting = testingIds.has(n.id);
            return (
              <button
                key={n.id}
                type="button"
                className={`node-card ${active ? "active" : ""}`}
                onClick={() => void onSelect(n.id)}
                disabled={busyId === n.id}
              >
                <div className="node-card-top">
                  <span className="node-dot">{active ? "●" : "○"}</span>
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
                  <span className="node-card-latency">
                    <LatencyDisplay
                      ms={n.latency_ms}
                      latencyAt={n.latency_at}
                      testing={isTesting}
                    />
                  </span>
                </div>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
