import { useState } from "react";
import { GlassSeg } from "../components/GlassSeg";
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
        <GlassSeg
          value={tab}
          ariaLabel={t("traffic.title")}
          onChange={(v) => setTab(v as TrafficTab)}
          options={[
            { value: "live", label: t("traffic.tabLive") },
            { value: "history", label: t("traffic.tabHistory") },
          ]}
        />
      </header>

      {/* key={tab} remounts on tab switch → page-enter fade/slide. */}
      <div className="traffic-panel page-enter" role="tabpanel" key={tab}>
        {tab === "live" ? (
          <ConnectionsPage embedded />
        ) : (
          <RequestsPage embedded />
        )}
      </div>
    </div>
  );
}
