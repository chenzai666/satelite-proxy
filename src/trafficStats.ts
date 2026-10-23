import type { ProxyStatus, TrafficBucket } from "./types";

export type TrafficRowId = "proxy" | "direct" | "unclassified" | "total";
export type TrafficValues = Record<keyof TrafficBucket, number | null>;
export interface TrafficStatsRow {
  id: TrafficRowId;
  values: TrafficValues;
}

function validCounter(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : null;
}

function values(bucket: TrafficBucket | null | undefined, running: boolean): TrafficValues {
  const speed = (value: unknown) => {
    const counter = validCounter(value);
    return counter === null ? null : running ? counter : 0;
  };
  return {
    upload_speed: speed(bucket?.upload_speed),
    download_speed: speed(bucket?.download_speed),
    upload_total: validCounter(bucket?.upload_total),
    download_total: validCounter(bucket?.download_total),
  };
}

export function trafficStatsRows(status: ProxyStatus | null | undefined): TrafficStatsRow[] {
  const breakdown = status?.traffic_breakdown;
  const available = breakdown?.source === "outbound" || breakdown?.source === "connections";
  const running = status?.running === true;
  const rows: TrafficStatsRow[] = [
    { id: "proxy", values: values(available ? breakdown.proxy : null, running) },
    { id: "direct", values: values(available ? breakdown.direct : null, running) },
  ];
  // Preserve a nonzero unknown bucket even after stopping zeros its live rates.
  if (available && breakdown.unclassified && Object.values(breakdown.unclassified).some(v => (validCounter(v) ?? 0) > 0)) {
    rows.push({ id: "unclassified", values: values(breakdown.unclassified, running) });
  }
  // Backend buckets include unclassified traffic and reconcile with root totals.
  // The UI displays them verbatim; it never infers or rebalances a category.
  rows.push({ id: "total", values: values(status, running) });
  return rows;
}

export function formatTrafficValue(value: number | null, speed = false): string {
  if (value === null || validCounter(value) === null) return "—";
  const units = ["B", "KB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];
  let amount = value;
  let unit = 0;
  while (amount >= 1024 && unit < units.length - 1) {
    amount /= 1024;
    unit++;
  }
  const number = unit === 0 || amount >= 100 ? Math.round(amount).toString() : amount.toFixed(1);
  return `${number} ${units[unit]}${speed ? "/s" : ""}`;
}
