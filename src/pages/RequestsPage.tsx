import { useCallback, useEffect, useMemo, useState } from "react";
import { clearRequestHistory, listRequests } from "../api";
import { GlassButton } from "../components/GlassButton";
import { GlassSeg } from "../components/GlassSeg";
import { useVisibleInterval } from "../hooks/useVisibleInterval";
import { useI18n } from "../i18n";
import type { ConnectionView } from "../types";
import { scopeFilter, type TrafficScope } from "../trafficFilter";

function fmtBytes(n: number) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(2)} MB`;
}

function fmtTime(ms?: number | null) {
  if (!ms) return "—";
  try {
    return new Date(ms).toLocaleString();
  } catch {
    return String(ms);
  }
}

interface Props {
  /** When true, omit page chrome (used under Traffic tabs). */
  embedded?: boolean;
}

export function RequestsPage({ embedded = false }: Props) {
  const { t } = useI18n();
  const [rows, setRows] = useState<ConnectionView[]>([]);
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [scope, setScope] = useState<TrafficScope>("all");

  const reload = useCallback(async () => {
    try {
      const list = await listRequests(query.trim() || null, 800);
      setRows(list);
      setError(null);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setLoading(false);
    }
  }, [query]);

  useEffect(() => {
    void reload();
  }, [reload]);

  // History UI can refresh slower; journal keeps filling in Rust.
  useVisibleInterval(() => {
    void reload();
  }, 2500);

  async function onClear() {
    if (!confirm(t("req.clearConfirm"))) return;
    try {
      await clearRequestHistory();
      setRows([]);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }

  const scoped = useMemo(() => scopeFilter(rows, scope), [rows, scope]);
  const scopeOpts = useMemo(
    () => [
      { value: "all", label: t("traffic.scopeAll") },
      { value: "direct", label: t("traffic.scopeDirect") },
      { value: "proxy", label: t("traffic.scopeProxy") },
    ],
    [t],
  );

  const toolbar = (
    <div className={`traffic-toolbar ${embedded ? "" : "page-header"}`}>
      {!embedded && (
        <div>
          <h1>{t("req.title")}</h1>
          <p className="page-desc">{t("req.desc")}</p>
        </div>
      )}
      <div className="header-actions traffic-toolbar-actions">
        <input
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
          className="search"
          placeholder={t("req.filter")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <GlassSeg
          value={scope}
          ariaLabel={t("traffic.scopeLabel")}
          onChange={(v) => setScope(v as TrafficScope)}
          options={scopeOpts}
        />
        <GlassButton
          icon="↻"
          onClick={() => void reload()}
          title={t("common.refresh")}
        >
          {t("common.refresh")}
        </GlassButton>
        <GlassButton
          variant="danger"
          icon="⌫"
          onClick={() => void onClear()}
          title={t("common.clear")}
        >
          {t("common.clear")}
        </GlassButton>
      </div>
    </div>
  );

  const body = (
    <>
      {error && <div className="banner error">{error}</div>}

      <div className="muted mono traffic-meta">
        {t("req.count", { n: scoped.length })}
        {query.trim() ? t("req.filterLabel", { q: query.trim() }) : ""}
      </div>

      {loading ? (
        <div className="empty">{t("common.loading")}</div>
      ) : scoped.length === 0 ? (
        <div className="empty card muted">{t("req.empty")}</div>
      ) : (
        <div className="card table-wrap">
          <table className="conn-table">
            <thead>
              <tr>
                <th className="conn-th-time">{t("req.time")}</th>
                <th>{t("conn.dest")}</th>
                <th className="conn-th-node">{t("conn.node")}</th>
                <th className="conn-th-rule">{t("conn.rule")}</th>
                <th className="conn-th-process">{t("conn.process")}</th>
                <th className="conn-th-traffic">{t("conn.traffic")}</th>
              </tr>
            </thead>
            <tbody>
              {scoped.map((r) => (
                <tr key={`${r.id}-${r.last_seen ?? 0}`}>
                  <td className="conn-time">
                    <div
                      className="conn-cell"
                      title={`${fmtTime(r.closed_at ?? r.last_seen)}${
                        r.first_seen && r.first_seen !== (r.closed_at ?? r.last_seen)
                          ? ` · ${t("req.first", { t: fmtTime(r.first_seen) })}`
                          : ""
                      }`}
                    >
                      {fmtTime(r.closed_at ?? r.last_seen)}
                    </div>
                  </td>
                  <td>
                    <div
                      className="conn-cell conn-dest"
                      title={`${r.destination}${r.host || r.source ? ` · ${r.host || r.source}` : ""}`}
                    >
                      {r.destination}
                    </div>
                  </td>
                  <td>
                    <div
                      className="conn-cell conn-node"
                      title={
                        r.subscription_name
                          ? `${r.subscription_name} · ${r.node_name}`
                          : r.node_name
                      }
                    >
                      {r.node_name || r.node_tag || "—"}
                    </div>
                  </td>
                  <td>
                    <div className="conn-cell conn-rule" title={r.rule}>
                      {r.rule || "—"}
                    </div>
                  </td>
                  <td>
                    <div className="conn-cell" title={r.process}>
                      {r.process || "—"}
                    </div>
                  </td>
                  <td className="conn-traffic">
                    <span title={`↑${fmtBytes(r.upload)} ↓${fmtBytes(r.download)}`}>
                      <span className="tr-dir up">↑</span>{fmtBytes(r.upload)}{" "}
                      <span className="tr-dir down">↓</span>{fmtBytes(r.download)}
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );

  if (embedded) {
    return (
      <div className="traffic-embed">
        {toolbar}
        {body}
      </div>
    );
  }

  return (
    <div className="page">
      {toolbar}
      {body}
    </div>
  );
}
