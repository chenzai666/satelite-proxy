import { useCallback, useEffect, useMemo, useState } from "react";
import { getProxyStatus, listConnections } from "../../api";
import { useVisibleInterval } from "../../hooks/useVisibleInterval";
import type { ConnectionView } from "../../types";

function fmtBytes(n: number) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(2)} MB`;
}

export function SimpleTrafficPage() {
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

  useVisibleInterval(() => {
    void reload();
  }, 1500);

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
    <div className="simple-page simple-traffic">
      <header className="simple-page-head">
        <div>
          <div className="simple-kicker muted">LIVE</div>
          <h1 className="simple-title">流量</h1>
        </div>
        <span className="muted mono" style={{ fontSize: 12 }}>
          {running ? `${filtered.length} 连接` : "未运行"}
        </span>
      </header>

      <input
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
        className="search simple-search"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="过滤域名 / 节点…"
      />

      {error && <div className="banner error">{error}</div>}

      {!running ? (
        <div className="simple-card empty muted">启动代理后显示实时连接</div>
      ) : filtered.length === 0 ? (
        <div className="simple-card empty muted">暂无连接</div>
      ) : (
        <ul className="simple-conn-list">
          {filtered.map((r) => (
            <li key={r.id} className="simple-card simple-conn-item">
              <div className="simple-conn-host" title={r.destination || r.host}>
                {r.host || r.destination || "—"}
              </div>
              <div className="simple-conn-meta muted">
                <span>{r.node_name || r.node_tag || "—"}</span>
                <span className="mono">
                  ↑{fmtBytes(r.upload)} ↓{fmtBytes(r.download)}
                </span>
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
