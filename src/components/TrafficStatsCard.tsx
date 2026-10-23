import { useI18n, type MessageKey } from "../i18n";
import type { ProxyStatus } from "../types";
import { formatTrafficValue, trafficStatsRows, type TrafficRowId } from "../trafficStats";

interface Props {
  status?: ProxyStatus | null;
  onOpen?: () => void;
}

const rowLabels: Record<TrafficRowId, MessageKey> = {
  proxy: "traffic.scopeProxy",
  direct: "traffic.scopeDirect",
  unclassified: "dashboard.trafficUnclassified",
  total: "dashboard.trafficCombined",
};

export function TrafficStatsCard({ status, onOpen }: Props) {
  const { t } = useI18n();
  const source = status?.traffic_breakdown?.source;
  const sourceKey = source === "connections" ? "Connections" : source === "outbound" ? "Outbound" : "Unavailable";
  const hint = t(`dashboard.traffic${sourceKey}Hint`);

  function metric(value: number | null, direction: "down" | "up", speed: boolean) {
    const label = t(direction === "down" ? "dashboard.download" : "dashboard.upload");
    const text = formatTrafficValue(value, speed);
    return (
      <div className="traffic-stats-metric" data-direction={direction} aria-label={`${label}: ${text}`}>
        <span className={`tr-dir ${direction}`} aria-hidden="true">{direction === "down" ? "↓" : "↑"}</span>
        <span className="traffic-stats-number">{text}</span>
      </div>
    );
  }

  return (
    <article
      className={`instrument accent-blue traffic-stats-card${onOpen ? " instrument-click" : ""}`}
      aria-label={t("dashboard.cardTrafficStats")}
      onClick={onOpen}
    >
      <header className="instrument-head">
        {onOpen ? (
          <a className="instrument-label traffic-stats-open" href="#traffic" onClick={event => {
            event.preventDefault();
            event.stopPropagation();
            onOpen();
          }}>{t("dashboard.cardTrafficStats")} ↗</a>
        ) : <span className="instrument-label">{t("dashboard.cardTrafficStats")}</span>}
        <span className="traffic-stats-source" tabIndex={0} title={hint} aria-label={hint} onClick={event => event.stopPropagation()}>
          {t(`dashboard.traffic${sourceKey}`)}
        </span>
      </header>
      <table className="traffic-stats-table" aria-label={t("dashboard.cardTrafficStats")}>
        <colgroup><col className="traffic-stats-category" /><col /><col /></colgroup>
        <thead><tr>
          <th scope="col">{t("traffic.scopeLabel")}</th>
          <th scope="col">{t("dashboard.trafficLive")}</th>
          <th scope="col">{t("dashboard.trafficSession")}</th>
        </tr></thead>
        <tbody>{trafficStatsRows(status).map(({ id, values }) => (
          <tr key={id} data-traffic-row={id}>
            <th scope="row">{t(rowLabels[id])}</th>
            <td data-period="live">
              {metric(values.download_speed, "down", true)}
              {metric(values.upload_speed, "up", true)}
            </td>
            <td data-period="session">
              {metric(values.download_total, "down", false)}
              {metric(values.upload_total, "up", false)}
            </td>
          </tr>
        ))}</tbody>
      </table>
    </article>
  );
}
