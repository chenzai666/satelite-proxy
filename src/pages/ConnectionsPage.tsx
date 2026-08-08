import { useCallback, useEffect, useState } from "react";
import { getProxyStatus, listConnections } from "../api";
import { useVisibleInterval } from "../hooks/useVisibleInterval";
import { useI18n } from "../i18n";
import type { ConnectionView } from "../types";

function fmtBytes(n: number) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(2)} MB`;
}

interface Props {
  /** When true, omit page chrome (used under Traffic tabs). */
  embedded?: boolean;
}

export function ConnectionsPage({ embedded = false }: Props) {
  const { t } = useI18n();
  const [rows, setRows] = useState<ConnectionView[]>([]);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  const reload = useCallback(async () => {
    try {
      const [status, list] = await Promise.all([
        getProxyStatus().catch(() => null),
        listConnections(),
      ]);
      setRunning(!!status?.running);
      setRows(list);
      setError(null);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // Live list: 1.5s while visible only (history filled by backend journal).
  useVisibleInterval(() => {
    void reload();
  }, 1500);

  const q = query.trim().toLowerCase();
  const filtered = q
    ? rows.filter((r) => {
        const hay = [
          r.destination,
          r.host,
          r.node_name,
          r.node_tag,
          r.chains_display,
          r.rule,
          r.process,
          r.network,
          r.conn_type,
          r.source,
        ]
          .join(" ")
          .toLowerCase();
        return hay.includes(q);
      })
    : rows;

  const toolbar = (
    <div className={`traffic-toolbar ${embedded ? "" : "page-header"}`}>
      {!embedded && (
        <div>
          <h1>{t("conn.title")}</h1>
          <p className="page-desc">{t("conn.desc")}</p>
        </div>
      )}
      <div className="header-actions traffic-toolbar-actions">
        <input
          className="search"
          placeholder={t("conn.filter")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <span className={`pill ${running ? "ok" : "warn"}`}>
          {running
            ? t("conn.active", { n: filtered.length })
            : t("common.coreStopped")}
        </span>
      </div>
    </div>
  );

  const body = (
    <>
      {error && <div className="banner error">{error}</div>}

      {!running ? (
        <div className="empty card muted">{t("conn.needStart")}</div>
      ) : filtered.length === 0 ? (
        <div className="empty card muted">{t("conn.empty")}</div>
      ) : (
        <div className="card table-wrap">
          <table className="conn-table">
            <thead>
              <tr>
                <th>{t("conn.dest")}</th>
                <th>{t("conn.node")}</th>
                <th>{t("conn.net")}</th>
                <th>{t("conn.rule")}</th>
                <th>{t("conn.process")}</th>
                <th>{t("conn.traffic")}</th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((r) => (
                <tr key={r.id}>
                  <td>
                    <div
                      className="conn-cell conn-dest"
                      title={`${r.destination}${r.source ? ` · ${r.source}` : ""}`}
                    >
                      {r.destination}
                    </div>
                  </td>
                  <td>
                    <div
                      className="conn-cell conn-node"
                      title={r.node_tag || r.node_name}
                    >
                      {r.node_name || r.node_tag || "—"}
                    </div>
                  </td>
                  <td>
                    <div className="conn-cell" title={r.network}>
                      <code>{r.network || "—"}</code>
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
                      ↑{fmtBytes(r.upload)} ↓{fmtBytes(r.download)}
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
