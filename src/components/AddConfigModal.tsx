import { useEffect, useState, type FormEvent } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { readImportFile } from "../api";
import { GlassButton } from "./GlassButton";
import { ErrorModal } from "./ErrorModal";
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
import { useI18n } from "../i18n";

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
  userAgent?: string;
}

type AutoUpdateInterval = "disabled" | "1h" | "12h" | "24h" | "custom";

const AUTO_UPDATE_MINUTES: Record<"1h" | "12h" | "24h", number> = {
  "1h": 60,
  "12h": 720,
  "24h": 1440,
};

/** A subscription scheduler interval is a whole positive u32 minute count. */
function parseCustomMinutes(raw: string): number | null {
  const trimmed = raw.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const minutes = Number.parseInt(trimmed, 10);
  return minutes > 0 && minutes <= 4_294_967_295 ? minutes : null;
}

function kindToProfile(kind: AddSourceKind): ProfileKind {
  if (kind === "url") return "subscription";
  if (kind === "custom") return "custom";
  return "local";
}

function kindToLocal(kind: AddSourceKind, hasManualForm?: boolean): LocalKind {
  if (kind === "node" && hasManualForm) return "node";
  return "multi";
}

/** Client-side mirror of the backend `detect_custom_config_kind` heuristic —
 *  advisory only (shows a detected-type hint under the textarea); the
 *  backend remains authoritative and validates on submit. */
export function detectCustomKind(
  content: string,
): "singbox" | "mihomo" | "xray" | null {
  const trimmed = content.trim();
  if (!trimmed) return null;
  if (trimmed.startsWith("{")) {
    let parsed: unknown;
    try {
      parsed = JSON.parse(trimmed);
    } catch {
      return null;
    }
    if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
      return null;
    }
    const obj = parsed as Record<string, unknown>;
    const entries = [
      ...(Array.isArray(obj.outbounds) ? obj.outbounds : []),
      ...(Array.isArray(obj.inbounds) ? obj.inbounds : []),
    ];
    let hasType = false;
    let hasProtocol = false;
    for (const entry of entries) {
      if (entry !== null && typeof entry === "object") {
        const map = entry as Record<string, unknown>;
        if (typeof map.type === "string") hasType = true;
        if (typeof map.protocol === "string") hasProtocol = true;
      }
    }
    if (hasProtocol && !hasType) return "xray";
    if (hasType && !hasProtocol) return "singbox";
    return null;
  }
  if (
    /^\s*(proxies|proxy-groups|mixed-port|port|socks-port|listeners|rules|external-controller|tun)\s*:/m.test(
      trimmed,
    )
  ) {
    return "mihomo";
  }
  return null;
}

