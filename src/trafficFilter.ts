import type { ConnectionView } from "./types";

/**
 * Traffic filter scope: which outbound a row took.
 *
 * Classification is based on the Clash `chains` array (the outbound path the
 * connection actually used): a connection whose chains include the `direct`
 * outbound went out directly; everything else (chains like
 * `["node-…", "proxy"]`) went through a proxy node. `block` rows are dropped by
 * the kernel and rarely appear here, but when they do they count as neither —
 * "proxy" (the default scope) will simply not show them, which is fine.
 */
export type TrafficScope = "all" | "direct" | "proxy";

/** True when the connection used the `direct` outbound (bypassed the proxy). */
export function isDirectRow(r: ConnectionView): boolean {
  return r.chains.some((c) => c.toLowerCase() === "direct");
}

/** True when the connection went through a real proxy node. */
export function isProxyRow(r: ConnectionView): boolean {
  return !isDirectRow(r);
}

export function scopeFilter(rows: ConnectionView[], scope: TrafficScope) {
  if (scope === "all") return rows;
  const keep = scope === "direct" ? isDirectRow : isProxyRow;
  return rows.filter(keep);
}
