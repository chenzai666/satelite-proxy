import { useEffect, useState } from "react";
import { GlassSeg } from "../components/GlassSeg";
import { useI18n } from "../i18n";
import { getProxyStatus, peekProxyStatus } from "../api";
import type { ProxyStatus } from "../types";
import { ConnectionsPage } from "./ConnectionsPage";
import { FailuresPage } from "./FailuresPage";
import { RequestsPage } from "./RequestsPage";

type TrafficTab = "live" | "history" | "failures";

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

  // Xray has no per-connection API — all three tabs are connection data.
  const xrayCore = coreType === "xray";

  return (
    <div className="page traffic-page">
      <header className="page-header traffic-header">
        <div>
          <h1>{t("traffic.title")}</h1>
          <p className="page-desc">{t("traffic.desc")}</p>
        </div>
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
      </header>

      {/* key={tab} remounts on tab switch → page-enter fade/slide. */}
      <div className="traffic-panel page-enter" role="tabpanel" key={tab}>
        {xrayCore ? (
          <div className="traffic-xray-notice">
            <div className="traffic-xray-mark" aria-hidden>
              ⌁
            </div>
            <div>
              <div className="traffic-xray-title">
                {t("traffic.xrayTitle")}
              </div>
              <p className="muted">{t("traffic.xrayDesc")}</p>
            </div>
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
