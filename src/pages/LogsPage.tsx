import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { clearAppLogs, listAppLogs, type AppLogEntry, type AppLogLevel } from "../api";
import { GlassButton } from "../components/GlassButton";
import { GlassSeg } from "../components/GlassSeg";
import { GlassSwitch } from "../components/GlassSwitch";
import { useVisibleInterval } from "../hooks/useVisibleInterval";
import { useI18n } from "../i18n";

const LEVELS: AppLogLevel[] = ["error", "warn", "info", "debug", "trace"];

function levelRank(l: AppLogLevel): number {
  switch (l) {
    case "trace":
      return 0;
    case "debug":
      return 1;
    case "info":
      return 2;
    case "warn":
      return 3;
    case "error":
      return 4;
  }
}

function fmtTs(ms: number) {
  try {
    const d = new Date(ms);
    return d.toLocaleTimeString(undefined, {
      hour12: false,
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      fractionalSecondDigits: 3,
    } as Intl.DateTimeFormatOptions);
  } catch {
    return String(ms);
  }
}

export function LogsPage() {
  const { t } = useI18n();
  const [minLevel, setMinLevel] = useState<AppLogLevel>("info");
  const [query, setQuery] = useState("");
  const [rows, setRows] = useState<AppLogEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const listRef = useRef<HTMLDivElement>(null);

  const reload = useCallback(async () => {
    try {
      const list = await listAppLogs({
        minLevel,
        limit: 800,
        query: query.trim() || null,
      });
      setRows(list);
      setError(null);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }, [minLevel, query]);

  useEffect(() => {
    void reload();
  }, [reload]);

  useVisibleInterval(() => {
    void reload();
  }, 1200);

  useEffect(() => {
    if (!autoScroll || !listRef.current) return;
    listRef.current.scrollTop = listRef.current.scrollHeight;
  }, [rows, autoScroll]);

  async function onClear() {
    try {
      await clearAppLogs();
      setRows([]);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }

  const countLabel = useMemo(() => `${rows.length}`, [rows.length]);

  return (
    <div className="page logs-page">
      <div className="page-header traffic-header">
        <div>
          <h1>{t("logs.title")}</h1>
          <p className="page-desc">{t("logs.desc")}</p>
        </div>
        <div className="header-actions traffic-toolbar-actions">
          <span className="muted mono" style={{ fontSize: 12 }}>
            {countLabel}
          </span>
          <GlassSwitch
            checked={autoScroll}
            onChange={setAutoScroll}
            label={t("logs.autoScroll")}
            title={t("logs.autoScroll")}
            capsule
            size="sm"
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

      <div className="logs-toolbar">
        <GlassSeg
          value={minLevel}
          ariaLabel={t("logs.level")}
          onChange={(v) => setMinLevel(v as AppLogLevel)}
          titles={Object.fromEntries(
            LEVELS.map((lv) => [lv, `${t("logs.minLevel")}: ${lv}`]),
          )}
          options={LEVELS.map((lv) => ({ value: lv, label: lv }))}
        />
        <input
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
          className="search"
          placeholder={t("logs.filter")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>

      {error && <p className="error-banner">{error}</p>}

      <div className="logs-panel card glass" ref={listRef}>
        {rows.length === 0 ? (
          <p className="muted logs-empty">{t("logs.empty")}</p>
        ) : (
          <ul className="logs-list mono">
            {rows.map((e) => (
              <li
                key={e.id}
                className={`log-line log-${e.level}`}
                data-level={e.level}
                style={{
                  opacity: levelRank(e.level) < levelRank(minLevel) ? 0.5 : 1,
                }}
              >
                <span className="log-ts">{fmtTs(e.ts_ms)}</span>
                <span className={`log-lvl log-lvl-${e.level}`}>{e.level}</span>
                <span className="log-target">{e.target}</span>
                <span className="log-msg">{e.message}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
