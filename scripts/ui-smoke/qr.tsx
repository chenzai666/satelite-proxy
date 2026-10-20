import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import QRCode from "qrcode";
import { ConfigPage } from "../../src/pages/ConfigPage";
import { NodeDraftFields, emptyNodeDraft } from "../../src/components/NodeDraftFields";
import { ImportIntentProvider } from "../../src/ImportIntentContext";
import { LocaleProvider } from "../../src/i18n";
import "../../src/App.css";

const state = { capture: 0, read: 0, writes: [] as unknown[], submits: 0, dialog: 0,
  image: "", cancel: false, fail: false, hold: false, release: null as null | (() => void),
  async setQr(text: string) { state.image = await QRCode.toDataURL(text, { width: 600 }); },
};
(window as unknown as { qrFixture: typeof state }).qrFixture = state;
mockIPC((command, payload) => {
  if (command === "get_settings") return { locale: "zh", runtime_source: "generated", mix_mode: false };
  if (command === "list_subscriptions" || command === "list_subscription_urls" || command === "peek_pending_import_urls") return [];
  if (command === "capture_qr_screen") {
    state.capture++;
    if (state.fail) throw "模拟屏幕捕获失败";
    if (state.hold) return new Promise<string>(resolve => { state.release = () => { state.release = null; resolve(state.image); }; });
    return state.image;
  }
  if (command === "plugin:dialog|open") { state.dialog++; return state.cancel ? null : "test-only-qr.png"; }
  if (command === "read_qr_image") { state.read++; return state.image; }
  if (command.startsWith("add_subscription_")) {
    state.writes.push({ command, payload });
    return { subscription: { name: "fixture" }, node_count: 1, skipped_count: 0 };
  }
  throw new Error(`二维码夹具未实现命令：${command}`);
}, { shouldMockEvents: true });
function Draft() {
  const [node, setNode] = useState(emptyNodeDraft());
  const [disabled, setDisabled] = useState(false);
  return <form onSubmit={event => { event.preventDefault(); state.submits++; }}>
    <button type="button" onClick={() => setDisabled(!disabled)}>toggle-disabled</button>
    <NodeDraftFields value={node} onChange={setNode} disabled={disabled} />
  </form>;
}
createRoot(document.getElementById("root")!).render(<LocaleProvider><ImportIntentProvider>
  <main className="main">{location.search.includes("draft") ? <Draft /> : <ConfigPage />}</main>
</ImportIntentProvider></LocaleProvider>);
