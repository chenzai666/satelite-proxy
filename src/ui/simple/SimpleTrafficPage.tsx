import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getProxyStatus, listConnectionChanges } from "../../api";
import { useVisibleInterval } from "../../hooks/useVisibleInterval";
import { useI18n } from "../../i18n";
import type { ConnectionView } from "../../types";
import { applyConnectionChanges } from "../../connectionChanges";

function fmtBytes(n: number) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(2)} MB`;
}

export function SimpleTrafficPage() {
  const { t } = useI18n();
  const [rows, setRows] = useState<ConnectionView[]>([]);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const revisionRef = useRef<number | null>(null);
  const orderRevRef = useRef<number | null>(null);

  const reload = useCallback(async () => {
    try {
      const [status, batch] = await Promise.all([
        getProxyStatus().catch(() => null),
        listConnectionChanges(revisionRef.current, orderRevRef.current),
      ]);
      setRunning(!!status?.running);
      revisionRef.current = batch.revision;
      orderRevRef.current = batch.order_revision;
      if (!batch.unchanged) setRows((current) => applyConnectionChanges(current, batch));
      setError(null);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  useVisibleInterval(() => reload(), 1500);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return rows;
    return rows.filter((r) => {
      const hay = [r.destination, r.host, r.node_name, r.process, r.network]
        .join(" ")
        .toLowerCase();
      return hay.includes(q);
    });
  }, [rows, query]);

  return (
    <div className="page simple-page simple-traffic">
      <header className="page-header">
        <div>
          <h1>{t("traffic.title")}</h1>
          <p className="page-desc">
            {t("conn.desc")}
            {" · "}
            <span className="mono">
              {running
                ? t("conn.active", { n: filtered.length })
                : t("simple.notRunning")}
            </span>
          </p>
        </div>
      </header>

      <input
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
        className="search simple-search"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder={t("conn.filter")}
      />

      {error && <div className="banner error">{error}</div>}

      {!running ? (
        <div className="empty card muted">{t("conn.needStart")}</div>
      ) : filtered.length === 0 ? (
        <div className="empty card muted">{t("conn.empty")}</div>
      ) : (
        <ul className="simple-conn-list">
          {filtered.map((r) => (
            <li key={r.id} className="simple-conn-item">
              <div className="simple-conn-host" title={r.destination || r.host}>
                {r.host || r.destination || "—"}
              </div>
              <div className="simple-conn-meta muted">
                <span>{r.node_name || r.node_tag || "—"}</span>
                <span className="mono">
                  <span className="tr-dir up">↑</span>
                  {fmtBytes(r.upload)}{" "}
                  <span className="tr-dir down">↓</span>
                  {fmtBytes(r.download)}
                </span>
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
