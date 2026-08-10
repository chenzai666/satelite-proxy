import { useCallback, useEffect, useMemo, useState } from "react";
import { getProxyStatus, listConnections } from "../api";
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

/** Format a Clash-API ISO start time (e.g. 2024-01-01T00:00:00Z) to local. */
function fmtIso(iso: string) {
  if (!iso) return "—";
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
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
  const [scope, setScope] = useState<TrafficScope>("all");

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
  const scoped = useMemo(() => scopeFilter(rows, scope), [rows, scope]);
  const filtered = q
    ? scoped.filter((r) => {
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
    : scoped;

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
          <h1>{t("conn.title")}</h1>
          <p className="page-desc">{t("conn.desc")}</p>
        </div>
      )}
      <div className="header-actions traffic-toolbar-actions">
        <input
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
          className="search"
          placeholder={t("conn.filter")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <GlassSeg
          value={scope}
          ariaLabel={t("traffic.scopeLabel")}
          onChange={(v) => setScope(v as TrafficScope)}
          options={scopeOpts}
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
                <th className="conn-th-time">{t("conn.time")}</th>
                <th>{t("conn.dest")}</th>
                <th className="conn-th-node">{t("conn.node")}</th>
                <th className="conn-th-rule">{t("conn.rule")}</th>
                <th className="conn-th-process">{t("conn.process")}</th>
                <th className="conn-th-traffic">{t("conn.traffic")}</th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((r) => (
                <tr key={r.id}>
                  <td className="conn-time">
                    <div className="conn-cell" title={fmtIso(r.start)}>
                      {fmtIso(r.start)}
                    </div>
                  </td>
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
