import { useCallback, useEffect, useRef, useState } from "react";
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

/** Xray log line level → the shared log chip classes. */
function coreLogLevel(line: string): "error" | "warn" | "info" | "debug" {
  if (/\[Error\]/i.test(line)) return "error";
  if (/\[Warning\]/i.test(line)) return "warn";
  if (/\[Debug\]/i.test(line)) return "debug";
  return "info";
}

/**
 * Live tail of the active core's log — the Xray-mode stand-in for connection
 * monitoring. Xray has no per-connection API, but at `info` level its log
 * carries accepted connections and routing decisions, so this gives the
 * traffic page real visibility.
 */
function CoreLogPanel() {
  const { t } = useI18n();
  const [lines, setLines] = useState<string[]>([]);
  const [path, setPath] = useState<string | null>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const listRef = useRef<HTMLDivElement>(null);

  const reload = useCallback(async () => {
    try {
      const tail = await getCoreLogTail(400);
      setLines(tail.lines);
      setPath(tail.path);
    } catch {
      /* transient — keep the last view */
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  useVisibleInterval(reload, 1500);

  useEffect(() => {
    if (autoScroll && listRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
    }
  }, [lines, autoScroll]);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "0.55rem",
        flex: "1 1 auto",
        minHeight: 0,
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          gap: "0.8rem",
        }}
      >
        <div
          className="settings-app-title"
          style={{ fontSize: "0.85rem", fontWeight: 600 }}
        >
          {t("traffic.xrayLogTitle")}
          <span
            className="muted mono"
            style={{ marginLeft: "0.6rem", fontSize: 11, fontWeight: 400 }}
            title={path ?? undefined}
          >
            {lines.length}
          </span>
        </div>
        <GlassSwitch
          checked={autoScroll}
          onChange={setAutoScroll}
          label={t("logs.autoScroll")}
          title={t("logs.autoScroll")}
          capsule
          size="sm"
        />
      </div>
      <div
        className="logs-panel card glass"
        ref={listRef}
        style={{ flex: "1 1 auto", minHeight: 260 }}
      >
        {lines.length === 0 ? (
          <p className="muted logs-empty">{t("traffic.xrayLogEmpty")}</p>
        ) : (
          <ul className="logs-list mono" style={{ listStyle: "none" }}>
            {lines.map((line, i) => {
              const level = coreLogLevel(line);
              return (
                <li
                  key={i}
                  style={{
                    display: "flex",
                    alignItems: "baseline",
                    gap: "0.5rem",
                    padding: "0.18rem 0.25rem",
                    borderBottom:
                      "1px solid color-mix(in srgb, var(--border, #333) 55%, transparent)",
                    whiteSpace: "pre-wrap",
                    wordBreak: "break-word",
                    fontSize: "0.74rem",
                    lineHeight: 1.45,
                  }}
                  title={line}
                >
                  <span className={`log-lvl log-lvl-${level}`}>
                    {level.toUpperCase()}
                  </span>
                  <span>{line}</span>
                </li>
              );
            })}
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

  // Xray has no per-connection API — the tabs are connection data, so the
  // page swaps in a live core-log view instead.
  const xrayCore = coreType === "xray";

  return (
    <div className="page traffic-page">
      <header className="page-header traffic-header">
        <div>
          <h1>{t("traffic.title")}</h1>
          <p className="page-desc">{t("traffic.desc")}</p>
        </div>
        {/* All three tabs are connection data — meaningless under Xray, so the
            switcher hides and the log view takes the whole panel. */}
        {!xrayCore && (
          <GlassSeg
            value={tab}
            ariaLabel={t("traffic.title")}
            onChange={(v) => setTab(v as TrafficTab)}
            options={[
              { value: "live", label: t("traffic.tabLive") },
              { value: "history", label: t("traffic.tabHistory") },
              { value: "failures", label: t("traffic.tabFailures") },
            ]}
          />
        )}
      </header>

      {/* key={tab} remounts on tab switch → page-enter fade/slide. */}
      <div className="traffic-panel page-enter" role="tabpanel" key={tab}>
        {xrayCore ? (
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              gap: "0.6rem",
              flex: "1 1 auto",
              minHeight: 0,
            }}
          >
            <div className="traffic-xray-notice">
              <div className="traffic-xray-mark" aria-hidden>
                ⌁
              </div>
              <div>
                <div className="traffic-xray-title">
                  {t("traffic.xrayTitle")}
                </div>
                <p className="muted">{t("traffic.xrayLogDesc")}</p>
              </div>
            </div>
            <CoreLogPanel />
          </div>
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
