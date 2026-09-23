import { GlassSwitchControl } from "./GlassSwitchControl";
import { SolidSelect } from "./SolidSelect";
import { useI18n } from "../i18n";
import type { ManualNodeDraft } from "../types";

export const NODE_PROTOCOLS = [
  { value: "vless", label: "VLESS" },
  { value: "vmess", label: "VMess" },
  { value: "trojan", label: "Trojan" },
  { value: "shadowsocks", label: "Shadowsocks" },
  { value: "hysteria2", label: "Hysteria2" },
  { value: "tuic", label: "TUIC" },
  { value: "socks5", label: "SOCKS5" },
  { value: "http", label: "HTTP" },
  { value: "anytls", label: "AnyTLS" },
  { value: "snell", label: "Snell" },
  { value: "masque", label: "MASQUE" },
  { value: "hysteria", label: "Hysteria" },
  { value: "ssh", label: "SSH" },
  { value: "wireguard", label: "WireGuard" },
  { value: "shadowtls", label: "ShadowTLS" },
  { value: "naive", label: "Naive" },
  { value: "tor", label: "Tor" },
] as const;

const SS_METHODS = [
  "aes-128-gcm",
  "aes-256-gcm",
  "chacha20-ietf-poly1305",
  "2022-blake3-aes-128-gcm",
  "2022-blake3-aes-256-gcm",
  "2022-blake3-chacha20-poly1305",
  "none",
];

const VMESS_SECURITY = ["auto", "aes-128-gcm", "chacha20-poly1305", "none"];
const NETWORKS = [
  { value: "tcp", label: "TCP" },
  { value: "ws", label: "WebSocket" },
  { value: "grpc", label: "gRPC" },
  { value: "http", label: "HTTP" },
  { value: "httpupgrade", label: "HTTPUpgrade" },
  { value: "xhttp", label: "XHTTP" },
];
const FINGERPRINTS = [
  "chrome",
  "firefox",
  "safari",
  "ios",
  "android",
  "edge",
  "random",
];

export function emptyNodeDraft(): ManualNodeDraft {
  return {
    protocol: "vless",
    server: "",
    port: 443,
    tls: true,
    network: "tcp",
    packetEncoding: "xudp",
    method: "aes-256-gcm",
    security: "auto",
    version: 4,
  };
}

function hasTls(protocol: string) {
  return [
    "vless",
    "vmess",
    "trojan",
    "hysteria2",
    "tuic",
    "http",
    "hysteria",
    "anytls",
    "shadowtls",
    "naive",
    "socks5",
  ].includes(protocol);
}

function hasTransport(protocol: string) {
  return ["vless", "vmess", "trojan", "shadowsocks"].includes(protocol);
}

function needsServer(protocol: string) {
  return protocol !== "tor";
}

export function nodeDraftReady(draft: ManualNodeDraft): boolean {
  const p = draft.protocol;
  if (needsServer(p) && (!draft.server.trim() || !draft.port)) return false;
  switch (p) {
    case "shadowsocks":
    case "trojan":
    case "hysteria2":
    case "anytls":
      return !!(draft.password ?? "").trim();
    case "vmess":
    case "vless":
    case "tuic":
      return !!(draft.uuid ?? "").trim();
    case "hysteria":
      return !!(draft.password ?? "").trim();
    case "naive":
      return !!(draft.username ?? "").trim() && !!(draft.password ?? "").trim();
    case "ssh":
      return !!(draft.password ?? "").trim() || !!(draft.privateKey ?? "").trim();
    case "tor":
      return !!(draft.executablePath ?? "").trim();
    case "wireguard":
      return (
        !!(draft.privateKey ?? "").trim() &&
        !!(draft.peerPublicKey ?? "").trim() &&
        !!(draft.localAddress ?? "").trim()
      );
    case "snell":
      return !!(draft.psk ?? draft.password ?? "").trim();
    case "masque":
      return (
        !!(draft.privateKey ?? "").trim() && !!(draft.publicKey ?? "").trim()
      );
    default:
      return true;
  }
}

interface Props {
  value: ManualNodeDraft;
  disabled?: boolean;
  onChange: (next: ManualNodeDraft) => void;
}

