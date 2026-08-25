import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  getDnsSettings,
  testDnsLookup,
  updateDnsSettings,
} from "../api";
import { GlassButton } from "../components/GlassButton";
import { GlassSeg } from "../components/GlassSeg";
import { GlassSwitchControl } from "../components/GlassSwitchControl";
import { ErrorModal } from "../components/ErrorModal";
import { useI18n } from "../i18n";
import type { DnsFinalStrategy, DnsSettings, DnsTestResult } from "../types";

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
  const [testBusy, setTestBusy] = useState(false);
  const [bypassText, setBypassText] = useState("");

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
      const r = await testDnsLookup(testDomain);
      setTestResult(r);
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
              </div>
            ) : (
              <div className="dns-empty soft">{t("dns.diagEmptyHint")}</div>
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
