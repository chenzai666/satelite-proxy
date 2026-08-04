import { useState } from "react";
import { useI18n } from "../i18n";
import { ConnectionsPage } from "./ConnectionsPage";
import { RequestsPage } from "./RequestsPage";

type TrafficTab = "live" | "history";

export function TrafficPage() {
  const { t } = useI18n();
  const [tab, setTab] = useState<TrafficTab>("live");

  return (
    <div className="page traffic-page">
      <header className="page-header traffic-header">
        <div>
          <h1>{t("traffic.title")}</h1>
          <p className="page-desc">{t("traffic.desc")}</p>
        </div>
        <div
          className="segmented compact traffic-tabs"
          role="tablist"
          aria-label={t("traffic.title")}
        >
          <button
            type="button"
            role="tab"
            aria-selected={tab === "live"}
            className={`seg ${tab === "live" ? "active" : ""}`}
            onClick={() => setTab("live")}
          >
            {t("traffic.tabLive")}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "history"}
            className={`seg ${tab === "history" ? "active" : ""}`}
            onClick={() => setTab("history")}
          >
            {t("traffic.tabHistory")}
          </button>
        </div>
      </header>

      <div className="traffic-panel" role="tabpanel">
        {tab === "live" ? (
          <ConnectionsPage embedded />
        ) : (
          <RequestsPage embedded />
        )}
      </div>
    </div>
  );
}
