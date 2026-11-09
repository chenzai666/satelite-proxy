/** 仅当前进程内存：短期复用成功结果并合并相同路线的在途请求。 */
export function createIpLookupCache<T>(ttlMs: number | ((value: T) => number), limit: number, now = () => Date.now()) {
  const entries = new Map<string, { at: number; value: T }>();
  const flights = new Map<string, Promise<T>>();
  const latest = new Map<string, number>();
  let sequence = 0;
  function peek(key: string): T | undefined {
    const entry = entries.get(key);
    if (!entry) return;
    const lifetime = typeof ttlMs === "function" ? ttlMs(entry.value) : ttlMs;
    if (now() - entry.at >= lifetime || now() < entry.at) { entries.delete(key); return; }
    return entry.value;
  }
  function load(key: string, fetch: () => Promise<T>, force = false, scope = key): Promise<T> {
    if (!force) {
      const cached = peek(key);
      if (cached !== undefined) return Promise.resolve(cached);
    }
    const flightKey = JSON.stringify([key, scope]);
    const pending = flights.get(flightKey);
    if (pending) return pending;
    const ticket = ++sequence;
    latest.set(key, ticket);
    const request = Promise.resolve().then(fetch).then(value => {
      if (latest.get(key) === ticket) {
        entries.delete(key);
        entries.set(key, { at: now(), value });
        while (entries.size > limit) entries.delete(entries.keys().next().value!);
      }
      return value;
    }).finally(() => {
      if (flights.get(flightKey) === request) flights.delete(flightKey);
      if (latest.get(key) === ticket) latest.delete(key);
    });
    flights.set(flightKey, request);
    return request;
  }
  return { peek, load };
}
