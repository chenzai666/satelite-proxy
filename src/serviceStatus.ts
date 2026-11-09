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
export type ServiceStatus = "reachable" | "redirect" | "restricted" | "http_error" | "timeout" | "network_error"
  | "available" | "unavailable" | "blocked" | "disallowed_isp" | "unsupported_region" | "originals_only" | "coming_soon" | "failed";
export type ServiceCheckKind = "reachability" | "availability";

/** 固定原因只映射到本地文案，禁止展示响应正文或任意诊断文本。 */
export const SERVICE_REASON_MESSAGES = {
  response_too_large: "serviceStatus.reasonResponseTooLarge",
  unsupported_encoding: "serviceStatus.reasonUnsupportedEncoding",
  invalid_response: "serviceStatus.reasonInvalidResponse",
  unrecognized_response: "serviceStatus.reasonUnrecognizedResponse",
  expected_denial: "serviceStatus.reasonExpectedDenial",
  challenge: "serviceStatus.reasonChallenge",
  availability_requires_session: "serviceStatus.reasonAvailabilityRequiresSession",
  unsafe_redirect: "serviceStatus.reasonUnsafeRedirect",
  redirect_loop: "serviceStatus.reasonRedirectLoop",
  redirect_limit: "serviceStatus.reasonRedirectLimit",
} as const;
export const SERVICE_REGION_REASON_MESSAGES = {
  not_provided: "serviceStatus.regionReasonNotProvided",
  lookup_timeout: "serviceStatus.regionReasonLookupTimeout",
  invalid_response: "serviceStatus.regionReasonInvalidResponse",
  conflicting_region: "serviceStatus.regionReasonConflictingRegion",
} as const;
export type ServiceStatusReason = keyof typeof SERVICE_REASON_MESSAGES;
export type ServiceRegionReason = keyof typeof SERVICE_REGION_REASON_MESSAGES;

/** 单次服务请求的路线证据；出口只能来自服务回显或匹配出站复测。 */
export interface ServiceRequestRoute {
  role: "primary" | "auxiliary";
  host: string;
  matchStatus: "matched" | "not_observed" | "unsupported";
  chains?: string[];
  rule?: string;
  rulePayload?: string;
  outboundTag?: string;
  exitIp?: string;
  exitSource?: "service_response" | "pinned_outbound_probe";
  exitCached?: boolean;
}

