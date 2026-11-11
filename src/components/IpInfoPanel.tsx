import { useEffect, useRef, useState } from "react";
import { checkIpMetadata } from "../api";
import { useI18n } from "../i18n";
import type { ExitIpInfo, IpMetadata } from "../types";
import { createIpLookupCache } from "../ipLookupCache";
import "./IpInfoPanel.css";
const metadataCache = createIpLookupCache<IpMetadata>(value => ["unknown", "isp"].includes(value.networkType) ? 30_000 : 300_000, 64);

const labels = {
  zh: { title: "IP 信息", org: "组织", isp: "运营商", ispNetwork: "ISP 网络", business: "商业", type: "网络类型", quality: "IP 风险", unknown: "未知", datacenter: "机房", mobile: "移动网络", residential: "家宽", low: "风险低", moderate: "风险中", high: "风险高", checking: "查询中…", refresh: "重新查询 IP 信息", close: "关闭", copy: "复制完整组织名称", copied: "已复制", source: "资料来源", score: "风险评分", failed: "IP 信息暂不可用", details: "查看 IP 信息详情", exitSource: "出口检测来源", exitCheckedAt: "出口检测时间", copyUrl: "复制检测 URL", copyUrlFailed: "复制失败，请手动选择地址复制" },
  en: { title: "IP information", org: "Organization", isp: "ISP", ispNetwork: "ISP network", business: "Commercial", type: "Network type", quality: "IP risk", unknown: "Unknown", datacenter: "Datacenter", mobile: "Mobile", residential: "Residential", low: "Low risk", moderate: "Moderate risk", high: "High risk", checking: "Checking…", refresh: "Refresh IP information", close: "Close", copy: "Copy full organization name", copied: "Copied", source: "IP metadata source", score: "Risk score", failed: "IP information unavailable", details: "Open IP information details", exitSource: "Exit IP probe source", exitCheckedAt: "Exit IP checked at", copyUrl: "Copy probe URL", copyUrlFailed: "Copy failed. Select the URL to copy it manually." },
};

function ExitProbeSource({ source, l }: { source: string; l: typeof labels.en }) {
  const [copyStatus, setCopyStatus] = useState<"copied" | "failed" | null>(null);
  async function copyUrl() {
    try {
      await navigator.clipboard.writeText(source);
      setCopyStatus("copied");
    } catch {
      setCopyStatus("failed");
    }
  }
  return <>
    <span className="ip-exit-source-url">{source}</span>{" "}
    <button type="button" className="glass-btn small" onClick={copyUrl}>{l.copyUrl}</button>{" "}
    <span className="ip-info-hint" role="status">{copyStatus === "copied" ? l.copied : copyStatus === "failed" ? l.copyUrlFailed : ""}</span>
  </>;
}

