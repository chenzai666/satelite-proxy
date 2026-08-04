import { useCallback, useEffect, useState, type ReactNode } from "react";
import { getDnsSettings, testDnsLookup, updateDnsSettings } from "../api";
import { useI18n } from "../i18n";
import type {
  DnsAction,
  DnsMode,
  DnsRule,
  DnsServer,
  DnsServerRole,
  DnsSettings,
  DnsTestResult,
  DomainMatcher,
} from "../types";

function newId(prefix: string) {
  return `${prefix}-${Math.random().toString(36).slice(2, 10)}`;
}

function roleLabel(role: DnsServerRole) {
  switch (role) {
    case "local":
      return "系统";
    case "domestic":
      return "国内";
    case "remote":
      return "远程";
    case "custom":
      return "自定义";
  }
}

function actionLabel(a: DnsAction): string {
  switch (a.kind) {
    case "system":
      return "系统 DNS";
    case "domestic":
      return "国内 DNS";
    case "remote":
      return "远程 DNS";
    case "block":
      return "拦截";
    case "fake_ip":
      return "FakeIP";
    case "server":
      return `指定服务器`;
  }
}

function matcherLabel(m: DomainMatcher) {
  switch (m) {
    case "domain":
      return "精确";
    case "domain_suffix":
      return "后缀";
    case "domain_keyword":
      return "关键字";
  }
}

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

interface Props {
  /** Hide page chrome when embedded under Settings. */
  embedded?: boolean;
}