interface Props {
  open: boolean;
  busy: boolean;
  error: string | null;
  /** Clear the passed-in `error` (backends surface import failures here). */
  onDismissError?: () => void;
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
  onDismissError,
  initial = null,
  isEdit = false,
  title,
  submitLabel,
  existingUrls = [],
  onClose,
  onSubmit,
}: Props) {
  const { t } = useI18n();
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
  const [customMinutes, setCustomMinutes] = useState("");
  const [userAgent, setUserAgent] = useState("");
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
      setUserAgent(initial.userAgent ?? "");
      const interval = initial.autoUpdateIntervalMin ?? 1440;
      setCustomMinutes(initial.autoUpdate === false ? "" : String(interval));
      setAutoUpdateInterval(
        initial.autoUpdate === false
          ? "disabled"
          : interval === 60
            ? "1h"
            : interval === 720
              ? "12h"
              : interval === 1440
                ? "24h"
                : "custom",
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
      setCustomMinutes("");
      setFileLabel("");
      setFileError(null);
      setUserAgent("");
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
    if (profile === "custom") return "custom";
    if (localKind === "node") return "node";
    return "text";
  }

  function handleSubmit(e: FormEvent) {
    e.preventDefault();
    const autoUpdate = autoUpdateInterval !== "disabled";
    const interval = autoUpdate
      ? autoUpdateInterval === "custom"
        ? (parseCustomMinutes(customMinutes) ?? 1440)
        : AUTO_UPDATE_MINUTES[autoUpdateInterval]
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
    if (kind === "url" && userAgent.trim()) payload.userAgent = userAgent.trim();
    if (kind === "text" || kind === "custom") payload.content = content.trim();
    if (kind === "node") {
      payload.node = {
        ...node,
        name: name.trim() || node.name || undefined,
      };
    }
    onSubmit(payload);
  }

  const kind = currentKind();
  const customIntervalInvalid =
    kind === "url" &&
    autoUpdateInterval === "custom" &&
    parseCustomMinutes(customMinutes) == null;
  const canSubmit =
    !busy &&
    !customIntervalInvalid &&
    ((kind === "url" && url.trim().length > 0) ||
      ((kind === "text" || kind === "custom") && content.trim().length > 0) ||
      (kind === "node" && name.trim().length > 0 && nodeDraftReady(node)));
  const detectedKind =
    profile === "custom" && content.trim().length > 0
      ? detectCustomKind(content)
      : null;
  const detectedLabel = (k: "singbox" | "mihomo" | "xray") =>
    k === "singbox"
      ? t("config.typeSingbox")
      : k === "mihomo"
        ? t("config.typeMihomo")
        : t("config.typeXray");
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
            {title ?? (isEdit ? t("modal.editTitle") : t("modal.addTitle"))}
          </h2>
          <button
            type="button"
            className="icon-btn"
            onClick={onClose}
            disabled={busy}
            aria-label={t("common.close")}
          >
            ×
          </button>
        </header>

        <form className="modal-body" onSubmit={handleSubmit}>
          <div className="field">
            <span>{t("modal.type")}</span>
            <GlassSeg
              value={
                profile === "local"
                  ? localKind === "node"
                    ? "manual"
                    : "parse"
                  : profile
              }
              ariaLabel={t("modal.typeAria")}
              disabled={busy}
              onChange={(v) => {
                if (v === "subscription" || v === "custom") {
                  setProfile(v);
                } else {
                  setProfile("local");
                  setLocalKind(v === "manual" ? "node" : "multi");
                }
              }}
              options={[
                { value: "subscription", label: t("modal.tabSubscription") },
                { value: "manual", label: t("modal.tabManual") },
                { value: "parse", label: t("modal.tabParse") },
                { value: "custom", label: t("modal.tabCustom") },
              ]}
            />
            <span className="field-hint muted">
              {profile === "subscription"
                ? t("modal.hintSubscription")
                : profile === "custom"
                  ? t("modal.hintCustom")
                  : localKind === "node"
                    ? t("modal.hintManual")
                    : t("modal.hintParse")}
            </span>
          </div>

          <label className="field">
            <span>{t("modal.name")}</span>
            <input
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={
                profile === "subscription"
                  ? t("modal.namePhSubscription")
                  : profile === "custom"
                    ? t("modal.namePhCustom")
                    : localKind === "node"
                      ? t("modal.namePhManual")
                      : t("modal.namePhParse")
              }
              disabled={busy}
            />
          </label>

          {profile === "subscription" && (
            <>
              <label className="field">
                <span>{t("modal.urlLabel")}</span>
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
                    {t("modal.duplicateUrl")}
                  </span>
                )}
              </label>
              <div className="via-proxy-row">
                <div>
                  <div className="sys-proxy-title">{t("modal.viaProxy")}</div>
                  <div className="sys-proxy-desc">
                    {t("modal.viaProxyDesc")}
                  </div>
                </div>
                <GlassSwitchControl
                  checked={viaProxy}
                  title={t("modal.viaProxy")}
                  disabled={busy}
                  onChange={setViaProxy}
                />
              </div>
              <div className="field">
                <span>{t("modal.autoUpdate")}</span>
                <GlassSeg
                  value={autoUpdateInterval}
                  ariaLabel={t("modal.autoUpdateAria")}
                  disabled={busy}
                  onChange={(value) =>
                    setAutoUpdateInterval(value as AutoUpdateInterval)
                  }
                  options={[
                    { value: "disabled", label: t("modal.intervalDisabled") },
                    { value: "1h", label: t("modal.interval1h") },
                    { value: "12h", label: t("modal.interval12h") },
                    { value: "24h", label: t("modal.interval24h") },
                    { value: "custom", label: t("modal.intervalCustom") },
                  ]}
                />
              </div>
              {autoUpdateInterval === "custom" && (
                <label className="field">
                  <span>{t("modal.customInterval")}</span>
                  <input
                    type="number"
                    min={1}
                    step={1}
                    inputMode="numeric"
                    autoCapitalize="off"
                    autoCorrect="off"
                    spellCheck={false}
                    value={customMinutes}
                    onChange={(e) => setCustomMinutes(e.target.value)}
                    placeholder={t("modal.customIntervalPh")}
                    disabled={busy}
                  />
                  {customIntervalInvalid && (
                    <span className="field-warning" role="status">
                      {t("modal.customIntervalInvalid")}
                    </span>
                  )}
                </label>
              )}
              <label className="field">
                <span>{t("modal.userAgent")}</span>
                <input
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                  value={userAgent}
                  onChange={(e) => setUserAgent(e.target.value)}
                  placeholder="satelite-proxy/0.1 clash-verge/v2.5 flclash/1"
                  disabled={busy}
                />
              </label>
            </>
          )}

          {profile === "local" && localKind === "node" && (
            <NodeDraftFields
              value={node}
              disabled={busy}
              onChange={setNode}
            />
          )}

          {(profile === "custom" ||
            (profile === "local" && localKind === "multi")) && (
              <>
                <div className="field">
                  <span>{t("modal.inputMode")}</span>
                  <GlassSeg
                    value={configMode}
                    ariaLabel={t("modal.inputModeAria")}
                    disabled={busy}
                    onChange={(v) => setConfigMode(v as ConfigInputMode)}
                    options={[
                      { value: "paste", label: t("modal.modePaste") },
                      { value: "file", label: t("modal.modeFile") },
                    ]}
                  />
                </div>
                {configMode === "file" && (
                  <div className="field">
                    <span>{t("modal.copyFile")}</span>
                    <div className="file-row">
                      <input
                        readOnly
                        value={fileLabel}
                        placeholder={t("modal.copyFilePh")}
                        disabled={busy}
                      />
                      <button
                        type="button"
                        className="secondary"
                        onClick={() => void pickFile()}
                        disabled={busy}
                      >
                        {t("modal.browse")}
                      </button>
                    </div>
                  </div>
                )}
                <label className="field">
                  <span>
                    {profile === "custom"
                      ? t("modal.contentCustom")
                      : t("modal.content")}
                  </span>
                  <textarea
                    className="config-paste"
                    autoCapitalize="off"
                    autoCorrect="off"
                    spellCheck={false}
                    value={content}
                    onChange={(e) => setContent(e.target.value)}
                    placeholder={
                      profile === "custom"
                        ? t("modal.contentPhCustom")
                        : t("modal.contentPhParse")
                    }
                    disabled={busy}
                    rows={profile === "custom" ? 12 : 8}
                  />
                </label>
                {profile === "custom" && content.trim().length > 0 && (
                  <span
                    className={`field-hint ${
                      detectedKind ? "detect-ok" : "field-warning"
                    }`}
                    role="status"
                  >
                    {detectedKind
                      ? t("modal.detectedAs", { kind: detectedLabel(detectedKind) })
                      : t("modal.detectUnknown")}
                  </span>
                )}
              </>
            )}

          <p className="hint">
            {profile === "subscription"
              ? isEdit
                ? t("modal.hintSubEdit")
                : t("modal.hintSubNew")
              : profile === "custom"
                ? t("modal.hintCustomTail")
                : localKind === "node"
                  ? t("modal.hintManualTail")
                  : t("modal.hintParseTail")}
          </p>

          {error && (
            <ErrorModal
              message={error}
              onClose={() => onDismissError?.()}
            />
          )}
          {fileError && <div className="form-error">{fileError}</div>}

          <footer className="modal-footer">
            <GlassButton onClick={onClose} disabled={busy}>
              {t("common.cancel")}
            </GlassButton>
            <GlassButton type="submit" variant="primary" disabled={!canSubmit}>
              {busy
                ? isEdit
                  ? t("common.saving")
                  : t("modal.importing")
                : (submitLabel ?? (isEdit ? t("common.save") : t("common.add")))}
            </GlassButton>
          </footer>
        </form>
      </div>
    </div>
  );
}
