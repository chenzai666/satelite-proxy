import { useI18n } from "../i18n";
import { formatTrafficValue } from "../trafficStats";
import "./DashboardRateCard.css";

interface Props {
  direction: "up" | "down";
  speed: number;
  total: number;
  samples: number[];
  running: boolean;
  onOpen?: () => void;
}

function safeValue(value: number): number {
  return Number.isFinite(value) && value >= 0 ? value : 0;
}

function rateParts(value: number) {
  const text = formatTrafficValue(value, true);
  const split = text.lastIndexOf(" ");
  return { text, number: text.slice(0, split), unit: text.slice(split + 1) };
}

export function DashboardRateCard({ direction, speed, total, samples, running, onOpen }: Props) {
  const { t } = useI18n();
  const label = t(direction === "up" ? "dashboard.upload" : "dashboard.download");
  // Keep the supplied sampling order and window; invalid values occupy a zero slot.
  const recent = samples.slice(-60).map(safeValue);
  const peak = Math.max(0, ...recent);
  const scale = peak || 1;
  const points = recent.map((value, index) => ({
    x: recent.length > 1 ? 6 + (index / (recent.length - 1)) * 288 : 294,
    y: 58 - (value / scale) * 52,
  }));
  const line = points.length
    ? points.map(({ x, y }, index) => `${index ? "L" : "M"}${x.toFixed(2)} ${y.toFixed(2)}`).join(" ")
    : "M6 58 L294 58";
  const area = points.length > 1 ? `${line} L294 58 L6 58 Z` : undefined;
  const rate = rateParts(running ? safeValue(speed) : 0);
  const last = points[points.length - 1];
  const heading = <><span className="dashboard-rate-card__arrow" aria-hidden="true">{direction === "up" ? "↑" : "↓"}</span>{label}</>;

  return (
    <article className="dashboard-rate-card" data-direction={direction} data-running={running} aria-label={label}>
      <header className="dashboard-rate-card__head">
        {onOpen ? (
          <button className="dashboard-rate-card__title dashboard-rate-card__open" type="button" onClick={onOpen}
            aria-label={t("dashboard.rateOpen", { direction: label })}>
            {heading}<span className="dashboard-rate-card__link" aria-hidden="true">↗</span>
          </button>
        ) : <h3 className="dashboard-rate-card__title">{heading}</h3>}
      </header>
      <div className="dashboard-rate-card__rate" title={rate.text}>
        <span className="dashboard-rate-card__number">{rate.number}</span>
        <span className="dashboard-rate-card__unit">{rate.unit}</span>
      </div>
      <dl className="dashboard-rate-card__stats">
        <div>
          <dt>{t("dashboard.trafficTotal")}</dt>
          <dd>{formatTrafficValue(safeValue(total))}</dd>
        </div>
        <div title={t("dashboard.ratePeakHint")}>
          <dt>{t("dashboard.ratePeak")}</dt>
          <dd>{formatTrafficValue(peak, true)}</dd>
        </div>
      </dl>
      <svg className="dashboard-rate-card__chart" viewBox="0 0 300 64" preserveAspectRatio="none"
        role="img" aria-label={t("dashboard.rateHistory")}>
        <title>{t("dashboard.rateHistory")}</title>
        <path className="dashboard-rate-card__baseline" d="M6 58 H294" />
        {area && <path className="dashboard-rate-card__area" d={area} />}
        <path className="dashboard-rate-card__line" d={line} vectorEffect="non-scaling-stroke" />
        {last && <circle cx={last.x} cy={last.y} r="2.5" fill="currentColor" />}
      </svg>
    </article>
  );
}