export function IpInfoPanel({ exitIp, sessionKey }: { exitIp: ExitIpInfo | null; sessionKey: string }) {
  const { locale } = useI18n();
  const l = labels[locale === "en" ? "en" : "zh"];
  const ip = exitIp?.ip;
  const checkedAt = typeof exitIp?.checkedAt === "number" ? new Date(exitIp.checkedAt) : null;
  const [storedData, setData] = useState<IpMetadata | null>(null);
  const data = storedData?.ip === ip ? storedData : null;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  const [revision, setRevision] = useState(0);
  const lastRevision = useRef(0);
  const epoch = useRef(0);
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const token = ++epoch.current;
    const force = revision !== lastRevision.current;
    lastRevision.current = revision;
    setError(null); setCopied(false);
    if (!ip) { setData(null); setBusy(false); return; }
    const cached = metadataCache.peek(ip);
    setData(previous => cached ?? (previous?.ip === ip ? previous : null));
    if (cached && !force) { setBusy(false); return; }
    setBusy(true);
    void metadataCache.load(ip, async () => {
      const value = await checkIpMetadata(ip);
      if (value.ip !== ip) throw new Error("IP metadata mismatch");
      return value;
    }, force, sessionKey).then(value => {
      if (epoch.current === token && value.ip === ip) setData(value);
    }).catch(e => { if (epoch.current === token) setError(String(e)); })
      .finally(() => { if (epoch.current === token) setBusy(false); });
    return () => { epoch.current++; };
  }, [ip, sessionKey, revision]);
  useEffect(() => {
    if (open && dialog.current && !dialog.current.open) dialog.current.showModal();
    if (!open && dialog.current?.open) dialog.current.close();
  }, [open]);
  const type = data ? data.networkType === "isp" ? l.ispNetwork : l[data.networkType] : l.unknown;
  const qualityText = locale === "en"
    ? { good: "Good", normal: "Normal", risky: "Risky", unknown: "Unknown" }
    : { good: "优", normal: "普通", risky: "风险", unknown: "未知" };
  const quality = qualityText[data?.ipQuality ?? "unknown"];
  const risk = data ? l[data.quality] : l.unknown;
  const qualityLabel = locale === "en" ? "Quality" : "质量";
  const showType = !!data && ["residential", "mobile", "business", "datacenter"].includes(data.networkType);
  const showQuality = showType || data?.ipQuality === "risky";
  const lookupNotice = locale === "en"
    ? "Queries ident.me, IPQuery, IPLocate, ipapi.is and proxycheck.io over HTTPS in fallback waves. Only the public IP is used, without node credentials. Types require explicit data for the same IP. Quality follows FlClash: residential/mobile/commercial Good, datacenter Normal, Tor or abuse Risky; missing data Unknown. Risk scores remain separate and use IPQuery bands: 0–20 low, 21–50 moderate, 51–100 high."
    : "按批次通过 HTTPS 查询 ident.me、IPQuery、IPLocate、ipapi.is、proxycheck.io，只使用公网 IP，不发送节点凭据。类型以同一 IP 的明确数据为准。质量沿用 FlClash 分级：家宽/移动/商业为优，机房为普通，Tor 或滥用标记为风险，信息不足为未知。风险分数单独展示，沿用 IPQuery 阈值：0–20低、21–50中、51–100高。";
  return (
    <div className="ip-info-panel">
      <div className="ip-info-badges">
        <button type="button" className="ip-info-open glass-btn small" onClick={() => setOpen(true)} title={l.details}>{l.title} ↗</button>
        {showType && <span className="ip-info-tag ip-network-tag">{type}</span>}
        {showQuality && <span className={`ip-info-tag ip-quality-${data?.ipQuality ?? "unknown"}`}>{qualityLabel} {quality}</span>}
        {busy && <span className="ip-info-hint">{l.checking}</span>}
      </div>
      <div className="ip-organization-row">
        <span>{l.org}</span>
        <strong className="ip-organization" title={data?.organization ?? undefined}>{data?.organization ?? "—"}</strong>
        {data?.organization && <button type="button" className="icon-btn" title={copied ? l.copied : l.copy} aria-label={l.copy} onClick={() => {
          void navigator.clipboard.writeText(data.organization!).then(() => setCopied(true)).catch(() => setError(l.copy));
        }}>{copied ? "✓" : "⧉"}</button>}
      </div>
      <dialog ref={dialog} className="ip-details-dialog" aria-labelledby="ip-details-title" onCancel={() => setOpen(false)} onClose={() => setOpen(false)}>
        <header><h2 id="ip-details-title">{l.title}</h2><button type="button" className="glass-btn small" onClick={() => setOpen(false)} autoFocus>{l.close}</button></header>
        <dl>
          <dt>IP</dt><dd>{ip ?? "—"}</dd>
          <dt>{l.exitSource}</dt><dd>{exitIp?.source ? <ExitProbeSource key={exitIp.source} source={exitIp.source} l={l} /> : "—"}</dd>
          <dt>{l.exitCheckedAt}</dt><dd>{checkedAt && Number.isFinite(checkedAt.getTime())
            ? <time dateTime={checkedAt.toISOString()}>{checkedAt.toLocaleString(locale === "en" ? "en-US" : "zh-CN")}</time>
            : "—"}</dd>
          <dt>{l.org}</dt><dd className="ip-organization">{data?.organization ?? "—"}</dd>
          <dt>{l.isp}</dt><dd>{data?.isp ?? "—"}</dd>
          <dt>ASN</dt><dd>{data?.asn ?? "—"}</dd>
          <dt>{l.type}</dt><dd>{type}</dd>
          <dt>{qualityLabel}</dt><dd>{quality}</dd>
          <dt>{l.quality}</dt><dd>{risk}</dd>
          <dt>{l.score}</dt><dd>{data?.riskScore == null ? "—" : `${data.riskScore} / 100`}</dd>
          <dt>{locale === "en" ? "Risk source" : "风险来源"}</dt><dd>{data?.riskSource || "—"}</dd>
          <dt>VPN / Proxy / Tor / Abuse</dt><dd>{[data?.isVpn, data?.isProxy, data?.isTor, data?.isAbuser].map(v => v == null ? "—" : v ? "✓" : "−").join(" / ")}</dd>
          <dt>{l.source}</dt><dd>{data?.source || "—"}</dd>
        </dl>
        <p className="ip-info-notice">{lookupNotice}</p>
        {error && <p role="alert">{l.failed}: {error}</p>}
        <button type="button" className="glass-btn" disabled={!ip || busy} onClick={() => setRevision(v => v + 1)}>{busy ? l.checking : l.refresh}</button>
      </dialog>
    </div>
  );
}
