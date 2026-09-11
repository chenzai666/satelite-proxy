import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  getDnsSettings,
  diagnoseDns,
  testDnsLookup,
  updateDnsSettings,
} from "../api";
import { GlassButton } from "../components/GlassButton";
import { GlassSeg } from "../components/GlassSeg";
import { GlassSwitchControl } from "../components/GlassSwitchControl";
import { ErrorModal } from "../components/ErrorModal";
import { useI18n } from "../i18n";
import type { DnsDiagReport, DnsFinalStrategy, DnsSettings, DnsTestResult } from "../types";

function SettingRow({
  title,
  desc,
  children,
}: {
  title: string;
  desc?: string;
  children: ReactNode;
}) {
  return (
    <div className="dns-setting-row">
      <div className="dns-setting-text">
        <div className="dns-setting-title">{title}</div>
        {desc && <div className="dns-setting-desc">{desc}</div>}
      </div>
      <div className="dns-setting-control">{children}</div>
    </div>
  );
}

export function DnsPage({ embedded = false }: { embedded?: boolean }) {
  const { t } = useI18n();
  const [dns, setDns] = useState<DnsSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [testDomain, setTestDomain] = useState("www.baidu.com");
  const [testResult, setTestResult] = useState<DnsTestResult | null>(null);
  const [diagReport, setDiagReport] = useState<DnsDiagReport | null>(null);
  const [testBusy, setTestBusy] = useState(false);
  const [bypassText, setBypassText] = useState("");
  const [remoteOpen, setRemoteOpen] = useState(false);
  const [remoteText, setRemoteText] = useState("");
  const [remoteError, setRemoteError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setError(null);
    try {
      const s = await getDnsSettings();
      setDns(s);
      setBypassText((s.fake_ip.bypass || []).join("\n"));
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  async function save(next: DnsSettings) {
    setBusy(true);
    setError(null);
    try {
      const s = await updateDnsSettings(next, true);
      setDns(s);
      setBypassText((s.fake_ip.bypass || []).join("\n"));
      return true;
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
      return false;
    } finally {
      setBusy(false);
    }
  }

  function patch(partial: Partial<DnsSettings>) {
    if (!dns) return;
    void save({ ...dns, ...partial });
  }

  function openRemoteModal() {
    if (!dns) return;
    setRemoteText((dns.remote_dns || []).join("\n"));
    setRemoteError(null);
    setRemoteOpen(true);
  }
  async function saveRemoteModal() {
    if (!dns) return;
    const entries = [...new Set(remoteText.split(/[\n,]/).map(s => s.trim()).filter(Boolean))];
    if (entries.length > 8) { setRemoteError(t("dns.remoteDnsMax")); return; }
    const invalid = entries.find(s => !s.startsWith("https://"));
    if (invalid) { setRemoteError(t("dns.remoteDnsInvalid", { v: invalid })); return; }
    if (await save({ ...dns, remote_dns: entries })) setRemoteOpen(false);
  }

  function saveFakeIp() {
    if (!dns) return;
    const bypass = bypassText
      .split(/[\n,]/)
      .map((s) => s.trim().replace(/^\*\./, "").replace(/^\./, ""))
      .filter(Boolean);
    void save({
      ...dns,
      fake_ip: { ...dns.fake_ip, bypass },
    });
  }

  async function onTest() {
    setTestBusy(true);
    setError(null);
    try {
      const [r, diag] = await Promise.all([
        testDnsLookup(testDomain),
        diagnoseDns([testDomain]),
      ]);
      setTestResult(r);
      setDiagReport(diag);
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setTestBusy(false);
    }
  }

  if (!dns && !error) {
    return (
      <div className={embedded ? "settings-embed empty" : "page empty"}>
        {t("common.loading")}
      </div>
    );
  }
  if (!dns) {
    return (
      <div className={embedded ? "settings-embed" : "page"}>
        {error && (
          <ErrorModal message={error} onClose={() => setError(null)} />
        )}
      </div>
    );
  }

  const wrapClass = embedded ? "settings-embed dns-page" : "page dns-page";

  return (
    <div className={wrapClass}>
      {!embedded && (
        <header className="page-header">
          <div>
            <h1>{t("dns.title")}</h1>
            <p className="page-desc">{t("dns.desc")}</p>
          </div>
        </header>
      )}

      {error && (
        <ErrorModal message={error} onClose={() => setError(null)} />
      )}

      <div className="dns-stack dns-grid dns-section-settings">
        <section className="card dns-panel dns-cell dns-cell-general">
          <header className="dns-panel-head">
            <h2>{t("dns.general")}</h2>
            <p>{t("dns.generalDesc")}</p>
          </header>

          <div className="dns-panel-body dns-general-body">
            <div className="dns-general-primary">
              <SettingRow title={t("dns.hijack")} desc={t("dns.hijackDesc")}>
                <GlassSwitchControl
                  checked={dns.hijack}
                  title={t("dns.hijack")}
                  disabled={busy}
                  onChange={(checked) => patch({ hijack: checked })}
                />
              </SettingRow>

              <SettingRow
                title={t("dns.defaultResolve")}
                desc={t("dns.defaultResolveDesc")}
              >
                <GlassSeg
                  value={dns.dns_final}
                  ariaLabel={t("dns.defaultResolve")}
                  disabled={busy}
                  onChange={(v) => patch({ dns_final: v as DnsFinalStrategy })}
                  options={[
                    { value: "local", label: t("dns.finalLocal") },
                    { value: "domestic", label: t("dns.finalDomestic") },
                    { value: "remote", label: t("dns.finalRemote") },
                  ]}
                />
              </SettingRow>
            </div>

            <SettingRow title={t("dns.remoteDns")} desc={t("dns.remoteDnsDesc")}>
              <GlassButton disabled={busy} onClick={openRemoteModal}>{t("dns.remoteDnsOptions")}</GlassButton>
            </SettingRow>
            <div className="dns-general-toggles">
              <SettingRow title={t("dns.cache")} desc={t("dns.cacheDesc")}>
                <GlassSwitchControl
                  checked={dns.cache}
                  title={t("dns.cache")}
                  disabled={busy}
                  onChange={(checked) => patch({ cache: checked })}
                />
              </SettingRow>
              <SettingRow title={t("dns.leak")} desc={t("dns.leakDesc")}>
                <GlassSwitchControl
                  checked={dns.leak_protect}
                  title={t("dns.leak")}
                  disabled={busy}
                  onChange={(checked) => patch({ leak_protect: checked })}
                />
              </SettingRow>
            </div>
          </div>
        </section>

        <section className="card dns-panel dns-cell dns-cell-fakeip">
          <header className="dns-panel-head">
            <h2>FakeIP</h2>
            <p>{t("dns.fakeipDesc")}</p>
          </header>
          <div className="dns-panel-body">
            <SettingRow
              title={t("dns.enableFakeip")}
              desc={t("dns.enableFakeipDesc")}
            >
              <GlassSwitchControl
                checked={dns.fake_ip.enabled}
                title={t("dns.enableFakeip")}
                disabled={busy}
                onChange={(checked) =>
                  void save({
                    ...dns,
                    fake_ip: {
                      ...dns.fake_ip,
                      enabled: checked,
                    },
                  })
                }
              />
            </SettingRow>

            <label className="field dns-field">
              <span>{t("dns.ipv4Pool")}</span>
              <input
                autoCapitalize="off"
                autoCorrect="off"
                spellCheck={false}
                value={dns.fake_ip.inet4_range}
                disabled={busy}
                onChange={(e) =>
                  setDns({
                    ...dns,
                    fake_ip: {
                      ...dns.fake_ip,
                      inet4_range: e.target.value,
                    },
                  })
                }
                onBlur={saveFakeIp}
              />
            </label>

            <SettingRow title={t("dns.ipv6Fakeip")} desc={t("dns.ipv6FakeipDesc")}>
              <GlassSwitchControl
                checked={dns.fake_ip.inet6_enabled}
                title={t("dns.ipv6Fakeip")}
                disabled={busy}
                onChange={(checked) =>
                  void save({
                    ...dns,
                    fake_ip: {
                      ...dns.fake_ip,
                      inet6_enabled: checked,
                    },
                  })
                }
              />
            </SettingRow>

            <label className="field dns-field">
              <span>{t("dns.bypassSuffix")}</span>
              <textarea
                autoCapitalize="off"
                autoCorrect="off"
                spellCheck={false}
                rows={4}
                value={bypassText}
                disabled={busy}
                onChange={(e) => setBypassText(e.target.value)}
                onBlur={saveFakeIp}
                placeholder={"local\nlan\ninternal"}
              />
            </label>
          </div>
        </section>

        <section className="card dns-panel dns-cell dns-cell-diag">
          <header className="dns-panel-head">
            <h2>{t("dns.diagTitle")}</h2>
            <p>{t("dns.diagDesc")}</p>
          </header>
          <div className="dns-panel-body">
            <label className="field dns-field">
              <span>{t("dns.domainLabel")}</span>
              <div className="dns-test-row">
                <input
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                  value={testDomain}
                  onChange={(e) => setTestDomain(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void onTest();
                  }}
                />
                <GlassButton
                  variant="primary"
                  icon="⌕"
                  disabled={testBusy}
                  onClick={() => void onTest()}
                >
                  {testBusy ? t("dns.testing") : t("dns.test")}
                </GlassButton>
              </div>
            </label>

            {testResult ? (
              <div className={`dns-test-card ${testResult.ok ? "ok" : "fail"}`}>
                <div className="dns-test-top">
                  <strong>{testResult.domain}</strong>
                  <span className="dns-test-badge">
                    {testResult.ok ? t("dns.testSuccess") : t("dns.testFail")}
                  </span>
                  <span className="muted">{testResult.elapsed_ms} ms</span>
                </div>
                {testResult.addrs.length > 0 && (
                  <div className="mono dns-test-addrs">
                    {testResult.addrs.join("\n")}
                  </div>
                )}
                {testResult.error && (
                  <div className="warn">{testResult.error}</div>
                )}
                <div className="dns-test-note">{testResult.note}</div>
                {diagReport?.results[0]?.path && (
                  <div className="dns-test-note">
                    <strong>{t("dns.corePath")}：</strong>
                    {diagReport.results[0].path.matched_by} · {diagReport.results[0].path.servers.join(" / ") || "—"}
                    {diagReport.results[0].path.via_proxy ? ` · ${t("dns.viaProxy")}` : ""}
                    {diagReport.results[0].query && ` · ${diagReport.results[0].query.elapsed_ms} ms`}
                  </div>
                )}
                {diagReport?.results[0]?.query_note && (
                  <div className="dns-test-note">{diagReport.results[0].query_note}</div>
                )}
              </div>
            ) : (
              <div className="dns-empty soft">{t("dns.diagEmptyHint")}</div>
            )}
          </div>
        </section>
        {remoteOpen && (
          <div className="modal-backdrop" onClick={() => setRemoteOpen(false)}>
            <div
              className="modal dns-fakeip-modal"
              onClick={(e) => e.stopPropagation()}
            >
              <header className="modal-header">
                <h2>{t("dns.remoteDnsOptions")}</h2>
                <button
                  type="button"
                  className="icon-btn"
                  onClick={() => setRemoteOpen(false)}
                >
                  ×
                </button>
              </header>
              <div className="modal-body">
                <label className="field dns-field">
                  <span>{t("dns.remoteDns")}</span>
                  <textarea
                    autoCapitalize="off"
                    autoCorrect="off"
                    spellCheck={false}
                    rows={4}
                    value={remoteText}
                    onChange={(e) => {
                      setRemoteText(e.target.value);
                      setRemoteError(null);
                    }}
                    placeholder={"https://9.9.9.9/dns-query\nhttps://94.140.14.14/dns-query"}
                  />
                </label>
                <p className="dns-remote-hint">{t("dns.remoteDnsHint")}</p>
                {remoteError && (
                  <p className="dns-remote-error">{remoteError}</p>
                )}
                <div className="dns-fakeip-modal-actions">
                  <GlassButton disabled={busy} onClick={() => setRemoteOpen(false)}>
                    {t("common.cancel")}
                  </GlassButton>
                  <GlassButton
                    variant="primary"
                    disabled={busy}
                    onClick={() => void saveRemoteModal()}
                  >
                    {t("common.save")}
                  </GlassButton>
                </div>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
