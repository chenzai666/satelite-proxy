import { useEffect, useState, type FormEvent } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { readImportFile } from "../api";
import { GlassButton } from "./GlassButton";
import { GlassSeg } from "./GlassSeg";
import { GlassSwitchControl } from "./GlassSwitchControl";
import {
  emptyNodeDraft,
  nodeDraftReady,
  NodeDraftFields,
} from "./NodeDraftFields";
import type {
  AddSourceKind,
  ConfigInputMode,
  LocalKind,
  ManualNodeDraft,
  ProfileKind,
} from "../types";
import { canonicalSubscriptionUrl } from "../subscriptionUrl";

export interface ConfigFormValues {
  name: string;
  kind: AddSourceKind;
  url?: string;
  path?: string;
  content?: string;
  uri?: string;
  node?: ManualNodeDraft;
  viaProxy?: boolean;
  autoUpdate?: boolean;
  autoUpdateIntervalMin?: number;
}

type AutoUpdateInterval = "disabled" | "1h" | "12h" | "24h";

const AUTO_UPDATE_MINUTES: Record<Exclude<AutoUpdateInterval, "disabled">, number> = {
  "1h": 60,
  "12h": 720,
  "24h": 1440,
};

function kindToProfile(kind: AddSourceKind): ProfileKind {
  return kind === "url" ? "subscription" : "local";
}

function kindToLocal(kind: AddSourceKind, hasManualForm?: boolean): LocalKind {
  if (kind === "singbox") return "singbox";
  if (kind === "node" && hasManualForm) return "node";
  if (kind === "node") return "multi";
  return "multi";
}

interface Props {
  open: boolean;
  busy: boolean;
  error: string | null;
  initial?: ConfigFormValues | null;
  isEdit?: boolean;
  title?: string;
  submitLabel?: string;
  existingUrls?: string[];
  onClose: () => void;
  onSubmit: (payload: ConfigFormValues) => void;
}