export interface ServiceStatusResult {
  id: ServiceId;
  status: ServiceStatus;
  latencyMs?: number;
  httpStatus?: number;
  /** 服务响应提供的大写两字母地区码；缺失时不推测。 */
  region?: string;
  /** 旧后端可能不提供检测类型。 */
  checkKind?: ServiceCheckKind;
  /** 旧后端可缺失；只接受固定原因码。 */
  reason?: ServiceStatusReason;
  regionReason?: ServiceRegionReason;
  routes?: ServiceRequestRoute[];
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
export const SERVICE_REFRESH_MS = 5 * 60_000;

const serviceIds = new Set<string>(SERVICE_STATUS_CATALOG.map(service => service.id));
const reasons = new Set<string>(Object.keys(SERVICE_REASON_MESSAGES));
const regionReasons = new Set<string>(Object.keys(SERVICE_REGION_REASON_MESSAGES));
const statuses = new Set<ServiceStatus>([
  "reachable", "redirect", "restricted", "http_error", "timeout", "network_error",
  "available", "unavailable", "blocked", "disallowed_isp", "unsupported_region", "originals_only", "coming_soon", "failed",
]);

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

/** 不接受 URL、主机名、端口、CIDR、IPv6 zone 或非标准 IPv4 简写。 */
export function isServiceExitIpLiteral(value: unknown): value is string {
  if (typeof value !== "string" || value.length > 45) return false;
  if (value.includes(":")) {
    if (!/^[0-9a-f:.]+$/i.test(value)) return false;
    try { return new URL(`http://[${value}]/`).hostname.startsWith("["); }
    catch { return false; }
  }
  const parts = value.split(".");
  return parts.length === 4 && parts.every(part => /^(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)$/.test(part));
}

export function primaryServiceRoute(result?: ServiceStatusResult): ServiceRequestRoute | undefined {
  return result?.routes?.find(route => route.role === "primary");
}

function boundedRouteText(value: unknown, max: number): value is string {
  return typeof value === "string" && value.length <= max;
}

function validRoute(value: ServiceRequestRoute): boolean {
  return !!value && typeof value === "object" && !Array.isArray(value)
    && (value.role === "primary" || value.role === "auxiliary")
    && boundedRouteText(value.host, 253) && value.host.length > 0
    && (value.matchStatus === "matched" || value.matchStatus === "not_observed" || value.matchStatus === "unsupported")
    && (value.chains === undefined || (Array.isArray(value.chains) && value.chains.length <= 16
      && value.chains.every(item => boundedRouteText(item, 256))))
    && (value.rule === undefined || boundedRouteText(value.rule, 512))
    && (value.rulePayload === undefined || boundedRouteText(value.rulePayload, 512))
    && (value.outboundTag === undefined || boundedRouteText(value.outboundTag, 256))
    && (value.exitIp === undefined || isServiceExitIpLiteral(value.exitIp))
    && (value.exitSource === undefined || value.exitSource === "service_response" || value.exitSource === "pinned_outbound_probe")
    && (value.exitCached === undefined || typeof value.exitCached === "boolean");
}

function copyRoute(route: ServiceRequestRoute): ServiceRequestRoute {
  return {
    role: route.role, host: route.host, matchStatus: route.matchStatus,
    ...(route.chains === undefined ? {} : { chains: [...route.chains] }),
    ...(route.rule === undefined ? {} : { rule: route.rule }),
    ...(route.rulePayload === undefined ? {} : { rulePayload: route.rulePayload }),
    ...(route.outboundTag === undefined ? {} : { outboundTag: route.outboundTag }),
    ...(route.exitIp === undefined ? {} : { exitIp: route.exitIp }),
    ...(route.exitSource === undefined ? {} : { exitSource: route.exitSource }),
    ...(route.exitCached === undefined ? {} : { exitCached: route.exitCached }),
  };
}

function validResult(value: ServiceStatusResult): boolean {
  return !!value && typeof value === "object" && serviceIds.has(value.id) && statuses.has(value.status)
    && Number.isFinite(value.checkedAt) && value.checkedAt >= 0 && typeof value.viaProxy === "boolean"
    && (value.latencyMs === undefined || (Number.isFinite(value.latencyMs) && value.latencyMs >= 0))
    && (value.httpStatus === undefined || (Number.isInteger(value.httpStatus) && value.httpStatus >= 100 && value.httpStatus <= 599))
    && (value.region === undefined || (typeof value.region === "string" && value.region.length === 2 && /^[A-Z]{2}$/.test(value.region)))
    && (value.checkKind === undefined || value.checkKind === "reachability" || value.checkKind === "availability")
    && (value.reason === undefined || (typeof value.reason === "string" && reasons.has(value.reason)))
    && (value.regionReason === undefined || (typeof value.regionReason === "string" && regionReasons.has(value.regionReason)))
    && (value.routes === undefined || (Array.isArray(value.routes) && value.routes.length <= 8 && value.routes.every(validRoute)));
}

/** 每个组件会话一个实例；所有检测只通过注入的 IPC 调用。 */
export interface ServiceMonitorClock {
  now(): number;
  setTimeout(callback: () => void, ms: number): ReturnType<typeof setTimeout>;
  clearTimeout(timer: ReturnType<typeof setTimeout>): void;
}

export function createServiceStatusMonitor(check: (ids: ServiceId[]) => Promise<ServiceStatusBatch>, clock: ServiceMonitorClock = {
  now: () => Date.now(), setTimeout: (callback, ms) => setTimeout(callback, ms), clearTimeout: timer => clearTimeout(timer),
}) {
  let snapshot: ServiceStatusSnapshot = { results: {}, errors: {}, pending: [] };
  const listeners = new Set<() => void>();
  let epoch = 0;
  let active = false;
  let enabled = true;
  let visible: ServiceId[] = [];
  const queued = new Set<ServiceId>();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let retryStep = 0;
  let fastRetryUsed = false;
  let lastStarted = -Infinity;
  let nextDueAt: number | null = null;

  function clearTimer() {
    if (timer !== undefined) clock.clearTimeout(timer);
    timer = undefined;
  }

  function drain() {
    if (!active || !enabled || snapshot.pending.length || !queued.size) return;
    const ids = [...queued];
    queued.clear();
    void request(ids);
  }

  function enqueue(ids: readonly ServiceId[]) {
    normalizeServiceIds(ids).filter(id => !snapshot.pending.includes(id)).forEach(id => queued.add(id));
    clearTimer();
    drain();
  }

  function schedule(completed = false) {
    if (!active) return;
    if (completed) {
      const ipcFailure = visible.some(id => snapshot.errors[id]);
      const transient = ipcFailure || visible.some(id => ["timeout", "network_error"].includes(snapshot.results[id]?.status ?? ""));
      let delay = SERVICE_REFRESH_MS;
      if (transient) {
        if (ipcFailure && !fastRetryUsed) { delay = 2000; fastRetryUsed = true; }
        else delay = [30000, 60000, 120000, SERVICE_REFRESH_MS][Math.min(retryStep++, 3)];
      } else { retryStep = 0; fastRetryUsed = false; }
      nextDueAt = clock.now() + delay;
    }
    clearTimer();
    if (!enabled || snapshot.pending.length || !visible.length) return;
    if (queued.size) { drain(); return; }
    if (nextDueAt === null || clock.now() >= nextDueAt) { enqueue(visible); return; }
    const token = epoch;
    timer = clock.setTimeout(() => {
      timer = undefined;
      if (active && token === epoch) enqueue(visible);
    }, nextDueAt - clock.now());
  }

  function publish(next: ServiceStatusSnapshot) {
    snapshot = next;
    listeners.forEach(listener => listener());
  }

  async function request(input: readonly ServiceId[]) {
    // 同一实例一次只发一个批次，单项与全量按钮共用同步互斥。
    if (!active || !enabled || snapshot.pending.length) return;
    const ids = normalizeServiceIds(input);
    if (!ids.length) return;
    const token = epoch;
    clearTimer();
    lastStarted = clock.now();
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
          region: result.region, checkKind: result.checkKind,
          reason: result.reason, regionReason: result.regionReason,
          ...(result.routes === undefined ? {} : { routes: result.routes.map(copyRoute) }),
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
    if (active && token === epoch) schedule(true);
  }

  return {
    getSnapshot: () => snapshot,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
    start(visibleIds: readonly ServiceId[]) {
      active = true;
      visible = normalizeServiceIds(visibleIds).slice(0, MAX_VISIBLE_SERVICES);
      retryStep = 0;
      fastRetryUsed = false;
      nextDueAt = null;
      const token = ++epoch;
      // 延至微任务，StrictMode 的首次 setup/cleanup 不会发重复 IPC。
      queueMicrotask(() => {
        if (!active || token !== epoch) return;
        enqueue(visible);
      });
    },
    setVisible(ids: readonly ServiceId[]) {
      const next = normalizeServiceIds(ids).slice(0, MAX_VISIBLE_SERVICES);
      const added = next.filter(id => !visible.includes(id));
      visible = next;
      for (const id of queued) if (!next.includes(id)) queued.delete(id);
      if (active && added.length) enqueue(added);
    },
    setEnabled(value: boolean) {
      if (enabled === value) return;
      enabled = value;
      if (!enabled) { clearTimer(); return; }
      if (active) schedule();
    },
    refreshVisible(networkRecovered = false) {
      if (!active || !enabled || snapshot.pending.length) return;
      if (networkRecovered && clock.now() - lastStarted >= 5000) enqueue(visible);
      else schedule();
    },
    stop() {
      active = false;
      ++epoch;
      clearTimer();
      queued.clear();
      if (snapshot.pending.length) publish({ ...snapshot, pending: [] });
    },
    request,
  };
}
