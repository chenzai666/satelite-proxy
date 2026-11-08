import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { AppSettings, SubscriptionView } from "../../src/types";
import "../../src/App.css";
import "../../src/ProAppearance.css";

// 只使用合成数据；不连接后端、不读取用户配置、不启动真实内核。
const storageKey = "config-activation-fixture";
const params = new URLSearchParams(location.search);
const mix = params.get("mix") === "1";
const customBodies: Record<string, string> = {
  "custom-singbox": '{"outbounds":[{"type":"direct","tag":"direct"}]}',
  "custom-mihomo": "mixed-port: 2080\nproxies: []\nrules: [MATCH,DIRECT]\n",
  "custom-xray": '{"outbounds":[{"protocol":"freedom","tag":"direct"}]}',
};
const profile = (id: string, source_kind: string, enabled: boolean): SubscriptionView => ({
  id, name: `Fixture ${id}`, source_kind, enabled,
  source_display: source_kind === "url" ? "https://example.test/sub" : "fixture",
  last_update: 1_700_000_000, node_count: source_kind === "custom" ? 0 : 1,
  skipped_count: 0,
  custom_kind: source_kind === "custom" ? id.slice("custom-".length) : null,
});
const defaults: { settings: AppSettings; profiles: SubscriptionView[] } = {
  settings: {
    mixed_port: 2080, api_port: 19090, probe_url: "https://example.test/204",
    locale: "zh", core_type: "singbox", mix_mode: mix,
    runtime_source: params.get("source") ? `singbox:${params.get("source")}` : "generated",
  },
  profiles: [
    profile("generated-a", "url", true),
    profile("generated-b", "text", mix),
    ...["singbox", "mihomo", "xray"].map(core => profile(`custom-${core}`, "custom", false)),
  ],
};
const saved = JSON.parse(sessionStorage.getItem(storageKey) ?? "null") as typeof defaults | null;
const state = {
  ...(saved ?? defaults),
  commands: [] as { command: string; payload: unknown }[],
  unexpected: [] as string[],
  activations: [] as string[],
  mixWrites: [] as boolean[],
  settingsReads: 0,
  listReads: 0,
  failActivation: null as string | null,
  failAfterSave: null as string | null,
  failSettings: false,
  failList: false,
  holdActivation: false,
  holdSettings: false,
  releaseActivation: null as null | (() => void),
  releaseSettings: null as null | (() => void),
};
function persist() {
  sessionStorage.setItem(storageKey, JSON.stringify({ settings: state.settings, profiles: state.profiles }));
}
function findProfile(id: string) {
  const found = state.profiles.find(item => item.id === id);
  if (!found) throw new Error(`夹具配置不存在：${id}`);
  return found;
}
function activate(id: string) {
  if (state.failActivation) throw state.failActivation;
  const target = findProfile(id);
  if (target.source_kind === "custom") {
    state.settings.runtime_source = `singbox:${id}`;
  } else {
    const fromCustom = state.settings.runtime_source !== "generated";
    state.settings.runtime_source = "generated";
    if (state.settings.mix_mode) {
      // 从自定义模式回到已启用普通卡，仅切运行来源。
      if (!(fromCustom && target.enabled)) target.enabled = !target.enabled;
    } else {
      for (const item of state.profiles) {
        if (item.source_kind !== "custom") item.enabled = item.id === id;
      }
    }
  }
  persist();
  if (state.failAfterSave) throw state.failAfterSave;
  return structuredClone(state.profiles);
}
(window as unknown as { configActivationFixture: typeof state }).configActivationFixture = state;
mockIPC((command, payload) => {
  state.commands.push({ command, payload: structuredClone(payload) });
  switch (command) {
    case "get_settings": {
      state.settingsReads++;
      const read = () => {
        if (state.failSettings) throw "模拟设置刷新失败";
        return structuredClone(state.settings);
      };
      if (!state.holdSettings) return read();
      return new Promise((resolve, reject) => {
        state.releaseSettings = () => {
          state.holdSettings = false;
          state.releaseSettings = null;
          try { resolve(read()); } catch (error) { reject(error); }
        };
      });
    }
    case "list_subscriptions":
      state.listReads++;
      if (state.failList) throw "模拟列表刷新失败";
      return structuredClone(state.profiles);
    case "activate_subscription": {
      const { id } = payload as { id: string };
      if (typeof id !== "string") throw new Error("激活命令缺少 id");
      state.activations.push(id);
      if (!state.holdActivation) return activate(id);
      return new Promise((resolve, reject) => {
        state.releaseActivation = () => {
          state.holdActivation = false;
          state.releaseActivation = null;
          try { resolve(activate(id)); } catch (error) { reject(error); }
        };
      });
    }
    case "set_mix_mode": {
      const { mix } = payload as { mix: boolean };
      if (typeof mix !== "boolean") throw new Error("Mix 命令缺少 mix");
      state.mixWrites.push(mix);
      state.settings.mix_mode = mix;
      if (!mix) {
        let kept = false;
        for (const item of state.profiles) {
          if (item.source_kind === "custom" || !item.enabled) continue;
          item.enabled = !kept;
          kept = true;
        }
      }
      persist();
      return structuredClone(state.settings);
    }
    case "list_subscription_urls":
      return [{ id: "generated-a", name: "Fixture generated-a", url: "https://example.test/sub" }];
    case "get_subscription": {
      const item = findProfile((payload as { id: string }).id);
      return { ...structuredClone(item), content: customBodies[item.id] ?? "", via_proxy: false };
    }
    case "get_subscription_raw_config":
      return customBodies[findProfile((payload as { id: string }).id).id] ?? null;
    case "peek_pending_import_urls":
      return [];
    case "clear_pending_import_urls":
      return null;
    case "plugin:app|version":
      return "1.0.46";
    default:
      state.unexpected.push(command);
      throw new Error(`配置激活夹具未实现命令：${command}`);
  }
}, { shouldMockEvents: true });

// 应用依赖可能在模块求值时注册 Tauri 回调，必须先完成 IPC mock。
const { ConfigPage } = await import("../../src/pages/ConfigPage");
const { LocaleProvider } = await import("../../src/i18n");
const { ImportIntentProvider } = await import("../../src/ImportIntentContext");
createRoot(document.getElementById("root")!).render(
  <LocaleProvider><ImportIntentProvider>
    <div className="app-shell pro-shell"><main className="main"><ConfigPage /></main></div>
  </ImportIntentProvider></LocaleProvider>,
);