export function NodeDraftFields({ value, disabled, onChange }: Props) {
  const { t } = useI18n();
  const p = value.protocol || "vless";
  const set = (patch: Partial<ManualNodeDraft>) =>
    onChange({ ...value, ...patch });

  const protocolOptions = NODE_PROTOCOLS.map((o) => ({
    value: o.value,
    label: o.value === "masque" ? t("nodeDraft.masqueOption") : o.label,
  }));
  const networkOptions = NETWORKS.map((o) => ({
    value: o.value,
    label: o.value === "xhttp" ? t("nodeDraft.xhttpOption") : o.label,
  }));

  return (
    <div className="node-draft">
      <label className="field">
        <span>{t("nodeDraft.protocol")}</span>
        <SolidSelect
          aria-label={t("nodeDraft.protocolAria")}
          value={p}
          disabled={disabled}
          options={protocolOptions}
          onChange={(protocol) => {
            const tlsOn = [
              "vless",
              "trojan",
              "hysteria2",
              "tuic",
              "anytls",
              "hysteria",
              "shadowtls",
              "naive",
              "masque",
            ].includes(protocol);
            set({
              protocol,
              tls: tlsOn,
              port:
                value.port ||
                (protocol === "shadowsocks"
                  ? 8388
                  : protocol === "socks5"
                    ? 1080
                    : 443),
            });
          }}
        />
      </label>

      {needsServer(p) && (
        <div className="field-grid">
          <label className="field">
            <span>{t("nodeDraft.server")}</span>
            <input
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              value={value.server}
              onChange={(e) => set({ server: e.target.value })}
              placeholder="example.com"
              disabled={disabled}
            />
          </label>
          <label className="field">
            <span>{t("nodeDraft.port")}</span>
            <input
              type="number"
              min={1}
              max={65535}
              value={value.port || ""}
              onChange={(e) =>
                set({ port: Number.parseInt(e.target.value, 10) || 0 })
              }
              disabled={disabled}
            />
          </label>
        </div>
      )}

      {p === "shadowsocks" && (
        <>
          <label className="field">
            <span>{t("nodeDraft.encryption")}</span>
            <SolidSelect
              aria-label={t("nodeDraft.encryptionAria")}
              value={value.method || "aes-256-gcm"}
              disabled={disabled}
              options={SS_METHODS.map((m) => ({ value: m, label: m }))}
              onChange={(method) => set({ method })}
            />
          </label>
          <label className="field">
            <span>{t("nodeDraft.password")}</span>
            <input
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              value={value.password ?? ""}
              onChange={(e) => set({ password: e.target.value })}
              disabled={disabled}
            />
          </label>
          <div className="field-grid">
            <label className="field">
              <span>{t("nodeDraft.plugin")}</span>
              <input
                value={value.plugin ?? ""}
                onChange={(e) => set({ plugin: e.target.value })}
                placeholder={t("nodeDraft.optional")}
                disabled={disabled}
              />
            </label>
            <label className="field">
              <span>{t("nodeDraft.pluginOpts")}</span>
              <input
                value={value.pluginOpts ?? ""}
                onChange={(e) => set({ pluginOpts: e.target.value })}
                placeholder={t("nodeDraft.optional")}
                disabled={disabled}
              />
            </label>
          </div>
        </>
      )}

      {(p === "vless" || p === "vmess" || p === "tuic") && (
        <label className="field">
          <span>UUID</span>
          <input
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            value={value.uuid ?? ""}
            onChange={(e) => set({ uuid: e.target.value })}
            disabled={disabled}
          />
        </label>
      )}

      {p === "vless" && (
        <label className="field">
          <span>Flow</span>
          <input
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            value={value.flow ?? ""}
            onChange={(e) => set({ flow: e.target.value })}
            placeholder={t("nodeDraft.flowPh")}
            disabled={disabled}
          />
        </label>
      )}

      {p === "vmess" && (
        <div className="field-grid">
          <label className="field">
            <span>{t("nodeDraft.encryption")}</span>
            <SolidSelect
              aria-label={t("nodeDraft.vmessEncryptionAria")}
              value={value.security || "auto"}
              disabled={disabled}
              options={VMESS_SECURITY.map((m) => ({ value: m, label: m }))}
              onChange={(security) => set({ security })}
            />
          </label>
          <label className="field">
            <span>alterId</span>
            <input
              type="number"
              min={0}
              value={value.alterId ?? 0}
              onChange={(e) =>
                set({ alterId: Number.parseInt(e.target.value, 10) || 0 })
              }
              disabled={disabled}
            />
          </label>
        </div>
      )}

      {(p === "trojan" ||
        p === "hysteria2" ||
        p === "anytls" ||
        p === "hysteria" ||
        p === "tuic") && (
        <label className="field">
          <span>{p === "hysteria" ? "Auth" : t("nodeDraft.password")}</span>
          <input
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            value={value.password ?? ""}
            onChange={(e) => set({ password: e.target.value })}
            disabled={disabled}
          />
        </label>
      )}

      {(p === "socks5" || p === "http" || p === "naive") && (
        <div className="field-grid">
          <label className="field">
            <span>{t("nodeDraft.username")}</span>
            <input
              value={value.username ?? ""}
              onChange={(e) => set({ username: e.target.value })}
              placeholder={p === "naive" ? t("nodeDraft.required") : t("nodeDraft.optional")}
              disabled={disabled}
            />
          </label>
          <label className="field">
            <span>{t("nodeDraft.password")}</span>
            <input
              value={value.password ?? ""}
              onChange={(e) => set({ password: e.target.value })}
              placeholder={p === "naive" ? t("nodeDraft.required") : t("nodeDraft.optional")}
              disabled={disabled}
            />
          </label>
        </div>
      )}

      {(p === "hysteria2" || p === "hysteria") && (
        <div className="field-grid">
          <label className="field">
            <span>{t("nodeDraft.upMbps")}</span>
            <input
              type="number"
              min={0}
              value={value.upMbps ?? ""}
              onChange={(e) =>
                set({
                  upMbps: e.target.value
                    ? Number.parseInt(e.target.value, 10)
                    : null,
                })
              }
              disabled={disabled}
            />
          </label>
          <label className="field">
            <span>{t("nodeDraft.downMbps")}</span>
            <input
              type="number"
              min={0}
              value={value.downMbps ?? ""}
              onChange={(e) =>
                set({
                  downMbps: e.target.value
                    ? Number.parseInt(e.target.value, 10)
                    : null,
                })
              }
              disabled={disabled}
            />
          </label>
        </div>
      )}

      {p === "hysteria2" && (
        <div className="field-grid">
          <label className="field">
            <span>{t("nodeDraft.obfs")}</span>
            <input
              value={value.obfs ?? ""}
              onChange={(e) => set({ obfs: e.target.value })}
              placeholder={t("nodeDraft.obfsPh")}
              disabled={disabled}
            />
          </label>
          <label className="field">
            <span>{t("nodeDraft.obfsPassword")}</span>
            <input
              value={value.obfsPassword ?? ""}
              onChange={(e) => set({ obfsPassword: e.target.value })}
              disabled={disabled}
            />
          </label>
        </div>
      )}

      {p === "tuic" && (
        <div className="field-grid">
          <label className="field">
            <span>{t("nodeDraft.congestionControl")}</span>
            <input
              value={value.congestionControl ?? ""}
              onChange={(e) => set({ congestionControl: e.target.value })}
              placeholder="bbr / cubic"
              disabled={disabled}
            />
          </label>
          <label className="field">
            <span>{t("nodeDraft.udpRelayMode")}</span>
            <input
              value={value.udpRelayMode ?? ""}
              onChange={(e) => set({ udpRelayMode: e.target.value })}
              placeholder="native / quic"
              disabled={disabled}
            />
          </label>
        </div>
      )}

      {p === "snell" && (
        <>
          <label className="field">
            <span>PSK</span>
            <input
              value={value.psk ?? value.password ?? ""}
              onChange={(e) => set({ psk: e.target.value, password: e.target.value })}
              disabled={disabled}
            />
          </label>
          <div className="field-grid">
            <label className="field">
              <span>{t("nodeDraft.version")}</span>
              <SolidSelect
                aria-label={t("nodeDraft.snellVersionAria")}
                value={String(value.version ?? 4)}
                disabled={disabled}
                options={[
                  { value: "4", label: "4" },
                  { value: "6", label: "6" },
                ]}
                onChange={(v) => set({ version: Number(v) })}
              />
            </label>
            <label className="field">
              <span>Obfs</span>
              <input
                value={value.obfs ?? ""}
                onChange={(e) => set({ obfs: e.target.value })}
                placeholder="http / none"
                disabled={disabled}
              />
            </label>
          </div>
        </>
      )}

      {p === "masque" && (
        <>
          <label className="field">
            <span>{t("nodeDraft.masquePrivKey")}</span>
            <textarea
              className="config-paste"
              value={value.privateKey ?? ""}
              onChange={(e) => set({ privateKey: e.target.value })}
              placeholder={t("nodeDraft.masquePrivKeyPh")}
              disabled={disabled}
              rows={3}
            />
          </label>
          <label className="field">
            <span>{t("nodeDraft.masquePubKey")}</span>
            <textarea
              className="config-paste"
              value={value.publicKey ?? ""}
              onChange={(e) => set({ publicKey: e.target.value })}
              placeholder={t("nodeDraft.masquePubKeyPh")}
              disabled={disabled}
              rows={3}
            />
          </label>
          <div className="field-grid">
            <label className="field">
              <span>{t("nodeDraft.localIpv4")}</span>
              <input
                value={value.ip ?? ""}
                onChange={(e) => set({ ip: e.target.value })}
                placeholder={t("nodeDraft.localIpv4Ph")}
                disabled={disabled}
              />
            </label>
            <label className="field">
              <span>{t("nodeDraft.localIpv6")}</span>
              <input
                value={value.ipv6 ?? ""}
                onChange={(e) => set({ ipv6: e.target.value })}
                placeholder={t("nodeDraft.localIpv6Ph")}
                disabled={disabled}
              />
            </label>
          </div>
          <div className="field-grid">
            <label className="field">
              <span>{t("nodeDraft.mode")}</span>
              <SolidSelect
                aria-label={t("nodeDraft.masqueModeAria")}
                value={value.network || "quic"}
                disabled={disabled}
                options={[
                  { value: "quic", label: "QUIC (H3)" },
                  { value: "h2", label: "H2" },
                  { value: "h3-l4proxy", label: "H3 L4 Proxy" },
                ]}
                onChange={(network) => set({ network })}
              />
            </label>
              <label className="field">
              <span>MTU</span>
              <input
                type="number"
                min={0}
                value={value.mtu ?? ""}
                onChange={(e) =>
                  set({
                    mtu: e.target.value
                      ? Number.parseInt(e.target.value, 10)
                      : null,
                  })
                }
                placeholder={t("nodeDraft.mtuPh")}
                disabled={disabled}
              />
            </label>
          </div>
          <label className="field">
            <span>SNI</span>
            <input
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              value={value.sni ?? ""}
              onChange={(e) => set({ sni: e.target.value })}
              placeholder={t("nodeDraft.sniPh")}
              disabled={disabled}
            />
          </label>
          <div className="via-proxy-row">
            <div>
              <div className="sys-proxy-title">{t("nodeDraft.skipCertVerify")}</div>
            </div>
            <GlassSwitchControl
              checked={!!value.insecure}
              title="insecure"
              disabled={disabled}
              onChange={(insecure) => set({ insecure })}
            />
          </div>
        </>
      )}

      {p === "ssh" && (
        <>
          <label className="field">
            <span>{t("nodeDraft.sshUser")}</span>
            <input
              value={value.user ?? value.username ?? ""}
              onChange={(e) => set({ user: e.target.value })}
              placeholder="root"
              disabled={disabled}
            />
          </label>
          <label className="field">
            <span>{t("nodeDraft.password")}</span>
            <input
              value={value.password ?? ""}
              onChange={(e) => set({ password: e.target.value })}
              placeholder={t("nodeDraft.sshPasswordPh")}
              disabled={disabled}
            />
          </label>
          <label className="field">
            <span>{t("nodeDraft.privateKey")}</span>
            <textarea
              className="config-paste"
              value={value.privateKey ?? ""}
              onChange={(e) => set({ privateKey: e.target.value })}
              placeholder={t("nodeDraft.sshPrivKeyPh")}
              disabled={disabled}
              rows={3}
            />
          </label>
        </>
      )}

      {p === "wireguard" && (
        <>
          <label className="field">
            <span>{t("nodeDraft.localAddress")}</span>
            <input
              value={value.localAddress ?? ""}
              onChange={(e) => set({ localAddress: e.target.value })}
              placeholder="10.0.0.2/32"
              disabled={disabled}
            />
          </label>
          <label className="field">
            <span>{t("nodeDraft.privateKey")}</span>
            <input
              value={value.privateKey ?? ""}
              onChange={(e) => set({ privateKey: e.target.value })}
              disabled={disabled}
            />
          </label>
          <label className="field">
            <span>{t("nodeDraft.peerPublicKey")}</span>
            <input
              value={value.peerPublicKey ?? ""}
              onChange={(e) => set({ peerPublicKey: e.target.value })}
              disabled={disabled}
            />
          </label>
        </>
      )}

      {p === "shadowtls" && (
        <div className="field-grid">
          <label className="field">
            <span>{t("nodeDraft.version")}</span>
            <SolidSelect
              aria-label={t("nodeDraft.shadowtlsVersionAria")}
              value={String(value.version ?? 3)}
              disabled={disabled}
              options={[
                { value: "1", label: "1" },
                { value: "2", label: "2" },
                { value: "3", label: "3" },
              ]}
              onChange={(v) => set({ version: Number(v) })}
            />
          </label>
          <label className="field">
            <span>{t("nodeDraft.password")}</span>
            <input
              value={value.password ?? ""}
              onChange={(e) => set({ password: e.target.value })}
              disabled={disabled}
            />
          </label>
        </div>
      )}

      {p === "tor" && (
        <label className="field">
          <span>{t("nodeDraft.executable")}</span>
          <input
            value={value.executablePath ?? ""}
            onChange={(e) => set({ executablePath: e.target.value })}
            placeholder={t("nodeDraft.executablePh")}
            disabled={disabled}
          />
        </label>
      )}

      {hasTls(p) && (
        <>
          <div className="via-proxy-row">
            <div>
              <div className="sys-proxy-title">TLS</div>
              <div className="sys-proxy-desc">{t("nodeDraft.tlsDesc")}</div>
            </div>
            <GlassSwitchControl
              checked={!!value.tls}
              title="TLS"
              disabled={disabled}
              onChange={(tls) => set({ tls })}
            />
          </div>
          {value.tls && (
            <>
              <div className="field-grid">
                <label className="field">
                  <span>SNI</span>
                  <input
                    autoCapitalize="off"
                    autoCorrect="off"
                    spellCheck={false}
                    value={value.sni ?? ""}
                    onChange={(e) => set({ sni: e.target.value })}
                    placeholder={t("nodeDraft.sniPh")}
                    disabled={disabled}
                  />
                </label>
                <label className="field">
                  <span>{t("nodeDraft.fingerprint")}</span>
                  <SolidSelect
                    aria-label={t("nodeDraft.fingerprintAria")}
                    value={value.fingerprint || ""}
                    disabled={disabled}
                    placeholder={t("nodeDraft.fingerprintDefault")}
                    options={[
                      { value: "", label: t("nodeDraft.fingerprintDefault") },
                      ...FINGERPRINTS.map((f) => ({ value: f, label: f })),
                    ]}
                    onChange={(fingerprint) =>
                      set({ fingerprint: fingerprint || null })
                    }
                  />
                </label>
              </div>
              <div className="via-proxy-row">
                <div>
                  <div className="sys-proxy-title">{t("nodeDraft.skipCertVerify")}</div>
                </div>
                <GlassSwitchControl
                  checked={!!value.insecure}
                  title="insecure"
                  disabled={disabled}
                  onChange={(insecure) => set({ insecure })}
                />
              </div>
              {(p === "vless" || p === "vmess") && (
                <div className="field-grid">
                  <label className="field">
                    <span>{t("nodeDraft.realityPublicKey")}</span>
                    <input
                      value={value.realityPublicKey ?? ""}
                      onChange={(e) => set({ realityPublicKey: e.target.value })}
                      placeholder={t("nodeDraft.optional")}
                      disabled={disabled}
                    />
                  </label>
                  <label className="field">
                    <span>{t("nodeDraft.shortId")}</span>
                    <input
                      value={value.realityShortId ?? ""}
                      onChange={(e) => set({ realityShortId: e.target.value })}
                      placeholder={t("nodeDraft.optional")}
                      disabled={disabled}
                    />
                  </label>
                </div>
              )}
            </>
          )}
        </>
      )}

      {hasTransport(p) && (
        <>
          <label className="field">
            <span>{t("nodeDraft.transport")}</span>
            <SolidSelect
              aria-label={t("nodeDraft.transportAria")}
              value={value.network || "tcp"}
              disabled={disabled}
              options={networkOptions}
              onChange={(network) => set({ network })}
            />
          </label>
          {(value.network === "ws" ||
            value.network === "http" ||
            value.network === "httpupgrade" ||
            value.network === "xhttp") && (
            <div className="field-grid">
              <label className="field">
                <span>{t("nodeDraft.path")}</span>
                <input
                  value={value.path ?? ""}
                  onChange={(e) => set({ path: e.target.value })}
                  placeholder="/path"
                  disabled={disabled}
                />
              </label>
              <label className="field">
                <span>Host</span>
                <input
                  value={value.host ?? ""}
                  onChange={(e) => set({ host: e.target.value })}
                  disabled={disabled}
                />
              </label>
            </div>
          )}
          {value.network === "grpc" && (
            <label className="field">
              <span>Service Name</span>
              <input
                value={value.serviceName ?? ""}
                onChange={(e) => set({ serviceName: e.target.value })}
                disabled={disabled}
              />
            </label>
          )}
        </>
      )}
    </div>
  );
}