export function DnsPage({ embedded = false }: Props) {
  const { t } = useI18n();
  const [dns, setDns] = useState<DnsSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const [testDomain, setTestDomain] = useState("www.baidu.com");
  const [testResult, setTestResult] = useState<DnsTestResult | null>(null);
  const [testBusy, setTestBusy] = useState(false);

  const [newServerName, setNewServerName] = useState("");
  const [newServerAddr, setNewServerAddr] = useState("");
  const [newServerRole, setNewServerRole] = useState<DnsServerRole>("custom");

  const [newRulePayload, setNewRulePayload] = useState("");
  const [newRuleMatcher, setNewRuleMatcher] =
    useState<DomainMatcher>("domain_suffix");
  const [newRuleAction, setNewRuleAction] = useState<
    "system" | "domestic" | "remote"
  >("system");

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
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
  }

  function patch(partial: Partial<DnsSettings>) {
    if (!dns) return;
    void save({ ...dns, ...partial });
  }

  function setMode(mode: DnsMode) {
    if (!dns) return;
    void save({ ...dns, mode, enabled: true });
  }

  /** Top-level path: system resolver vs remote-oriented DNS. */
  function setDnsPath(path: "system" | "remote") {
    if (!dns) return;
    if (path === "system") {
      void save({ ...dns, enabled: true, mode: "system" });
      return;
    }
    // Remote: keep smart/custom if already non-system; default smart.
    const nextMode =
      dns.mode === "system" || !dns.enabled ? "smart" : dns.mode;
    void save({ ...dns, enabled: true, mode: nextMode });
  }

  function toggleServer(id: string) {
    if (!dns) return;
    void save({
      ...dns,
      servers: dns.servers.map((s) =>
        s.id === id ? { ...s, enabled: !s.enabled } : s,
      ),
    });
  }

  function removeServer(id: string) {
    if (!dns) return;
    if (dns.servers.find((s) => s.id === id)?.role === "local") {
      setError("系统 DNS 不可删除");
      return;
    }
    void save({ ...dns, servers: dns.servers.filter((s) => s.id !== id) });
  }

  function addServer() {
    if (!dns) return;
    const name = newServerName.trim() || "Custom";
    const address = newServerAddr.trim();
    if (!address) {
      setError("请填写 DNS 地址");
      return;
    }
    const s: DnsServer = {
      id: newId("srv"),
      name,
      address,
      role: newServerRole,
      enabled: true,
    };
    void save({ ...dns, servers: [...dns.servers, s] });
    setNewServerName("");
    setNewServerAddr("");
  }

  function toggleRule(id: string) {
    if (!dns) return;
    void save({
      ...dns,
      rules: dns.rules.map((r) =>
        r.id === id ? { ...r, enabled: !r.enabled } : r,
      ),
    });
  }

  function removeRule(id: string) {
    if (!dns) return;
    void save({ ...dns, rules: dns.rules.filter((r) => r.id !== id) });
  }

  function addRule() {
    if (!dns) return;
    const payload = newRulePayload
      .trim()
      .replace(/^\*\./, "")
      .replace(/^\./, "");
    if (!payload) {
      setError("请填写域名匹配");
      return;
    }
    const action: DnsAction =
      newRuleAction === "domestic"
        ? { kind: "domestic" }
        : newRuleAction === "remote"
          ? { kind: "remote" }
          : { kind: "system" };
    const r: DnsRule = {
      id: newId("rule"),
      enabled: true,
      matcher: newRuleMatcher,
      payload,
      action,
    };
    void save({ ...dns, rules: [...dns.rules, r] });
    setNewRulePayload("");
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
        加载中…
      </div>
    );
  }
  if (!dns) {
    return (
      <div className={embedded ? "settings-embed" : "page"}>
        <div className="banner error">{error}</div>
      </div>
    );
  }

  const mode = dns.enabled ? dns.mode : "system";
  const dnsPath: "system" | "remote" =
    !dns.enabled || mode === "system" ? "system" : "remote";
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

      {error && <div className="banner error">{error}</div>}

      <div className="dns-stack dns-grid">
        {/* —— General —— */}
        <section className="card dns-panel dns-cell dns-cell-general">
          <header className="dns-panel-head">
            <h2>常规</h2>
            <p>模式与全局行为</p>
          </header>

          <div className="dns-panel-body dns-general-body">
            <div className="dns-general-primary">
              <SettingRow
                title="DNS Provider"
                desc={
                  dnsPath === "system"
                    ? "仅使用系统解析，适合企业 VPN / 内网"
                    : "使用远程 / 分流 DNS（Smart 或自定义）"
                }
              >
                <div
                  className="segmented compact dns-path-seg"
                  role="group"
                  aria-label="DNS Provider"
                >
                  <button
                    type="button"
                    className={`seg ${dnsPath === "system" ? "active" : ""}`}
                    disabled={busy}
                    onClick={() => setDnsPath("system")}
                  >
                    系统
                  </button>
                  <button
                    type="button"
                    className={`seg ${dnsPath === "remote" ? "active" : ""}`}
                    disabled={busy}
                    onClick={() => setDnsPath("remote")}
                  >
                    远程
                  </button>
                </div>
              </SettingRow>

              {dnsPath === "remote" && (
                <div className="dns-mode-block">
                  <div className="dns-mode-label">解析模式</div>
                  <div className="seg-group dns-seg">
                    {(
                      [
                        ["smart", "Smart"],
                        ["custom", "Custom"],
                      ] as const
                    ).map(([k, label]) => (
                      <button
                        key={k}
                        type="button"
                        className={`seg ${mode === k ? "active" : ""}`}
                        disabled={busy}
                        onClick={() => setMode(k)}
                      >
                        {label}
                      </button>
                    ))}
                  </div>
                  <p className="dns-mode-hint">
                    {mode === "smart" &&
                      "国内 + 远程 DoH + 白名单走系统；可启用 FakeIP。"}
                    {mode === "custom" && "完全按下方服务器与规则配置。"}
                  </p>
                </div>
              )}
            </div>

            <div className="dns-general-toggles">
              <SettingRow
                title="DNS 劫持"
                desc="拦截系统 DNS 流量进入 sing-box（TUN 建议开）"
              >
                <button
                  type="button"
                  role="switch"
                  className={`switch ${dns.hijack ? "on" : ""}`}
                  disabled={busy}
                  aria-checked={dns.hijack}
                  onClick={() => patch({ hijack: !dns.hijack })}
                >
                  <span className="switch-thumb" />
                </button>
              </SettingRow>
              <SettingRow title="DNS 缓存" desc="independent_cache，减少重复查询">
                <button
                  type="button"
                  role="switch"
                  className={`switch ${dns.cache ? "on" : ""}`}
                  disabled={busy}
                  aria-checked={dns.cache}
                  onClick={() => patch({ cache: !dns.cache })}
                >
                  <span className="switch-thumb" />
                </button>
              </SettingRow>
              <SettingRow
                title="防泄漏"
                desc="优先按规则与 final 解析，避免静默回落"
              >
                <button
                  type="button"
                  role="switch"
                  className={`switch ${dns.leak_protect ? "on" : ""}`}
                  disabled={busy}
                  aria-checked={dns.leak_protect}
                  onClick={() => patch({ leak_protect: !dns.leak_protect })}
                >
                  <span className="switch-thumb" />
                </button>
              </SettingRow>
            </div>
          </div>
        </section>

        {/* —— Servers —— */}
        <section className="card dns-panel dns-cell dns-cell-servers">
          <header className="dns-panel-head">
            <h2>服务器</h2>
            <p>UDP / DoH / DoT / 系统 DNS</p>
          </header>

          <div className="dns-panel-body dns-panel-body--flush">
            <ul className="dns-list">
              {dns.servers.map((s) => (
                <li
                  key={s.id}
                  className={`dns-list-item${s.enabled ? "" : " off"}`}
                >
                  <button
                    type="button"
                    role="checkbox"
                    aria-checked={s.enabled}
                    className={`check ${s.enabled ? "on" : ""}`}
                    disabled={busy}
                    onClick={() => toggleServer(s.id)}
                    title="启用/禁用"
                  />
                  <div className="dns-list-body">
                    <div className="dns-list-title">
                      <span className="dns-list-name">{s.name}</span>
                      <span className={`pill role-${s.role}`}>
                        {roleLabel(s.role)}
                      </span>
                    </div>
                    <div className="dns-list-addr mono">{s.address}</div>
                  </div>
                  {s.role !== "local" && (
                    <button
                      type="button"
                      className="ghost danger small"
                      disabled={busy}
                      onClick={() => removeServer(s.id)}
                    >
                      删除
                    </button>
                  )}
                </li>
              ))}
            </ul>

            <div className="dns-add">
              <div className="dns-add-title">添加服务器</div>
              <div className="dns-add-grid">
                <input
                  placeholder="名称（可选）"
                  value={newServerName}
                  onChange={(e) => setNewServerName(e.target.value)}
                />
                <input
                  className="dns-add-wide"
                  placeholder="地址：223.5.5.5 / https://1.1.1.1/dns-query / tls://…"
                  value={newServerAddr}
                  onChange={(e) => setNewServerAddr(e.target.value)}
                />
                <select
                  value={newServerRole}
                  onChange={(e) =>
                    setNewServerRole(e.target.value as DnsServerRole)
                  }
                >
                  <option value="domestic">国内</option>
                  <option value="remote">远程</option>
                  <option value="custom">自定义</option>
                  <option value="local">系统</option>
                </select>
                <button
                  type="button"
                  className="secondary"
                  disabled={busy}
                  onClick={addServer}
                >
                  添加
                </button>
              </div>
            </div>
          </div>
        </section>

        {/* —— Rules —— */}
        <section className="card dns-panel dns-cell dns-cell-rules">
          <header className="dns-panel-head">
            <h2>白名单规则</h2>
            <p>内网 / 企业域名走指定解析器</p>
          </header>

          <div className="dns-panel-body dns-panel-body--flush">
            {dns.rules.length === 0 ? (
              <div className="dns-empty">暂无规则</div>
            ) : (
              <ul className="dns-list">
                {dns.rules.map((r) => (
                  <li
                    key={r.id}
                    className={`dns-list-item${r.enabled ? "" : " off"}`}
                  >
                    <button
                      type="button"
                      role="checkbox"
                      aria-checked={r.enabled}
                      className={`check ${r.enabled ? "on" : ""}`}
                      disabled={busy}
                      onClick={() => toggleRule(r.id)}
                    />
                    <div className="dns-list-body">
                      <div className="dns-list-title">
                        <span className="pill matcher-pill">
                          {matcherLabel(r.matcher)}
                        </span>
                        <span className="dns-list-name">{r.payload}</span>
                      </div>
                      <div className="dns-list-addr muted">
                        → {actionLabel(r.action)}
                      </div>
                    </div>
                    <button
                      type="button"
                      className="ghost danger small"
                      disabled={busy}
                      onClick={() => removeRule(r.id)}
                    >
                      删除
                    </button>
                  </li>
                ))}
              </ul>
            )}

            <div className="dns-add">
              <div className="dns-add-title">添加规则</div>
              <div className="dns-add-grid">
                <select
                  value={newRuleMatcher}
                  onChange={(e) =>
                    setNewRuleMatcher(e.target.value as DomainMatcher)
                  }
                >
                  <option value="domain_suffix">后缀</option>
                  <option value="domain">精确</option>
                  <option value="domain_keyword">关键字</option>
                </select>
                <input
                  className="dns-add-wide"
                  placeholder="company.com / git.internal"
                  value={newRulePayload}
                  onChange={(e) => setNewRulePayload(e.target.value)}
                />
                <select
                  value={newRuleAction}
                  onChange={(e) =>
                    setNewRuleAction(
                      e.target.value as "system" | "domestic" | "remote",
                    )
                  }
                >
                  <option value="system">系统 DNS</option>
                  <option value="domestic">国内 DNS</option>
                  <option value="remote">远程 DNS</option>
                </select>
                <button
                  type="button"
                  className="secondary"
                  disabled={busy}
                  onClick={addRule}
                >
                  添加
                </button>
              </div>
            </div>
          </div>
        </section>

        {/* —— FakeIP —— */}
        <section className="card dns-panel dns-cell dns-cell-fakeip">
          <header className="dns-panel-head">
            <h2>FakeIP</h2>
            <p>虚拟 IP，加速域名路由</p>
          </header>
          <div className="dns-panel-body">
            <SettingRow
              title="启用 FakeIP"
              desc="Smart / Custom 模式生效；System 模式下忽略"
            >
              <button
                type="button"
                role="switch"
                className={`switch ${dns.fake_ip.enabled ? "on" : ""}`}
                disabled={busy || mode === "system"}
                aria-checked={dns.fake_ip.enabled}
                onClick={() =>
                  void save({
                    ...dns,
                    fake_ip: {
                      ...dns.fake_ip,
                      enabled: !dns.fake_ip.enabled,
                    },
                  })
                }
              >
                <span className="switch-thumb" />
              </button>
            </SettingRow>

            <label className="field dns-field">
              <span>IPv4 地址池</span>
              <input
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

            <SettingRow title="IPv6 FakeIP" desc="需要时再开启">
              <button
                type="button"
                role="switch"
                className={`switch ${dns.fake_ip.inet6_enabled ? "on" : ""}`}
                disabled={busy}
                aria-checked={dns.fake_ip.inet6_enabled}
                onClick={() =>
                  void save({
                    ...dns,
                    fake_ip: {
                      ...dns.fake_ip,
                      inet6_enabled: !dns.fake_ip.inet6_enabled,
                    },
                  })
                }
              >
                <span className="switch-thumb" />
              </button>
            </SettingRow>

            <label className="field dns-field">
              <span>Bypass 后缀（每行一个，不走 FakeIP）</span>
              <textarea
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
            <h2>诊断</h2>
            <p>系统 DNS 解析测试</p>
          </header>
          <div className="dns-panel-body">
            <label className="field dns-field">
              <span>域名</span>
              <div className="dns-test-row">
                <input
                  value={testDomain}
                  onChange={(e) => setTestDomain(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void onTest();
                  }}
                />
                <button
                  type="button"
                  disabled={testBusy}
                  onClick={() => void onTest()}
                >
                  {testBusy ? "查询中…" : "测试"}
                </button>
              </div>
            </label>

            {testResult ? (
              <div
                className={`dns-test-card ${testResult.ok ? "ok" : "fail"}`}
              >
                <div className="dns-test-top">
                  <strong>{testResult.domain}</strong>
                  <span className="dns-test-badge">
                    {testResult.ok ? "成功" : "失败"}
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
              <div className="dns-empty soft">输入域名后点击测试</div>
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
