import { useI18n } from "../i18n";
import { latencyState, type LatencyFailure, type LatencyFields } from "../nodeLatencyState";

const latencyFailureMessages = {
  timeout: "nodes.latencyTimeout",
  unsupported: "nodes.latencyUnsupported",
  config: "nodes.latencyConfig",
  tls: "nodes.latencyTls",
  connection: "nodes.latencyConnection",
  http: "nodes.latencyHttp",
  core: "nodes.latencyCore",
  missing_core: "nodes.latencyMissingCore",
  failed: "nodes.latencyFailed",
} as const satisfies Record<LatencyFailure, string>;

/** 专业列表/卡片、简洁列表共用，错误详情通过文本 title 展示。 */
export function NodeLatencyDisplay({ node, testing, mono = false }: {
  node: LatencyFields;
  testing?: boolean;
  mono?: boolean;
}) {
  const { t } = useI18n();
  if (testing) return <span className="lat lat-spinner" aria-label={t("nodes.testingAria")} />;
  const state = latencyState(node);
  const base = mono ? "lat mono" : "lat";
  if (state.kind === "success") {
    const color = state.ms < 200 ? "lat-good" : state.ms < 300 ? "lat-ok" : "lat-slow";
    return <span className={`${base} ${color}`}>{state.ms}ms</span>;
  }
  if (state.kind === "untested") return <span className={`${base} lat-none`}>—</span>;
  const label = t(latencyFailureMessages[state.kind]);
  return <span className={`${base} lat-timeout`} title={state.detail ?? label}>{label}</span>;
}
