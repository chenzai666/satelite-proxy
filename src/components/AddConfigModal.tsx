import { useEffect, useState, type FormEvent } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { GlassButton } from "./GlassButton";
import { GlassSeg } from "./GlassSeg";
import { GlassSwitchControl } from "./GlassSwitchControl";
import type { AddSourceKind } from "../types";

export interface ConfigFormValues {
  name: string;
  kind: AddSourceKind;
  url?: string;
  path?: string;
  /** Fetch URL via local mixed proxy (core must be running). */
  viaProxy?: boolean;
  /** Periodically refresh this profile. */
  autoUpdate?: boolean;
  /** Minutes between auto updates (default 1440). */
  autoUpdateIntervalMin?: number;
}

type AutoUpdateInterval = "disabled" | "1h" | "12h" | "24h";

const AUTO_UPDATE_MINUTES: Record<Exclude<AutoUpdateInterval, "disabled">, number> = {
  "1h": 60,
  "12h": 720,
  "24h": 1440,
};

interface Props {
  open: boolean;
  busy: boolean;
  error: string | null;
  /**
   * Prefill form fields. Used for edit and for one-click subscribe (add).
   * Does not imply edit mode — set `isEdit` for that.
   */
  initial?: ConfigFormValues | null;
  /** When true, UI treats form as editing an existing profile. */
  isEdit?: boolean;
  title?: string;
  submitLabel?: string;
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
  onClose,
  onSubmit,
}: Props) {
  const [kind, setKind] = useState<AddSourceKind>("url");
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [path, setPath] = useState("");
  const [viaProxy, setViaProxy] = useState(false);
  const [autoUpdateInterval, setAutoUpdateInterval] =
    useState<AutoUpdateInterval>("24h");

  useEffect(() => {
    if (!isOpen) return;
    if (initial) {
      setKind(initial.kind);
      setName(initial.name);
      setUrl(initial.url ?? "");
      setPath(initial.path ?? "");
      setViaProxy(!!initial.viaProxy);
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
      setKind("url");
      setName("");
      setUrl("");
      setPath("");
      setViaProxy(false);
      setAutoUpdateInterval("24h");
    }
  }, [isOpen, initial]);

  if (!isOpen) return null;

  async function pickFile() {
    const selected = await open({
      multiple: false,
      filters: [
        { name: "Subscription", extensions: ["yaml", "yml", "txt", "conf"] },
        { name: "All", extensions: ["*"] },
      ],
    });
    if (typeof selected === "string") {
      setPath(selected);
    }
  }

  function handleSubmit(e: FormEvent) {
    e.preventDefault();
    const autoUpdate = autoUpdateInterval !== "disabled";
    const interval = autoUpdate
      ? AUTO_UPDATE_MINUTES[autoUpdateInterval]
      : 1440;
    if (kind === "url") {
      onSubmit({
        name: name.trim(),
        kind,
        url: url.trim(),
        viaProxy,
        autoUpdate,
        autoUpdateIntervalMin: interval,
      });
    } else {
      onSubmit({
        name: name.trim(),
        kind,
        path: path.trim(),
        viaProxy: false,
        autoUpdate,
        autoUpdateIntervalMin: interval,
      });
    }
  }

  const canSubmit =
    !busy &&
    ((kind === "url" && url.trim().length > 0) ||
      (kind === "file" && path.trim().length > 0));

  return (
    <div className="modal-backdrop" onClick={() => !busy && onClose()}>
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="config-modal-title"
        onClick={(e) => e.stopPropagation()}
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
          <label className="field">
            <span>名称</span>
            <input
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="例如：机场 A"
              disabled={busy}
            />
          </label>

          <div className="field">
            <span>来源</span>
            <GlassSeg
              value={kind}
              ariaLabel="来源"
              disabled={busy}
              onChange={(v) => setKind(v as ConfigFormValues["kind"])}
              options={[
                { value: "url", label: "订阅 URL" },
                { value: "file", label: "本地文件" },
              ]}
            />
          </div>

          {kind === "url" ? (
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
            </>
          ) : (
            <div className="field">
              <span>配置文件</span>
              <div className="file-row">
                <input
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                  value={path}
                  onChange={(e) => setPath(e.target.value)}
                  placeholder="选择 Clash YAML / URI 列表文件"
                  disabled={busy}
                />
                <button type="button" className="secondary" onClick={pickFile} disabled={busy}>
                  浏览…
                </button>
              </div>
            </div>
          )}

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

          <p className="hint">
            {isEdit
              ? "保存时会重新拉取/读取并解析节点（保留配置 id）。"
              : "提交后将下载或读取文件，解析 Clash / URI 节点并转换为内部配置格式。"}
          </p>

          {error && <div className="form-error">{error}</div>}

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
