import {
  useCallback,
  useEffect,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";
import {
  getDnsSettings,
  readSystemHosts,
  resetDnsDefaults,
  testDnsLookup,
  updateDnsSettings,
} from "../api";
import { GlassSeg } from "../components/GlassSeg";
import { SolidSelect } from "../components/SolidSelect";
import { useI18n } from "../i18n";
import type {
  DnsAction,
  DnsFinalStrategy,
  DnsMode,
  DnsRule,
  DnsRuleSet,
  DnsRuleSetKind,
  DnsSettings,
  DnsTestResult,
  DomainMatcher,
  HostsEntry,
} from "../types";

function newId(prefix: string) {
  return `${prefix}-${Math.random().toString(36).slice(2, 10)}`;
}

function actionLabel(a: DnsAction): string {
  switch (a.kind) {
    case "local":
      return "本地 DNS";
    case "domestic":
      return "国内 DNS";
    case "remote":
      return "远程 DNS";
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

const MODE_HINTS: Record<DnsMode, string> = {
  local: "默认使用系统解析；开启 DNS 规则后，命中的域名可改走指定解析器",
  smart_local: "办公网建议使用（直连域名走本地 DNS，其余走远程）",
  smart_cn: "使用国内公共 DNS 解析，办公网不建议使用（直连域名走国内 DNS）",
};

const MODE_LABELS: Record<DnsMode, string> = {
  local: "本地",
  smart_local: "优先本地",
  smart_cn: "优先国内",
};

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
  /** Render all content, DNS options only, or rule sets only. */
  section?: "all" | "settings" | "rules";
}

export function DnsPage({ embedded = false, section = "all" }: Props) {
  const { t } = useI18n();
  const [dns, setDns] = useState<DnsSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const [testDomain, setTestDomain] = useState("www.baidu.com");
  const [testResult, setTestResult] = useState<DnsTestResult | null>(null);
  const [testBusy, setTestBusy] = useState(false);

  const [newRulePayload, setNewRulePayload] = useState("");
  const [newRuleMatcher, setNewRuleMatcher] =
    useState<DomainMatcher>("domain_suffix");
  const [newRuleAction, setNewRuleAction] = useState<
    "local" | "domestic" | "remote"
  >("local");
  const [editRuleId, setEditRuleId] = useState<string | null>(null);
  const [editRuleEnabled, setEditRuleEnabled] = useState(true);
  const [ruleFormOpen, setRuleFormOpen] = useState(false);

  // Hosts feature state.
  const [newHostDomain, setNewHostDomain] = useState("");
  const [newHostAddr, setNewHostAddr] = useState("");
  const [editHostId, setEditHostId] = useState<string | null>(null);
  const [editHostEnabled, setEditHostEnabled] = useState(true);
  const [hostFormOpen, setHostFormOpen] = useState(false);
  const [viewSetId, setViewSetId] = useState<string | null>(null);
  const [newSetOpen, setNewSetOpen] = useState(false);
  const [newSetName, setNewSetName] = useState("自定义 DNS 规则");
  const [newSetKind, setNewSetKind] = useState<DnsRuleSetKind>("dns");
  const [systemHosts, setSystemHosts] = useState<HostsEntry[]>([]);
  const [systemHostsBusy, setSystemHostsBusy] = useState(false);

  const [bypassText, setBypassText] = useState("");

  const reload = useCallback(async () => {
    setError(null);
    try {
      const s = await getDnsSettings();
      setDns(s);
      setViewSetId((current) =>
        current && s.rule_sets.some((set) => set.id === current)
          ? current
          : (s.rule_sets[0]?.id ?? null),
      );
      setBypassText((s.fake_ip.bypass || []).join("\n"));
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // The system Hosts set is always viewable but its entries are read-only.
  useEffect(() => {
    if (viewSetId !== "system-hosts") {
      setSystemHosts([]);
      return;
    }
    let cancelled = false;
    setSystemHostsBusy(true);
    readSystemHosts()
      .then((entries) => {
        if (!cancelled) setSystemHosts(entries);
      })
      .catch(() => {
        if (!cancelled) setSystemHosts([]);
      })
      .finally(() => {
        if (!cancelled) setSystemHostsBusy(false);
      });
    return () => {
      cancelled = true;
    };
  }, [viewSetId]);

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

  function setMode(mode: DnsMode) {
    if (!dns) return;
    void save({ ...dns, mode });
  }

  function withUpdatedSet(
    setId: string,
    update: (set: DnsRuleSet) => DnsRuleSet,
  ): DnsSettings | null {
    if (!dns) return null;
    return {
      ...dns,
      rule_sets: dns.rule_sets.map((set) =>
        set.id === setId ? update(set) : set,
      ),
    };
  }

  function toggleRuleSet(setId: string) {
    const next = withUpdatedSet(setId, (set) => ({
      ...set,
      enabled: !set.enabled,
    }));
    if (next) void save(next);
  }

  function toggleRule(id: string) {
    if (!viewSetId) return;
    const next = withUpdatedSet(viewSetId, (set) => ({
      ...set,
      dns_rules: set.dns_rules.map((r) =>
        r.id === id ? { ...r, enabled: !r.enabled } : r,
      ),
    }));
    if (next) void save(next);
  }

  function removeRule(id: string) {
    if (!viewSetId) return;
    if (!window.confirm("删除该 DNS 规则？")) return;
    if (editRuleId === id) resetRuleForm();
    const next = withUpdatedSet(viewSetId, (set) => ({
      ...set,
      dns_rules: set.dns_rules.filter((r) => r.id !== id),
    }));
    if (next) void save(next);
  }

  function resetRuleForm() {
    setRuleFormOpen(false);
    setEditRuleId(null);
    setNewRulePayload("");
    setNewRuleMatcher("domain_suffix");
    setNewRuleAction("local");
    setEditRuleEnabled(true);
  }

  function openAddRule() {
    resetRuleForm();
    resetHostForm();
    setRuleFormOpen(true);
  }

  function openEditRule(r: DnsRule) {
    resetHostForm();
    setRuleFormOpen(true);
    setEditRuleId(r.id);
    setNewRulePayload(r.payload);
    setNewRuleMatcher(r.matcher);
    const k = r.action.kind;
    setNewRuleAction(
      k === "domestic" || k === "remote" ? k : "local",
    );
    setEditRuleEnabled(r.enabled);
  }

  async function saveRuleForm() {
    if (!dns || !viewSetId) return;
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
          : { kind: "local" };
    if (editRuleId) {
      const next = withUpdatedSet(viewSetId, (set) => ({
        ...set,
        dns_rules: set.dns_rules.map((r) =>
          r.id === editRuleId
            ? {
                ...r,
                enabled: editRuleEnabled,
                matcher: newRuleMatcher,
                payload,
                action,
              }
            : r,
        ),
      }));
      const saved = next ? await save(next) : false;
      if (saved) resetRuleForm();
      return;
    }
    const r: DnsRule = {
      id: newId("rule"),
      enabled: editRuleEnabled,
      matcher: newRuleMatcher,
      payload,
      action,
    };
    const next = withUpdatedSet(viewSetId, (set) => ({
      ...set,
      dns_rules: [...set.dns_rules, r],
    }));
    const saved = next ? await save(next) : false;
    if (saved) resetRuleForm();
  }

  // —— Hosts handlers ——
  function toggleHost(id: string) {
    if (!viewSetId) return;
    const next = withUpdatedSet(viewSetId, (set) => ({
      ...set,
      hosts: set.hosts.map((h) =>
        h.id === id ? { ...h, enabled: !h.enabled } : h,
      ),
    }));
    if (next) void save(next);
  }

  function removeHost(id: string) {
    if (!viewSetId) return;
    if (!window.confirm("删除该 Host 条目？")) return;
    if (editHostId === id) resetHostForm();
    const next = withUpdatedSet(viewSetId, (set) => ({
      ...set,
      hosts: set.hosts.filter((h) => h.id !== id),
    }));
    if (next) void save(next);
  }

  function resetHostForm() {
    setHostFormOpen(false);
    setEditHostId(null);
    setNewHostDomain("");
    setNewHostAddr("");
    setEditHostEnabled(true);
  }

  function openAddHost() {
    resetHostForm();
    resetRuleForm();
    setHostFormOpen(true);
  }

  function openEditHost(h: HostsEntry) {
    resetRuleForm();
    setHostFormOpen(true);
    setEditHostId(h.id);
    setNewHostDomain(h.domain);
    setNewHostAddr(h.addr);
    setEditHostEnabled(h.enabled);
  }

  async function saveHostForm() {
    if (!dns || !viewSetId) return;
    const domain = newHostDomain.trim().toLowerCase();
    const addr = newHostAddr.trim();
    if (!domain) {
      setError("请填写域名");
      return;
    }
    if (!addr) {
      setError("请填写 IP 地址");
      return;
    }
    if (editHostId) {
      const next = withUpdatedSet(viewSetId, (set) => ({
        ...set,
        hosts: set.hosts.map((h) =>
          h.id === editHostId
            ? { ...h, enabled: editHostEnabled, domain, addr }
            : h,
        ),
      }));
      const saved = next ? await save(next) : false;
      if (saved) resetHostForm();
      return;
    }
    const h: HostsEntry = {
      id: newId("host"),
      enabled: editHostEnabled,
      domain,
      addr,
    };
    const next = withUpdatedSet(viewSetId, (set) => ({
      ...set,
      hosts: [...set.hosts, h],
    }));
    const saved = next ? await save(next) : false;
    if (saved) resetHostForm();
  }

  function openNewSet() {
    setNewSetKind("dns");
    setNewSetName("自定义 DNS 规则");
    setNewSetOpen(true);
    setError(null);
  }

  async function createSet(e: FormEvent) {
    e.preventDefault();
    if (!dns) return;
    const name = newSetName.trim();
    if (!name) {
      setError("请输入规则集名称");
      return;
    }
    if (dns.rule_sets.some((set) => set.name.toLowerCase() === name.toLowerCase())) {
      setError(`已存在同名规则集「${name}」`);
      return;
    }
    const set: DnsRuleSet = {
      id: newId(newSetKind === "dns" ? "dns-set" : "hosts-set"),
      name,
      kind: newSetKind,
      builtin: false,
      read_only: false,
      enabled: true,
      dns_rules: [],
      hosts: [],
    };
    const saved = await save({ ...dns, rule_sets: [...dns.rule_sets, set] });
    if (saved) {
      setViewSetId(set.id);
      setNewSetOpen(false);
    }
  }

  async function deleteCurrentSet() {
    if (!dns || !viewSetId) return;
    const set = dns.rule_sets.find((item) => item.id === viewSetId);
    if (!set || set.builtin) return;
    if (!window.confirm(`删除规则集「${set.name}」？`)) return;
    const nextSets = dns.rule_sets.filter((item) => item.id !== viewSetId);
    const saved = await save({ ...dns, rule_sets: nextSets });
    if (saved) setViewSetId(nextSets[0]?.id ?? null);
  }

  async function moveCurrentSet(direction: -1 | 1) {
    if (!dns || !viewSetId) return;
    const index = dns.rule_sets.findIndex((set) => set.id === viewSetId);
    const target = index + direction;
    if (index < 0 || target < 0 || target >= dns.rule_sets.length) return;
    const nextSets = [...dns.rule_sets];
    const [moved] = nextSets.splice(index, 1);
    nextSets.splice(target, 0, moved);
    await save({ ...dns, rule_sets: nextSets });
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

  async function onResetRules() {
    if (
      !window.confirm(
        `将「内置 DNS 规则」恢复为出厂内容？\n当前对该规则集的修改会丢失，其它规则集不受影响。`,
      )
    ) {
      return;
    }
    setBusy(true);
    setError(null);
    resetRuleForm();
    try {
      const s = await resetDnsDefaults("rules", true);
      setDns(s);
      setBypassText((s.fake_ip.bypass || []).join("\n"));
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setBusy(false);
    }
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

  const mode = dns.mode;
  const viewSet =
    dns.rule_sets.find((set) => set.id === viewSetId) ?? dns.rule_sets[0] ?? null;
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

      <div className={`dns-stack dns-grid dns-section-${section}`}>
        {/* —— General —— */}
        {section !== "rules" && <section className="card dns-panel dns-cell dns-cell-general">
          <header className="dns-panel-head">
            <h2>常规</h2>
            <p>解析模式与全局行为</p>
          </header>

          <div className="dns-panel-body dns-general-body">
            <div className="dns-general-primary">
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

              <div className="dns-mode-block">
                <div className="dns-mode-label">解析模式</div>
                <GlassSeg
                  value={mode}
                  ariaLabel="解析模式"
                  disabled={busy}
                  onChange={(v) => setMode(v as DnsMode)}
                  options={[
                    { value: "local", label: "本地" },
                    { value: "smart_local", label: "优先本地" },
                    { value: "smart_cn", label: "优先国内" },
                  ]}
                />
                <p className="dns-mode-hint">{MODE_HINTS[mode]}</p>
              </div>

              <SettingRow
                title="兜底 DNS"
                desc="未命中规则的网站走兜底 DNS 解析，国外网站优先选择远程"
              >
                <GlassSeg
                  value={dns.dns_final}
                  ariaLabel="兜底 DNS"
                  disabled={busy}
                  onChange={(v) => patch({ dns_final: v as DnsFinalStrategy })}
                  options={[
                    { value: "local", label: "本地" },
                    { value: "domestic", label: "国内" },
                    { value: "remote", label: "远程" },
                  ]}
                />
              </SettingRow>
            </div>

            <div className="dns-general-toggles">
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
        </section>}

        {section !== "settings" && <aside className="card ruleset-list dns-ruleset-nav dns-cell-ruleset-nav">
          <button type="button" className="secondary" onClick={openNewSet}>
            新建规则集
          </button>
          <div className="ruleset-list-title">
            DNS 规则集
            <span className="ruleset-list-hint">顺序即匹配优先级</span>
          </div>
          {dns.rule_sets.map((set) => {
            const count =
              set.id === "system-hosts"
                ? "系统"
                : set.kind === "dns"
                  ? set.dns_rules.length
                  : set.hosts.length;
            return (
              <div
                key={set.id}
                className={`ruleset-item${viewSet?.id === set.id ? " selected" : ""}`}
                role="button"
                tabIndex={0}
                aria-current={viewSet?.id === set.id ? "page" : undefined}
                onClick={() => setViewSetId(set.id)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") setViewSetId(set.id);
                }}
              >
                <div className="ruleset-item-top">
                  <span className="ruleset-name">{set.name}</span>
                  <button
                    type="button"
                    role="switch"
                    className={`switch small ${set.enabled ? "on" : ""}`}
                    disabled={busy}
                    aria-checked={set.enabled}
                    title={`启用 / 禁用 ${set.name}`}
                    onClick={(e) => {
                      e.stopPropagation();
                      toggleRuleSet(set.id);
                    }}
                  >
                    <span className="switch-thumb" />
                  </button>
                </div>
                <div className="dns-ruleset-footer">
                  <span className="muted dns-ruleset-meta">
                    {set.read_only
                      ? `${set.enabled ? "已启用" : "未启用"} · 系统只读`
                      : set.enabled
                        ? `已启用 · ${set.kind === "dns" ? `叠加到${MODE_LABELS[mode]}` : "静态映射"}`
                        : "未启用"}
                  </span>
                  <span className="pill matcher-pill dns-ruleset-type">
                    {set.kind === "dns" ? "DNS" : "HOSTS"} · {count}
                  </span>
                </div>
              </div>
            );
          })}
        </aside>}

        {section !== "settings" && viewSet && (
          <section className="card dns-panel dns-cell dns-cell-rules">
            <header className="dns-panel-head">
              <div className="dns-panel-head-row">
                <div>
                  <h2>{viewSet.name}</h2>
                  <p>
                    {viewSet.read_only
                      ? "读取操作系统 Hosts 文件，仅供查看"
                      : viewSet.kind === "dns"
                        ? "域名匹配后指定本地、国内或远程解析"
                        : "域名 → IP 静态映射，优先级高于 DNS 规则"}
                  </p>
                </div>
                <div className="header-actions">
                  {!viewSet.read_only && (
                    <button
                      type="button"
                      disabled={busy}
                      onClick={viewSet.kind === "dns" ? openAddRule : openAddHost}
                    >
                      {viewSet.kind === "dns" ? "添加规则" : "添加 Host"}
                    </button>
                  )}
                  {viewSet.id === "builtin-dns" && (
                    <button
                      type="button"
                      className="secondary"
                      disabled={busy}
                      onClick={() => void onResetRules()}
                    >
                      恢复出厂
                    </button>
                  )}
                  <button
                    type="button"
                    className="ghost small"
                    disabled={busy || dns.rule_sets[0]?.id === viewSet.id}
                    onClick={() => void moveCurrentSet(-1)}
                    title="提高优先级"
                  >
                    ↑
                  </button>
                  <button
                    type="button"
                    className="ghost small"
                    disabled={
                      busy ||
                      dns.rule_sets[dns.rule_sets.length - 1]?.id === viewSet.id
                    }
                    onClick={() => void moveCurrentSet(1)}
                    title="降低优先级"
                  >
                    ↓
                  </button>
                  {!viewSet.builtin && (
                    <button
                      type="button"
                      className="danger"
                      disabled={busy}
                      onClick={() => void deleteCurrentSet()}
                    >
                      删除集
                    </button>
                  )}
                </div>
              </div>
            </header>

            <div className="dns-panel-body dns-panel-body--flush dns-rule-set-body">
              {viewSet.read_only ? (
                systemHostsBusy ? (
                  <div className="dns-empty soft">正在读取系统 Hosts…</div>
                ) : systemHosts.length === 0 ? (
                  <div className="dns-empty soft">系统 Hosts 中没有可用条目</div>
                ) : (
                  <ul className="dns-list">
                    {systemHosts.map((host) => (
                      <li key={host.id} className="dns-list-item">
                        <div className="dns-list-body">
                          <div className="dns-list-title">
                            <span className="pill matcher-pill">只读</span>
                            <span className="dns-list-name">{host.domain}</span>
                          </div>
                          <div className="dns-list-addr muted mono">→ {host.addr}</div>
                        </div>
                      </li>
                    ))}
                  </ul>
                )
              ) : viewSet.kind === "dns" ? (
                viewSet.dns_rules.length === 0 ? (
                  <div className="dns-empty">暂无 DNS 规则</div>
                ) : (
                  <ul className="dns-list">
                    {viewSet.dns_rules.map((rule) => (
                      <li
                        key={rule.id}
                        className={`dns-list-item${rule.enabled ? "" : " off"}`}
                        onClick={() => openEditRule(rule)}
                        title="点击编辑规则"
                      >
                        <div className="dns-list-body">
                          <div className="dns-list-title">
                            <span className="pill matcher-pill">{matcherLabel(rule.matcher)}</span>
                            <span className="dns-list-name">{rule.payload}</span>
                          </div>
                          <div className="dns-list-addr muted">→ {actionLabel(rule.action)}</div>
                        </div>
                        <div className="dns-list-actions" onClick={(e) => e.stopPropagation()}>
                          <button
                            type="button"
                            role="switch"
                            aria-checked={rule.enabled}
                            className={`switch small ${rule.enabled ? "on" : ""}`}
                            disabled={busy}
                            onClick={() => toggleRule(rule.id)}
                          >
                            <span className="switch-thumb" />
                          </button>
                          <button
                            type="button"
                            className="rule-menu-trigger"
                            disabled={busy}
                            aria-label="删除规则"
                            onClick={() => removeRule(rule.id)}
                          >
                            ×
                          </button>
                        </div>
                      </li>
                    ))}
                  </ul>
                )
              ) : viewSet.hosts.length === 0 ? (
                <div className="dns-empty">暂无 Hosts 条目</div>
              ) : (
                <ul className="dns-list">
                  {viewSet.hosts.map((host) => (
                    <li
                      key={host.id}
                      className={`dns-list-item${host.enabled ? "" : " off"}`}
                      onClick={() => openEditHost(host)}
                      title="点击编辑 Host"
                    >
                      <div className="dns-list-body">
                        <div className="dns-list-title">
                          <span className="dns-list-name">{host.domain}</span>
                        </div>
                        <div className="dns-list-addr muted mono">→ {host.addr}</div>
                      </div>
                      <div className="dns-list-actions" onClick={(e) => e.stopPropagation()}>
                        <button
                          type="button"
                          role="switch"
                          aria-checked={host.enabled}
                          className={`switch small ${host.enabled ? "on" : ""}`}
                          disabled={busy}
                          onClick={() => toggleHost(host.id)}
                        >
                          <span className="switch-thumb" />
                        </button>
                        <button
                          type="button"
                          className="rule-menu-trigger"
                          disabled={busy}
                          aria-label="删除 Host"
                          onClick={() => removeHost(host.id)}
                        >
                          ×
                        </button>
                      </div>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </section>
        )}

        {/* —— FakeIP —— */}
        {section !== "rules" && <section className="card dns-panel dns-cell dns-cell-fakeip">
          <header className="dns-panel-head">
            <h2>FakeIP</h2>
            <p>虚拟 IP，加速域名路由</p>
          </header>
          <div className="dns-panel-body">
            <SettingRow
              title="启用 FakeIP"
              desc="非「本地」模式生效；本地模式下忽略"
            >
              <button
                type="button"
                role="switch"
                className={`switch ${dns.fake_ip.enabled ? "on" : ""}`}
                disabled={busy || mode === "local"}
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
        </section>}

        {section !== "rules" && <section className="card dns-panel dns-cell dns-cell-diag">
          <header className="dns-panel-head">
            <h2>诊断</h2>
            <p>系统 DNS 解析测试</p>
          </header>
          <div className="dns-panel-body">
            <label className="field dns-field">
              <span>域名</span>
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
        </section>}
      </div>

      {newSetOpen && (
        <div
          className="modal-backdrop"
          onClick={() => !busy && setNewSetOpen(false)}
        >
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <header className="modal-header">
              <h2>新建 DNS 规则集</h2>
              <button
                type="button"
                className="icon-btn"
                disabled={busy}
                aria-label="关闭"
                onClick={() => setNewSetOpen(false)}
              >
                ×
              </button>
            </header>
            <form className="modal-body" onSubmit={(e) => void createSet(e)}>
              <div className="field">
                <span>规则集类型</span>
                <SolidSelect
                  value={newSetKind}
                  aria-label="规则集类型"
                  options={[
                    { value: "dns", label: "DNS 规则" },
                    { value: "hosts", label: "Hosts 映射" },
                  ]}
                  onChange={(value) => {
                    const kind = value as DnsRuleSetKind;
                    setNewSetKind(kind);
                    setNewSetName(kind === "dns" ? "自定义 DNS 规则" : "自定义 Hosts");
                  }}
                />
              </div>
              <label className="field">
                <span>规则集名称</span>
                <input
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                  value={newSetName}
                  onChange={(e) => setNewSetName(e.target.value)}
                  placeholder="请输入名称"
                  autoFocus
                />
              </label>
              <footer className="modal-footer">
                <button
                  type="button"
                  className="secondary"
                  disabled={busy}
                  onClick={() => setNewSetOpen(false)}
                >
                  取消
                </button>
                <button type="submit" disabled={busy || !newSetName.trim()}>
                  {busy ? "创建中…" : "创建"}
                </button>
              </footer>
            </form>
          </div>
        </div>
      )}

      {ruleFormOpen && (
        <div
          className="modal-backdrop"
          onClick={() => !busy && resetRuleForm()}
        >
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <header className="modal-header">
              <h2>{editRuleId ? "编辑 DNS 规则" : "添加 DNS 规则"}</h2>
              <button
                type="button"
                className="icon-btn"
                disabled={busy}
                aria-label="关闭"
                onClick={resetRuleForm}
              >
                ×
              </button>
            </header>
            <form
              className="modal-body"
              onSubmit={(e) => {
                e.preventDefault();
                void saveRuleForm();
              }}
            >
              <div className="field">
                <span>匹配类型</span>
                <SolidSelect
                  value={newRuleMatcher}
                  onChange={(v) => setNewRuleMatcher(v as DomainMatcher)}
                  aria-label="匹配类型"
                  options={[
                    { value: "domain_suffix", label: "后缀" },
                    { value: "domain", label: "精确" },
                    { value: "domain_keyword", label: "关键字" },
                  ]}
                />
              </div>
              <label className="field">
                <span>域名匹配</span>
                <input
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                  value={newRulePayload}
                  onChange={(e) => setNewRulePayload(e.target.value)}
                  placeholder="company.com / git.internal"
                  autoFocus
                />
              </label>
              <div className="field">
                <span>解析动作</span>
                <SolidSelect
                  value={newRuleAction}
                  onChange={(v) =>
                    setNewRuleAction(v as "local" | "domestic" | "remote")
                  }
                  aria-label="解析动作"
                  options={[
                    { value: "local", label: "本地 DNS" },
                    { value: "domestic", label: "国内 DNS" },
                    { value: "remote", label: "远程 DNS" },
                  ]}
                />
              </div>
              <label className="sys-proxy-row" style={{ border: "none", paddingTop: 0, marginTop: 0 }}>
                <span>启用</span>
                <button
                  type="button"
                  role="switch"
                  aria-checked={editRuleEnabled}
                  className={`switch ${editRuleEnabled ? "on" : ""}`}
                  onClick={() => setEditRuleEnabled((v) => !v)}
                >
                  <span className="switch-thumb" />
                </button>
              </label>
              <footer className="modal-footer">
                <button
                  type="button"
                  className="secondary"
                  disabled={busy}
                  onClick={resetRuleForm}
                >
                  取消
                </button>
                <button type="submit" disabled={busy || !newRulePayload.trim()}>
                  {busy ? "保存中…" : editRuleId ? "保存" : "添加"}
                </button>
              </footer>
            </form>
          </div>
        </div>
      )}

      {hostFormOpen && (
        <div
          className="modal-backdrop"
          onClick={() => !busy && resetHostForm()}
        >
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <header className="modal-header">
              <h2>{editHostId ? "编辑 Hosts" : "添加 Hosts"}</h2>
              <button
                type="button"
                className="icon-btn"
                disabled={busy}
                aria-label="关闭"
                onClick={resetHostForm}
              >
                ×
              </button>
            </header>
            <form
              className="modal-body"
              onSubmit={(e) => {
                e.preventDefault();
                void saveHostForm();
              }}
            >
              <label className="field">
                <span>域名</span>
                <input
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                  value={newHostDomain}
                  onChange={(e) => setNewHostDomain(e.target.value)}
                  placeholder="example.com"
                  autoFocus
                />
              </label>
              <label className="field">
                <span>IP 地址</span>
                <input
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                  value={newHostAddr}
                  onChange={(e) => setNewHostAddr(e.target.value)}
                  placeholder="10.0.0.1 / ::1"
                />
              </label>
              <label className="sys-proxy-row" style={{ border: "none", paddingTop: 0, marginTop: 0 }}>
                <span>启用</span>
                <button
                  type="button"
                  role="switch"
                  aria-checked={editHostEnabled}
                  className={`switch ${editHostEnabled ? "on" : ""}`}
                  onClick={() => setEditHostEnabled((v) => !v)}
                >
                  <span className="switch-thumb" />
                </button>
              </label>
              <footer className="modal-footer">
                <button
                  type="button"
                  className="secondary"
                  disabled={busy}
                  onClick={resetHostForm}
                >
                  取消
                </button>
                <button
                  type="submit"
                  disabled={busy || !newHostDomain.trim() || !newHostAddr.trim()}
                >
                  {busy ? "保存中…" : editHostId ? "保存" : "添加"}
                </button>
              </footer>
            </form>
          </div>
        </div>
      )}
    </div>
  );
}
