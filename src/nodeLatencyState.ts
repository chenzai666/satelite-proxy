import type { LatencyResult, NodeLatencyChange, ProxyNode } from "./types";

export type LatencyFields = Pick<ProxyNode,
  "latency_ms" | "latency_at" | "latency_method" | "latency_error">;

export type LatencyFailure = "timeout" | "unsupported" | "config" | "tls" |
  "connection" | "http" | "core" | "missing_core" | "failed";

/** 只展示约定的后端安全错误；未知旧格式使用通用失败提示。 */
export function latencyState(node: LatencyFields):
  | { kind: "success"; ms: number }
  | { kind: "untested" }
  | { kind: LatencyFailure; detail?: string } {
  if (node.latency_ms != null && Number.isFinite(node.latency_ms) && node.latency_ms >= 0) {
    return { kind: "success", ms: node.latency_ms };
  }
  const error = node.latency_error;
  const match = error?.match(/^(timeout|unsupported|config|tls|connection|http|core|missing_core):/);
  if (match) return { kind: match[1] as LatencyFailure, detail: error! };
  if (error) return { kind: "failed" };
  if (node.latency_method === "unsupported") return { kind: "unsupported" };
  return { kind: node.latency_at != null ? "failed" : "untested" };
}

export function isRealLatencyMethod(method?: string | null): boolean {
  return method === "clash_api" || !!method?.endsWith("_http");
}

/** 真延迟成功和失败均受保护；事件和流式回包共用时间戳及 TCP 降级保护。 */
export function mergeNodeLatency<T extends LatencyFields>(node: T, update: LatencyFields): T {
  if (update.latency_method === "tcp" && isRealLatencyMethod(node.latency_method)) return node;
  if (update.latency_at != null && node.latency_at != null && update.latency_at < node.latency_at) return node;
  return {
    ...node,
    ...update,
    latency_at: update.latency_at ?? node.latency_at,
    latency_method: update.latency_method ?? node.latency_method,
    latency_error: update.latency_ms != null && update.latency_ms >= 0
      ? null : update.latency_error ?? null,
  };
}

export function applyLatencyResult<T extends LatencyFields>(node: T, result: LatencyResult): T {
  return mergeNodeLatency(node, {
    latency_ms: result.latency_ms ?? null,
    latency_at: result.tested_at,
    latency_method: result.method,
    latency_error: result.error,
  });
}

export function applyNodeLatencyChange<T extends LatencyFields>(node: T, change: NodeLatencyChange): T {
  return mergeNodeLatency(node, {
    latency_ms: change.latency_ms,
    latency_at: change.latency_at,
    latency_method: change.method,
    latency_error: change.latency_error,
  });
}

/** 保留方法和时间戳用于拒绝降级/迟到回包，重测时由 spinner 遮盖旧值。 */
export function clearLatencyForRetest<T extends LatencyFields>(node: T, kind: "real" | "ping" = "real"): T {
  if (kind === "ping" && isRealLatencyMethod(node.latency_method)) return node;
  return { ...node, latency_ms: undefined, latency_error: null };
}

/** 合并本轮测速的不支持标记，不清除其他分组的结果。 */
export function mergeUnsupportedIds(
  previous: ReadonlySet<string>,
  testedIds: Iterable<string>,
  results: Iterable<{ id: string; method?: string | null }>,
): Set<string> {
  const next = new Set(previous);
  for (const id of testedIds) next.delete(id);
  for (const result of results)
    if (result.method === "unsupported") next.add(result.id);
  return next;
}
