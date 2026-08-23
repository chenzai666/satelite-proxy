import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { GlassSeg } from "../components/GlassSeg";
import { GlassSwitch } from "../components/GlassSwitch";
import { useI18n } from "../i18n";
import { getCoreLogTail, getProxyStatus, peekProxyStatus } from "../api";
import type { ProxyStatus } from "../types";
import { useVisibleInterval } from "../hooks/useVisibleInterval";
import { ConnectionsPage } from "./ConnectionsPage";
import { FailuresPage } from "./FailuresPage";
import { RequestsPage } from "./RequestsPage";

type TrafficTab = "live" | "history" | "failures";

type CoreLogLevel = "error" | "warn" | "info" | "debug";
/** Minimum-level options for the core log (no trace level in core output). */
const CORE_LEVELS: CoreLogLevel[] = ["debug", "info", "warn", "error"];

function coreLevelRank(l: CoreLogLevel): number {
  switch (l) {
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

/** Xray log line level → the shared log chip classes. */
function coreLogLevel(line: string): CoreLogLevel {
  if (/\[Error\]/i.test(line)) return "error";
  if (/\[Warning\]/i.test(line)) return "warn";
  if (/\[Debug\]/i.test(line)) return "debug";
  return "info";
}

/**
 * Core log tab — the Xray-mode stand-in for connection monitoring. Xray has
 * no per-connection API, but at `info` level its log carries accepted
 * connections and routing decisions. Reuses the Logs page UI (level seg +
 * search + .logs-panel list); filtering is client-side over the tail.
 */
function CoreLogTab() {
  const { t } = useI18n();
  const [lines, setLines] = useState<string[]>([]);
  const [minLevel, setMinLevel] = useState<CoreLogLevel>("info");
  const [query, setQuery] = useState("");
  const [autoScroll, setAutoScroll] = useState(true);
  const listRef = useRef<HTMLDivElement>(null);

  const reload = useCallback(async () => {
    try {
      const tail = await getCoreLogTail(400);
      setLines(tail.lines);
    } catch {
      /* transient — keep the last view */
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  useVisibleInterval(reload, 1500);

  useEffect(() => {
    // Newest line is at the top — "follow" means pinning to the top whenever
    // fresh lines arrive (prepending at scrollTop 0 needs no adjustment, this
    // pulls back readers who scrolled down).
    if (autoScroll && listRef.current) {
      listRef.current.scrollTop = 0;
    }
  }, [lines, autoScroll]);

  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    // The tail arrives oldest→newest; flip so fresh lines append at the top.
    return lines
      .slice()
      .reverse()
      .map((line, i) => ({ id: i, level: coreLogLevel(line), msg: line }))
      .filter((r) => coreLevelRank(r.level) >= coreLevelRank(minLevel))
      .filter((r) => !q || r.msg.toLowerCase().includes(q));
  }, [lines, minLevel, query]);

  return (
    <div className="core-log-tab">
      <div className="logs-toolbar">
        <GlassSeg
          value={minLevel}
          ariaLabel={t("logs.level")}
          onChange={(v) => setMinLevel(v as CoreLogLevel)}
          titles={Object.fromEntries(
            CORE_LEVELS.map((lv) => [lv, `${t("logs.minLevel")}: ${lv}`]),
          )}
          options={CORE_LEVELS.map((lv) => ({ value: lv, label: lv }))}
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
        <span className="muted mono core-log-count">{rows.length}</span>
        <GlassSwitch
          checked={autoScroll}
          onChange={setAutoScroll}
          label={t("logs.autoScroll")}
          title={t("logs.autoScroll")}
          capsule
          size="sm"
        />
      </div>
      <div className="logs-panel card glass" ref={listRef}>
        {rows.length === 0 ? (
          <p className="muted logs-empty">{t("traffic.xrayLogEmpty")}</p>
        ) : (
          <ul className="logs-list mono">
            {rows.map((r) => (
              <li
                key={r.id}
                className={`log-line core-log-line log-${r.level}`}
                data-level={r.level}
                title={r.msg}
              >
                <span className={`log-lvl log-lvl-${r.level}`}>{r.level}</span>
                <span className="log-msg">{r.msg}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

export function TrafficPage() {
  const { t } = useI18n();
  const [tab, setTab] = useState<TrafficTab>("live");
  const [coreType, setCoreType] = useState<string | null>(
    () => peekProxyStatus()?.core_type ?? null,
  );
  // Core-log view is opt-in (default off) and remembered across remounts —
  // pages remount on every nav switch (key={nav}).
  const [showLog, setShowLog] = useState<boolean>(
    () => localStorage.getItem("traffic.xrayLog") === "1",
  );

  function toggleLog(next: boolean) {
    setShowLog(next);
    try {
      localStorage.setItem("traffic.xrayLog", next ? "1" : "0");
    } catch {
      /* private mode etc. — session-only fallback */
    }
  }

  useEffect(() => {
    // Seed from the module snapshot for an instant first paint, then refresh.
    setCoreType(peekProxyStatus()?.core_type ?? null);
    let disposed = false;
    void getProxyStatus()
      .then((status: ProxyStatus) => {
        if (!disposed) setCoreType(status.core_type ?? "singbox");
      })
      .catch(() => {});
    return () => {
      disposed = true;
    };
  }, []);

  // Xray has no per-connection API — the page becomes a dedicated core-log
  // view (title swapped, tab switcher replaced by an opt-in toggle).
  const xrayCore = coreType === "xray";

  const tabOptions = [
    { value: "live", label: t("traffic.tabLive") },
    { value: "history", label: t("traffic.tabHistory") },
    { value: "failures", label: t("traffic.tabFailures") },
  ];

  return (
    <div className="page traffic-page">
      <header className="page-header traffic-header">
        <div>
          <h1>{xrayCore ? t("traffic.xrayLogTitle") : t("traffic.title")}</h1>
          <p className="page-desc">
            {xrayCore ? t("traffic.xrayLogDesc") : t("traffic.desc")}
          </p>
        </div>
        {xrayCore ? (
          <GlassSwitch
            checked={showLog}
            onChange={toggleLog}
            label={t("traffic.xrayLogToggle")}
            title={t("traffic.xrayLogToggle")}
            capsule
            size="sm"
          />
        ) : (
          <GlassSeg
            value={tab}
            ariaLabel={t("traffic.title")}
            onChange={(v) => setTab(v as TrafficTab)}
            options={tabOptions}
          />
        )}
      </header>

      {/* key remounts on tab/log switch → page-enter fade/slide. */}
      <div
        className="traffic-panel page-enter"
        role="tabpanel"
        key={xrayCore ? (showLog ? "corelog" : "corelog-off") : tab}
      >
        {xrayCore ? (
          showLog ? (
            <CoreLogTab />
          ) : (
            <div className="logs-panel card glass">
              <p className="muted logs-empty">{t("traffic.xrayLogOff")}</p>
            </div>
          )
        ) : tab === "live" ? (
          <ConnectionsPage embedded />
        ) : tab === "history" ? (
          <RequestsPage embedded />
        ) : (
          <FailuresPage embedded />
        )}
      </div>
    </div>
  );
}
