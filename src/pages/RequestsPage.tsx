import { useCallback, useEffect, useState } from "react";
import { clearRequestHistory, listRequests } from "../api";
import { useVisibleInterval } from "../hooks/useVisibleInterval";
import { useI18n } from "../i18n";
import type { ConnectionView } from "../types";

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
          className="search"
          placeholder={t("req.filter")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <button type="button" className="secondary" onClick={() => void reload()}>
          {t("common.refresh")}
        </button>
        <button type="button" className="danger" onClick={() => void onClear()}>
          {t("common.clear")}
        </button>
      </div>
    </div>
  );

  const body = (
    <>
      {error && <div className="banner error">{error}</div>}

      <div className="muted mono traffic-meta">
        {t("req.count", { n: rows.length })}
        {query.trim() ? t("req.filterLabel", { q: query.trim() }) : ""}
      </div>

      {loading ? (
        <div className="empty">{t("common.loading")}</div>
      ) : rows.length === 0 ? (
        <div className="empty card muted">{t("req.empty")}</div>
      ) : (
        <div className="card table-wrap">
          <table className="conn-table">
            <thead>
              <tr>
                <th>{t("req.time")}</th>
                <th>{t("conn.dest")}</th>
                <th>{t("conn.node")}</th>
                <th>{t("conn.chain")}</th>
                <th>{t("conn.rule")}</th>
                <th>{t("conn.process")}</th>
                <th>{t("req.type")}</th>
                <th>{t("conn.traffic")}</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={`${r.id}-${r.last_seen ?? 0}`}>
                  <td className="conn-time">
                    <div>{fmtTime(r.closed_at ?? r.last_seen)}</div>
                    {r.first_seen &&
                    r.first_seen !== (r.closed_at ?? r.last_seen) ? (
                      <div className="muted conn-sub">
                        {t("req.first", { t: fmtTime(r.first_seen) })}
                      </div>
                    ) : null}
                    <div className="muted conn-sub">
                      {r.closed ? t("req.closed") : t("req.live")}
                    </div>
                  </td>
                  <td>
                    <div className="conn-dest" title={r.destination}>
                      {r.destination}
                    </div>
                    <div className="muted conn-sub">{r.host || r.source}</div>
                  </td>
                  <td>
                    <strong title={r.node_tag}>
                      {r.node_name || r.node_tag || "—"}
                    </strong>
                  </td>
                  <td className="conn-chains" title={r.chains_display}>
                    {r.chains_display || "—"}
                  </td>
                  <td className="conn-rule" title={r.rule}>
                    {r.rule || "—"}
                  </td>
                  <td>{r.process || "—"}</td>
                  <td>
                    <code>
                      {r.network}
                      {r.conn_type ? `/${r.conn_type}` : ""}
                    </code>
                  </td>
                  <td className="conn-traffic">
                    ↑{fmtBytes(r.upload)}
                    <br />↓{fmtBytes(r.download)}
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
