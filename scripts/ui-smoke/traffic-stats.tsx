import { useState } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import { TrafficStatsCard } from "../../src/components/TrafficStatsCard";
import { LocaleProvider } from "../../src/i18n";
import type { ProxyStatus, TrafficBucket } from "../../src/types";
import "../../src/App.css";

const params = new URLSearchParams(location.search);
const locale = params.get("locale") === "en" ? "en" : "zh";
const theme = params.get("theme") === "day" ? "day" : "aerospace";
const zoom = [1, 1.5, 2].find(value => String(value) === params.get("zoom")) ?? 1;
const width = [240, 280, 320, 420, 960, 1280].find(value => String(value) === params.get("width")) ?? 320;
const grid = params.get("layout") === "grid";
document.documentElement.dataset.theme = theme;
document.documentElement.lang = locale === "zh" ? "zh-CN" : "en";
document.documentElement.style.zoom = String(zoom);
document.body.style.margin = "0";

// 夹具仅使用内存数据；任何非读取语言设置的 IPC 都立即失败。
mockIPC(command => {
  if (command === "get_settings") return { locale };
  throw new Error(`流量夹具禁止 IPC：${command}`);
});

const keys = ["upload_speed", "download_speed", "upload_total", "download_total"] as const;
const bucket = (up: number, down: number, upload: number, download: number): TrafficBucket => ({
  upload_speed: up, download_speed: down, upload_total: upload, download_total: download,
});
const zero = () => bucket(0, 0, 0, 0);
const base: ProxyStatus = {
  running: true, core_state: "running", system_proxy: false, tun_enabled: false,
  outbound_mode: "rule", mixed_port: 2080, api_port: 9090, connections: 3,
  ...zero(),
};

function classified(source: "connections" | "outbound", unclassified = bucket(512, 1024, 4096, 8192)): ProxyStatus {
  const proxy = bucket(2048, 8192, 32768, 65536);
  const direct = bucket(1024, 4096, 16384, 32768);
  // 每个方向的速度与累计均显式对账，不用缺口反推代理。
  const total = Object.fromEntries(keys.map(key => [key, proxy[key] + direct[key] + unclassified[key]])) as unknown as TrafficBucket;
  return { ...base, ...total, traffic_breakdown: { source, proxy, direct, unclassified } };
}

const connection = classified("connections");
const stopped: ProxyStatus = { ...connection, running: false, core_state: "stopped" };
// 故意保留停止快照中的陈旧速度，验证卡片主动归零。
const hugeBucket = bucket(123.4 * 1024 ** 3, 999.4 * 1024 ** 3, 123.4 * 1024 ** 5, 999.4 * 1024 ** 5);
const huge = classified("connections", hugeBucket);
const scenarios: Record<string, ProxyStatus | null> = {
  missing: null,
  legacy: { ...base, ...bucket(512, 1024, 4096, 8192) },
  unavailable: { ...connection, traffic_breakdown: { source: "unavailable", proxy: zero(), direct: zero(), unclassified: zero() } },
  connections: connection,
  outbound: classified("outbound"),
  three: classified("connections", zero()),
  stopped,
  reset: { ...base, running: false, core_state: "stopped", traffic_breakdown: { source: "connections", proxy: zero(), direct: zero(), unclassified: zero() } },
  huge,
};

function Fixture() {
  const [status, setStatus] = useState<ProxyStatus | null>(connection);
  const [opens, setOpens] = useState(0);
  (window as unknown as { trafficFixture: unknown }).trafficFixture = {
    scenarios,
    status,
    opens,
    setScenario(name: string) {
      if (!(name in scenarios)) throw new Error(`未知流量场景：${name}`);
      setStatus(scenarios[name]);
    },
  };
  const card = <TrafficStatsCard status={status} onOpen={() => setOpens(value => value + 1)} />;
  return (
    <main style={{ width, padding: grid ? 16 : 0, background: "var(--page-glow)" }}>
      {grid ? <div className="instrument-grid instrument-grid-6" data-fixture-grid>
        <article className="instrument" data-neighbor="core"><header className="instrument-head"><span className="instrument-label">核心状态</span></header><div className="instrument-value">Running</div></article>
        <article className="instrument" data-neighbor="spark"><header className="instrument-head"><span className="instrument-label">实时图表</span></header><div className="instrument-value">↓ 8 KB/s</div></article>
        {card}
        {["quality", "subscription", "system"].map(name => <article className="instrument" key={name} data-neighbor={name}><header className="instrument-head"><span className="instrument-label">相邻仪表 · {name}</span></header><div className="instrument-value">123.4</div><div className="instrument-kv">保留原三列宽度与最小高度</div></article>)}
      </div> : card}
    </main>
  );
}

createRoot(document.getElementById("root")!).render(<LocaleProvider><Fixture /></LocaleProvider>);
