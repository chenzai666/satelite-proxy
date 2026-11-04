import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import { NodeDraftFields, emptyNodeDraft } from "../../src/components/NodeDraftFields";
import { EditLocalNodesModal } from "../../src/components/EditLocalNodesModal";
import { NodesPage } from "../../src/pages/NodesPage";
import { SimpleServersPage } from "../../src/ui/simple/SimpleServersPage";
import { getProxyStatus } from "../../src/api";
import type { ManualNodeDraft } from "../../src/types";
import { LocaleProvider } from "../../src/i18n";
import { ThemeProvider } from "../../src/theme";
import "../../src/App.css";

const node = { id: "fixture", name: "Fixture", protocol: "vless", server: "example.test", port: 443, subscription_id: "fixture", subscription_name: "Fixture" };
const state = {
  settings: { locale: "zh", core_type: "singbox", runtime_source: "generated", auto_select: "off", current_node_id: null as string | null, multi_core_enabled: false, protocol_core_defaults: [] as { protocol: string; core: string }[], node_core_overrides: {} as Record<string, string>, ...JSON.parse(sessionStorage.getItem("core-selection-settings") ?? "{}") },
  draft: JSON.parse(sessionStorage.getItem("core-selection-draft") ?? "null") as ManualNodeDraft | null,
  writes: [] as unknown[], commands: [] as string[], unexpected: [] as string[], settingsReads: 0, hold: false, release: null as null | (() => void),
  status: { running: true, core_type: "singbox", runtime_source: "generated", outbound_mode: "rule" },
  async finish(core: string, result = "ready") { state.status.core_type = core; await getProxyStatus(); await emit("config-apply-status", { status: result }); },
};
state.status = { ...state.status, ...JSON.parse(sessionStorage.getItem("core-selection-status") ?? "{}") };
state.draft ??= { ...emptyNodeDraft(), name: "Fixture", server: "example.test", uuid: "fixture-uuid", preferredCore: "auto", network: "xhttp", xhttp_extra: '{"mode":"stream-one"}', sni: "sni.example.test" };
function persist() { sessionStorage.setItem("core-selection-settings", JSON.stringify(state.settings)); }
(window as unknown as { coreFixture: typeof state }).coreFixture = state;
mockIPC((command, payload) => {
  state.commands.push(command);
  if (command === "get_settings") { state.settingsReads++; return structuredClone(state.settings); }
  if (command === "update_settings") {
    state.writes.push(structuredClone(payload));
    const defaults = (payload as { protocolCoreDefaults?: typeof state.settings.protocol_core_defaults }).protocolCoreDefaults;
    const commit = () => {
      if (defaults) state.settings.protocol_core_defaults = structuredClone(defaults);
      persist();
      return structuredClone(state.settings);
    };
    if (state.hold) return new Promise(resolve => { state.release = () => { state.hold = false; state.release = null; resolve(commit()); }; });
    return commit();
  }
  if (command === "get_proxy_status") return structuredClone(state.status);
  if (command === "get_core_info") return { kind: (payload as { kind: string }).kind, version: null };
  if (command === "plugin:app|version") return "1.0.46";
  if (command === "get_app_install_path") return "fixture";
  if (command === "check_app_update") return { current_version: "1.0.46", latest_version: null, update_available: false };
  if (command === "list_subscription_nodes") return [node];
  if (command === "list_all_nodes" || command === "list_custom_config_nodes") return [node];
  if (command === "list_nodes_page") return { nodes: [node], total: 1 };
  if (command === "get_node_draft") return { ...structuredClone(state.draft), preferredCore: state.settings.node_core_overrides.fixture ?? "auto" };
  if (command === "set_current_node") {
    if ((payload as { nodeId?: string }).nodeId !== node.id) throw new Error("set_current_node 缺少 camelCase nodeId");
    state.settings.current_node_id = node.id;
    state.settings.core_type = "mihomo";
    persist();
    return structuredClone(state.settings);
  }
  if (command === "generate_singbox_config") return {};
  if (command === "list_mihomo_proxy_groups") return [];
  if (command === "get_mihomo_template") return { group_order: [], template: { url: "", ini: "", override_yaml: "" }, use_for_active: false };
  if (command === "update_node") {
    const draft = (payload as { draft: typeof state.draft }).draft;
    state.draft = structuredClone(draft);
    sessionStorage.setItem("core-selection-draft", JSON.stringify(draft));
    if (draft?.preferredCore === "auto") delete state.settings.node_core_overrides.fixture;
    else if (draft?.preferredCore) state.settings.node_core_overrides.fixture = draft.preferredCore;
    persist();
    state.writes.push(structuredClone(payload));
    return node;
  }
  state.unexpected.push(command);
  throw new Error(`fixture: ${command}`);
}, { shouldMockEvents: true });
// SettingsPage 的下载进度模块在加载时注册事件，必须先安装模拟回调接口。
const { SettingsPage } = await import("../../src/pages/SettingsPage");
function Draft() {
  const [draft, setDraft] = useState(emptyNodeDraft());
  return <NodeDraftFields value={draft} onChange={setDraft} />;
}
const mode = new URLSearchParams(location.search).get("mode");
createRoot(document.getElementById("root")!).render(<LocaleProvider><ThemeProvider><main className="main">
  {mode === "edit" ? <EditLocalNodesModal open profileId="fixture" profileName="Fixture" onClose={() => {}} /> : mode === "draft" ? <Draft /> : mode === "pro" ? <NodesPage /> : mode === "simple" ? <SimpleServersPage /> : <SettingsPage />}
</main></ThemeProvider></LocaleProvider>);
