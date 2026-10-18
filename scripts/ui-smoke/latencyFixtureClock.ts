/** 夹具的探测时间必须越过已恢复节点及已发事件，包括被 UI 忽略的事件。 */
export function createLatencyFixtureClock(
  nodes: ReadonlyArray<{ latency_at?: number | null }>,
  nowSeconds = () => Date.now() / 1000,
) {
  let last = 0;
  return {
    observe(at?: number | null) { last = Math.max(last, at ?? 0); },
    next() {
      last = Math.max(last, nowSeconds(), ...nodes.map(node => node.latency_at ?? 0)) + 1;
      return last;
    },
  };
}
