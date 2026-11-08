/** 目录只包含固定 ID 与品牌名称；检测目标和网络请求由后端维护。 */
export const SERVICE_STATUS_CATALOG = [
  { id: "google", title: "Google" },
  { id: "github", title: "GitHub" },
  { id: "youtube", title: "YouTube" },
  { id: "chatgpt", title: "ChatGPT" },
  { id: "claude", title: "Claude" },
  { id: "gemini", title: "Gemini" },
  { id: "netflix", title: "Netflix" },
  { id: "disney", title: "Disney+" },
  { id: "prime", title: "Prime Video" },
  { id: "spotify", title: "Spotify" },
  { id: "tiktok", title: "TikTok" },
  { id: "discord", title: "Discord" },
  { id: "bilibili", title: "哔哩哔哩 bilibili" },
] as const;

export type ServiceId = (typeof SERVICE_STATUS_CATALOG)[number]["id"];
export type ServiceStatus = "reachable" | "redirect" | "restricted" | "http_error" | "timeout" | "network_error";

export interface ServiceStatusResult {
  id: ServiceId;
  status: ServiceStatus;
  latencyMs?: number;
  httpStatus?: number;
  /** Unix 毫秒。 */
  checkedAt: number;
  /** 经过内核入口，最终出站仍遵循当前内核规则。 */
  viaProxy: boolean;
}

export interface ServiceStatusBatch {
  results: ServiceStatusResult[];
  checkedAt: number;
  viaProxy: boolean;
}

export const SERVICE_STATUS_STORAGE_KEY = "satelite.serviceStatus.visibleIds";
export const DEFAULT_SERVICE_IDS: readonly ServiceId[] = ["google", "github"];
export const MAX_VISIBLE_SERVICES = 4;

const serviceIds = new Set<string>(SERVICE_STATUS_CATALOG.map(service => service.id));
const statuses = new Set<ServiceStatus>(["reachable", "redirect", "restricted", "http_error", "timeout", "network_error"]);

export function normalizeServiceIds(value: unknown): ServiceId[] {
  if (!Array.isArray(value)) return [];
  return [...new Set(value.filter((id): id is ServiceId => typeof id === "string" && serviceIds.has(id)))];
}

export function readVisibleServiceIds(raw: string | null): ServiceId[] {
  try {
    const ids = normalizeServiceIds(raw && raw.length <= 4096 ? JSON.parse(raw) : null).slice(0, MAX_VISIBLE_SERVICES);
    return ids.length ? ids : [...DEFAULT_SERVICE_IDS];
  } catch {
    return [...DEFAULT_SERVICE_IDS];
  }
}

export interface ServiceStatusSnapshot {
  results: Partial<Record<ServiceId, ServiceStatusResult>>;
  errors: Partial<Record<ServiceId, boolean>>;
  pending: readonly ServiceId[];
}

function validResult(value: ServiceStatusResult): boolean {
  return !!value && typeof value === "object" && serviceIds.has(value.id) && statuses.has(value.status)
    && Number.isFinite(value.checkedAt) && value.checkedAt >= 0 && typeof value.viaProxy === "boolean"
    && (value.latencyMs === undefined || (Number.isFinite(value.latencyMs) && value.latencyMs >= 0))
    && (value.httpStatus === undefined || (Number.isInteger(value.httpStatus) && value.httpStatus >= 100 && value.httpStatus <= 599));
}

/** 每个组件会话一个实例；所有检测只通过注入的 IPC 调用。 */
export function createServiceStatusMonitor(check: (ids: ServiceId[]) => Promise<ServiceStatusBatch>) {
  let snapshot: ServiceStatusSnapshot = { results: {}, errors: {}, pending: [] };
  const listeners = new Set<() => void>();
  let epoch = 0;
  let active = false;
  let autoStarted = false;

  function publish(next: ServiceStatusSnapshot) {
    snapshot = next;
    listeners.forEach(listener => listener());
  }

  async function request(input: readonly ServiceId[]) {
    // 同一实例一次只发一个批次，单项与全量按钮共用同步互斥。
    if (!active || snapshot.pending.length) return;
    const ids = normalizeServiceIds(input);
    if (!ids.length) return;
    const token = epoch;
    const errors = { ...snapshot.errors };
    ids.forEach(id => delete errors[id]);
    publish({ ...snapshot, errors, pending: ids });
    try {
      const batch = await check(ids);
      if (!active || token !== epoch) return;
      if (!batch || !Array.isArray(batch.results) || !Number.isFinite(batch.checkedAt)
        || batch.checkedAt < 0 || typeof batch.viaProxy !== "boolean") throw new Error("invalid batch");
      const nextResults = { ...snapshot.results };
      const nextErrors = { ...snapshot.errors };
      for (const id of ids) {
        const matches = batch.results.filter(result => result?.id === id);
        const result = matches[0];
        if (matches.length !== 1 || !validResult(result)) {
          nextErrors[id] = true;
          continue;
        }
        // 仅复制契约字段，不把额外诊断数据带到 UI 或持久存储。
        nextResults[id] = {
          id, status: result.status, latencyMs: result.latencyMs, httpStatus: result.httpStatus,
          checkedAt: result.checkedAt, viaProxy: result.viaProxy,
        };
        delete nextErrors[id];
      }
      publish({ results: nextResults, errors: nextErrors, pending: [] });
    } catch {
      if (!active || token !== epoch) return;
      const nextErrors = { ...snapshot.errors };
      ids.forEach(id => { nextErrors[id] = true; });
      publish({ ...snapshot, errors: nextErrors, pending: [] });
    }
  }

  return {
    getSnapshot: () => snapshot,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
    start(visibleIds: readonly ServiceId[]) {
      active = true;
      const token = ++epoch;
      // 延至微任务，StrictMode 的首次 setup/cleanup 不会发重复 IPC。
      queueMicrotask(() => {
        if (!active || token !== epoch || autoStarted) return;
        autoStarted = true;
        void request(visibleIds.slice(0, 2));
      });
    },
    stop() {
      active = false;
      ++epoch;
      if (snapshot.pending.length) publish({ ...snapshot, pending: [] });
    },
    request,
  };
}