export function AddConfigModal({
  open: isOpen,
  busy,
  error,
  initial = null,
  isEdit = false,
  title,
  submitLabel,
  existingUrls = [],
  onClose,
  onSubmit,
}: Props) {
  const [profile, setProfile] = useState<ProfileKind>("subscription");
  const [localKind, setLocalKind] = useState<LocalKind>("node");
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [content, setContent] = useState("");
  const [configMode, setConfigMode] = useState<ConfigInputMode>("paste");
  const [node, setNode] = useState<ManualNodeDraft>(() => emptyNodeDraft());
  const [viaProxy, setViaProxy] = useState(false);
  const [autoUpdateInterval, setAutoUpdateInterval] =
    useState<AutoUpdateInterval>("24h");
  const [fileLabel, setFileLabel] = useState("");
  const [fileError, setFileError] = useState<string | null>(null);

  useEffect(() => {
    if (!isOpen) return;
    if (initial) {
      setProfile(kindToProfile(initial.kind));
      setLocalKind(
        kindToLocal(initial.kind, !!(initial.node && initial.node.server)),
      );
      setName(initial.name);
      setUrl(initial.url ?? "");
      setContent(initial.content ?? initial.uri ?? "");
      setNode(initial.node ?? emptyNodeDraft());
      setConfigMode("paste");
      setViaProxy(!!initial.viaProxy);
      setFileLabel("");
      setFileError(null);
      const interval = initial.autoUpdateIntervalMin ?? 1440;
      setAutoUpdateInterval(
        initial.autoUpdate === false
          ? "disabled"
          : interval === 60
            ? "1h"
            : interval === 720
              ? "12h"
              : "24h",
      );
    } else {
      setProfile("subscription");
      setLocalKind("node");
      setName("");
      setUrl("");
      setContent("");
      setNode(emptyNodeDraft());
      setConfigMode("paste");
      setViaProxy(false);
      setAutoUpdateInterval("24h");
      setFileLabel("");
      setFileError(null);
    }
  }, [isOpen, initial]);

  if (!isOpen) return null;

  async function pickFile() {
    setFileError(null);
    const selected = await open({
      multiple: false,
      filters: [
        {
          name: "Config",
          extensions: ["json", "yaml", "yml", "txt", "conf"],
        },
        { name: "All", extensions: ["*"] },
      ],
    });
    if (typeof selected !== "string") return;
    try {
      const text = await readImportFile(selected);
      setContent(text);
      setFileLabel(selected);
      setConfigMode("paste");
    } catch (e) {
      setFileError(typeof e === "string" ? e : String(e));
    }
  }

  function currentKind(): AddSourceKind {
    if (profile === "subscription") return "url";
    if (localKind === "node") return "node";
    if (localKind === "singbox") return "singbox";
    return "text";
  }

  function handleSubmit(e: FormEvent) {
    e.preventDefault();
    const autoUpdate = autoUpdateInterval !== "disabled";
    const interval = autoUpdate
      ? AUTO_UPDATE_MINUTES[autoUpdateInterval]
      : 1440;
    const kind = currentKind();
    const payload: ConfigFormValues = {
      name: name.trim(),
      kind,
      viaProxy: kind === "url" ? viaProxy : false,
      autoUpdate: kind === "url" ? autoUpdate : false,
      autoUpdateIntervalMin: interval,
    };
    if (kind === "url") payload.url = url.trim();
    if (kind === "text" || kind === "singbox") payload.content = content.trim();
    if (kind === "node") {
      payload.node = {
        ...node,
        name: name.trim() || node.name || undefined,
      };
    }
    onSubmit(payload);
  }

  const kind = currentKind();
  const canSubmit =
    !busy &&
    ((kind === "url" && url.trim().length > 0) ||
      ((kind === "text" || kind === "singbox") && content.trim().length > 0) ||
      (kind === "node" && name.trim().length > 0 && nodeDraftReady(node)));
  const normalizedUrl = url.trim();
  const canonicalUrl = canonicalSubscriptionUrl(normalizedUrl);
  const duplicateUrl =
    kind === "url" &&
    normalizedUrl.length > 0 &&
    existingUrls.some(
      (existingUrl) =>
        canonicalUrl != null &&
        canonicalSubscriptionUrl(existingUrl) === canonicalUrl,
    );

  return (
    <div className="modal-backdrop">
      <div
        className="modal config-add-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="config-modal-title"
      >
        <header className="modal-header">
          <h2 id="config-modal-title">
            {title ?? (isEdit ? "编辑配置" : "添加配置")}
          </h2>
          <button
            type="button"
            className="icon-btn"
            onClick={onClose}
            disabled={busy}
            aria-label="关闭"
          >
            ×
          </button>
        </header>

        <form className="modal-body" onSubmit={handleSubmit}>
          <div className="field">
            <span>类型</span>
            <GlassSeg
              value={profile}
              ariaLabel="配置类型"
              disabled={busy}
              onChange={(v) => setProfile(v as ProfileKind)}
              options={[
                { value: "subscription", label: "订阅" },
                { value: "local", label: "本地配置" },
              ]}
            />
          </div>

          {profile === "local" && (
            <div className="field">
              <span>本地类型</span>
              <GlassSeg
                value={localKind}
                ariaLabel="本地配置类型"
                disabled={busy}
                onChange={(v) => setLocalKind(v as LocalKind)}
                options={[
                  { value: "node", label: "手动填写" },
                  { value: "multi", label: "链接解析" },
                  { value: "singbox", label: "sing-box" },
                ]}
              />
            </div>
          )}

          <label className="field">
            <span>名称</span>
            <input
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={
                profile === "subscription"
                  ? "例如：机场 A"
                  : localKind === "node"
                    ? "必填，例如：家宽备用"
                    : localKind === "singbox"
                      ? "例如：自用完整配置"
                      : "例如：自建节点组 / 协议链接"
              }
              disabled={busy}
            />
          </label>

          {profile === "subscription" && (
            <>
              <label className="field">
                <span>订阅链接</span>
                <input
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  placeholder="https://…"
                  disabled={busy}
                  autoFocus
                />
                {duplicateUrl && (
                  <span className="field-warning" role="status">
                    订阅已存在，保存会覆盖已有配置
                  </span>
                )}
              </label>
              <div className="via-proxy-row">
                <div>
                  <div className="sys-proxy-title">走代理添加</div>
                  <div className="sys-proxy-desc">
                    经本地 mixed 端口拉取（需先启动代理核心）
                  </div>
                </div>
                <GlassSwitchControl
                  checked={viaProxy}
                  title="走代理添加"
                  disabled={busy}
                  onChange={setViaProxy}
                />
              </div>
              <div className="field">
                <span>自动更新</span>
                <GlassSeg
                  value={autoUpdateInterval}
                  ariaLabel="自动更新间隔"
                  disabled={busy}
                  onChange={(value) =>
                    setAutoUpdateInterval(value as AutoUpdateInterval)
                  }
                  options={[
                    { value: "disabled", label: "禁用" },
                    { value: "1h", label: "1 小时" },
                    { value: "12h", label: "12 小时" },
                    { value: "24h", label: "24 小时" },
                  ]}
                />
              </div>
            </>
          )}

          {profile === "local" && localKind === "node" && (
            <NodeDraftFields
              value={node}
              disabled={busy}
              onChange={setNode}
            />
          )}

          {profile === "local" &&
            (localKind === "multi" || localKind === "singbox") && (
              <>
                <div className="field">
                  <span>输入方式</span>
                  <GlassSeg
                    value={configMode}
                    ariaLabel="配置输入方式"
                    disabled={busy}
                    onChange={(v) => setConfigMode(v as ConfigInputMode)}
                    options={[
                      { value: "paste", label: "粘贴" },
                      { value: "file", label: "本地文件" },
                    ]}
                  />
                </div>
                {configMode === "file" && (
                  <div className="field">
                    <span>从文件拷贝</span>
                    <div className="file-row">
                      <input
                        readOnly
                        value={fileLabel}
                        placeholder="选择文件后会拷贝进应用，不记录原路径"
                        disabled={busy}
                      />
                      <button
                        type="button"
                        className="secondary"
                        onClick={() => void pickFile()}
                        disabled={busy}
                      >
                        浏览…
                      </button>
                    </div>
                  </div>
                )}
                <label className="field">
                  <span>
                    {localKind === "singbox"
                      ? "完整 sing-box JSON"
                      : "配置内容"}
                  </span>
                  <textarea
                    className="config-paste"
                    autoCapitalize="off"
                    autoCorrect="off"
                    spellCheck={false}
                    value={content}
                    onChange={(e) => setContent(e.target.value)}
                    placeholder={
                      localKind === "singbox"
                        ? "必须是含 inbounds + outbounds 的完整 sing-box JSON，导入后只读，不走应用生成配置"
                        : "一行一个协议链接（vless://…），也可粘贴 Clash / sing-box 订阅内容以提取节点"
                    }
                    disabled={busy}
                    rows={localKind === "singbox" ? 12 : 8}
                  />
                </label>
              </>
            )}

          <p className="hint">
            {profile === "subscription"
              ? isEdit
                ? "保存时会重新拉取并解析节点（保留配置 id）。"
                : "提交后将下载订阅并解析节点。"
              : localKind === "node"
                ? "按协议填写字段，添加一条手动节点。协议链接请用「链接解析」。"
                : localKind === "singbox"
                  ? "只接受完整 sing-box 配置。不会提取节点，也不会参与应用生成配置；后续可在首页用这份配置直接启动。"
                  : "支持单行或多行协议链接，也能从 Clash / sing-box 订阅里提取节点。本地文件会拷贝进应用。"}
          </p>

          {(error || fileError) && (
            <div className="form-error">{error || fileError}</div>
          )}

          <footer className="modal-footer">
            <GlassButton onClick={onClose} disabled={busy}>
              取消
            </GlassButton>
            <GlassButton type="submit" variant="primary" disabled={!canSubmit}>
              {busy
                ? isEdit
                  ? "保存中…"
                  : "导入中…"
                : (submitLabel ?? (isEdit ? "保存" : "添加"))}
            </GlassButton>
          </footer>
        </form>
      </div>
    </div>
  );
}
