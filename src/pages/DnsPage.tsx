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
import { GlassButton } from "../components/GlassButton";
import { GlassSeg } from "../components/GlassSeg";
import { GlassSwitchControl } from "../components/GlassSwitchControl";
import { SolidSelect } from "../components/SolidSelect";
import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n/messages";
import type {
  DnsAction,
  DnsFinalStrategy,
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

function actionLabel(
  a: DnsAction,
  t: (key: MessageKey, vars?: Record<string, string | number>) => string,
): string {
  switch (a.kind) {
    case "local":
      return t("dns.actionLocal");
    case "domestic":
      return t("dns.actionDomestic");
    case "remote":
      return t("dns.actionRemote");
    case "block":
      return t("dns.actionBlock");
  }
}

function matcherLabel(
  m: DomainMatcher,
  t: (key: MessageKey, vars?: Record<string, string | number>) => string,
) {
  switch (m) {
    case "domain":
      return t("dns.matcherExact");
    case "domain_suffix":
      return t("dns.matcherSuffix");
    case "domain_keyword":
      return t("dns.matcherKeyword");
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
  const [newSetName, setNewSetName] = useState(t("dns.setNamePhDns"));
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
    if (!window.confirm(t("dns.deleteRuleConfirm"))) return;
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
      setError(t("dns.needMatch"));
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
    if (!window.confirm(t("hosts.deleteEntryConfirm"))) return;
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
      setError(t("hosts.needDomain"));
      return;
    }
    if (!addr) {
      setError(t("hosts.needIp"));
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
    setNewSetName(t("dns.setNamePhDns"));
    setNewSetOpen(true);
    setError(null);
  }

  async function createSet(e: FormEvent) {
    e.preventDefault();
    if (!dns) return;
    const name = newSetName.trim();
    if (!name) {
      setError(t("rules.needName"));
      return;
    }
    if (dns.rule_sets.some((set) => set.name.toLowerCase() === name.toLowerCase())) {
      setError(t("dns.dupSetName", { name }));
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
    if (!window.confirm(t("rules.deleteSetConfirm", { name: set.name }))) return;
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
    if (!window.confirm(t("dns.resetBuiltinConfirm"))) {
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
        {t("common.loading")}
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
            <h2>{t("dns.general")}</h2>
            <p>{t("dns.generalDesc")}</p>
          </header>

          <div className="dns-panel-body dns-general-body">
            <div className="dns-general-primary">
              <SettingRow
                title={t("dns.hijack")}
                desc={t("dns.hijackDesc")}
              >
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
              <SettingRow
                title={t("dns.leak")}
                desc={t("dns.leakDesc")}
              >
                <GlassSwitchControl
                  checked={dns.leak_protect}
                  title={t("dns.leak")}
                  disabled={busy}
                  onChange={(checked) => patch({ leak_protect: checked })}
                />
              </SettingRow>
            </div>
          </div>
        </section>}

        {section !== "settings" && <aside className="card ruleset-list dns-ruleset-nav dns-cell-ruleset-nav">
          <GlassButton icon="+" onClick={openNewSet} title={t("rules.newSetTitle")}>
            {t("rules.newSetTitle")}
          </GlassButton>
          <div className="ruleset-list-title">
            {t("dns.setListTitle")}
            <span className="ruleset-list-hint">{t("dns.setListHint")}</span>
          </div>
          {dns.rule_sets.map((set) => {
            const count =
              set.id === "system-hosts"
                ? t("dns.systemBadge")
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
                  <GlassSwitchControl
                    checked={set.enabled}
                    size="sm"
                    disabled={busy}
                    title={t("dns.toggleSetTooltip", { name: set.name })}
                    onClick={(e) => {
                      e.stopPropagation();
                    }}
                    onChange={() => toggleRuleSet(set.id)}
                  />
                </div>
                <div className="dns-ruleset-footer">
                  <span className="muted dns-ruleset-meta">
                    {set.read_only
                      ? `${set.enabled ? t("common.enabled") : t("common.disabled")} · ${t("dns.systemReadonlySuffix")}`
                      : set.enabled
                        ? `${t("common.enabled")} · ${set.kind === "dns" ? t("dns.legacyCompatSuffix") : t("dns.staticMapSuffix")}`
                        : t("common.disabled")}
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
                      ? t("dns.hostsReadonlyDesc")
                      : viewSet.kind === "dns"
                        ? t("dns.dnsSetDesc")
                        : t("dns.hostsSetDesc")}
                  </p>
                </div>
                <div className="header-actions">
                  {!viewSet.read_only && (
                    <GlassButton
                      variant="primary"
                      icon="+"
                      disabled={busy}
                      onClick={viewSet.kind === "dns" ? openAddRule : openAddHost}
                    >
                      {viewSet.kind === "dns" ? t("rules.addRule") : t("dns.addHostBtn")}
                    </GlassButton>
                  )}
                  {viewSet.id === "builtin-dns" && (
                    <GlassButton
                      icon="↺"
                      disabled={busy}
                      onClick={() => void onResetRules()}
                      title={t("dns.resetFactoryBtn")}
                    >
                      {t("dns.resetFactoryBtn")}
                    </GlassButton>
                  )}
                  <button
                    type="button"
                    className="ghost small"
                    disabled={busy || dns.rule_sets[0]?.id === viewSet.id}
                    onClick={() => void moveCurrentSet(-1)}
                    title={t("dns.raisePrio")}
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
                    title={t("dns.lowerPrio")}
                  >
                    ↓
                  </button>
                  {!viewSet.builtin && (
                    <GlassButton
                      variant="danger"
                      icon="⌫"
                      disabled={busy}
                      onClick={() => void deleteCurrentSet()}
                      title={t("rules.deleteSet")}
                    >
                      {t("rules.deleteSet")}
                    </GlassButton>
                  )}
                </div>
              </div>
            </header>

            <div className="dns-panel-body dns-panel-body--flush dns-rule-set-body">
              {viewSet.read_only ? (
                systemHostsBusy ? (
                  <div className="dns-empty soft">{t("dns.loadingSystemHosts")}</div>
                ) : systemHosts.length === 0 ? (
                  <div className="dns-empty soft">{t("dns.emptySystemHosts")}</div>
                ) : (
                  <ul className="dns-list">
                    {systemHosts.map((host) => (
                      <li key={host.id} className="dns-list-item">
                        <div className="dns-list-body">
                          <div className="dns-list-title">
                            <span className="pill matcher-pill">{t("dns.readonlyBadge")}</span>
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
                  <div className="dns-empty">{t("dns.emptyRules")}</div>
                ) : (
                  <ul className="dns-list">
                    {viewSet.dns_rules.map((rule) => (
                      <li
                        key={rule.id}
                        className={`dns-list-item${rule.enabled ? "" : " off"}`}
                        onClick={() => openEditRule(rule)}
                        title={t("dns.clickEditRule")}
                      >
                        <div className="dns-list-body">
                          <div className="dns-list-title">
                            <span className="pill matcher-pill">{matcherLabel(rule.matcher, t)}</span>
                            <span className="dns-list-name">{rule.payload}</span>
                          </div>
                          <div className="dns-list-addr muted">→ {actionLabel(rule.action, t)}</div>
                        </div>
                        <div className="dns-list-actions" onClick={(e) => e.stopPropagation()}>
                          <GlassSwitchControl
                            checked={rule.enabled}
                            size="sm"
                            title={t("dns.enableRuleTooltip")}
                            disabled={busy}
                            onChange={() => toggleRule(rule.id)}
                          />
                          <button
                            type="button"
                            className="rule-menu-trigger"
                            disabled={busy}
                            aria-label={t("dns.deleteRuleAria")}
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
                <div className="dns-empty">{t("dns.emptyHostEntries")}</div>
              ) : (
                <ul className="dns-list">
                  {viewSet.hosts.map((host) => (
                    <li
                      key={host.id}
                      className={`dns-list-item${host.enabled ? "" : " off"}`}
                      onClick={() => openEditHost(host)}
                      title={t("dns.clickEditHost")}
                    >
                      <div className="dns-list-body">
                        <div className="dns-list-title">
                          <span className="dns-list-name">{host.domain}</span>
                        </div>
                        <div className="dns-list-addr muted mono">→ {host.addr}</div>
                      </div>
                      <div className="dns-list-actions" onClick={(e) => e.stopPropagation()}>
                        <GlassSwitchControl
                          checked={host.enabled}
                          size="sm"
                          title={t("dns.enableHostTooltip")}
                          disabled={busy}
                          onChange={() => toggleHost(host.id)}
                        />
                        <button
                          type="button"
                          className="rule-menu-trigger"
                          disabled={busy}
                          aria-label={t("dns.deleteHostAria")}
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
        </section>}

        {section !== "rules" && <section className="card dns-panel dns-cell dns-cell-diag">
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
              <div
                className={`dns-test-card ${testResult.ok ? "ok" : "fail"}`}
              >
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
        </section>}
      </div>

      {newSetOpen && (
        <div
          className="modal-backdrop"
          onClick={() => !busy && setNewSetOpen(false)}
        >
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <header className="modal-header">
              <h2>{t("dns.newSetModalTitle")}</h2>
              <button
                type="button"
                className="icon-btn"
                disabled={busy}
                aria-label={t("common.close")}
                onClick={() => setNewSetOpen(false)}
              >
                ×
              </button>
            </header>
            <form className="modal-body" onSubmit={(e) => void createSet(e)}>
              <div className="field">
                <span>{t("dns.setKindLabel")}</span>
                <SolidSelect
                  value={newSetKind}
                  aria-label={t("dns.setKindLabel")}
                  options={[
                    { value: "dns", label: t("dns.setKindDns") },
                    { value: "hosts", label: t("dns.setKindHosts") },
                  ]}
                  onChange={(value) => {
                    const kind = value as DnsRuleSetKind;
                    setNewSetKind(kind);
                    setNewSetName(kind === "dns" ? t("dns.setNamePhDns") : t("dns.setNamePhHosts"));
                  }}
                />
              </div>
              <label className="field">
                <span>{t("dns.setNameFieldLabel")}</span>
                <input
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                  value={newSetName}
                  onChange={(e) => setNewSetName(e.target.value)}
                  placeholder={t("dns.setNamePlaceholder")}
                  autoFocus
                />
              </label>
              <footer className="modal-footer">
                <GlassButton disabled={busy} onClick={() => setNewSetOpen(false)}>
                  {t("common.cancel")}
                </GlassButton>
                <GlassButton
                  type="submit"
                  variant="primary"
                  disabled={busy || !newSetName.trim()}
                >
                  {busy ? t("rules.creating") : t("rules.create")}
                </GlassButton>
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
              <h2>{editRuleId ? t("dns.editRuleTitle") : t("dns.addRuleModalTitle")}</h2>
              <button
                type="button"
                className="icon-btn"
                disabled={busy}
                aria-label={t("common.close")}
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
                <span>{t("dns.matchTypeLabel")}</span>
                <SolidSelect
                  value={newRuleMatcher}
                  onChange={(v) => setNewRuleMatcher(v as DomainMatcher)}
                  aria-label={t("dns.matchTypeLabel")}
                  options={[
                    { value: "domain_suffix", label: t("dns.matcherSuffix") },
                    { value: "domain", label: t("dns.matcherExact") },
                    { value: "domain_keyword", label: t("dns.matcherKeyword") },
                  ]}
                />
              </div>
              <label className="field">
                <span>{t("dns.domainMatchLabel")}</span>
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
                <span>{t("dns.resolveActionLabel")}</span>
                <SolidSelect
                  value={newRuleAction}
                  onChange={(v) =>
                    setNewRuleAction(v as "local" | "domestic" | "remote")
                  }
                  aria-label={t("dns.resolveActionLabel")}
                  options={[
                    { value: "local", label: t("dns.actionLocal") },
                    { value: "domestic", label: t("dns.actionDomestic") },
                    { value: "remote", label: t("dns.actionRemote") },
                  ]}
                />
              </div>
              <label className="sys-proxy-row" style={{ border: "none", paddingTop: 0, marginTop: 0 }}>
                <span>{t("rules.enabled")}</span>
                <GlassSwitchControl
                  checked={editRuleEnabled}
                  title={t("rules.enabled")}
                  onChange={setEditRuleEnabled}
                />
              </label>
              <footer className="modal-footer">
                <GlassButton disabled={busy} onClick={resetRuleForm}>
                  {t("common.cancel")}
                </GlassButton>
                <GlassButton
                  type="submit"
                  variant="primary"
                  disabled={busy || !newRulePayload.trim()}
                >
                  {busy ? t("common.saving") : editRuleId ? t("common.save") : t("common.add")}
                </GlassButton>
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
              <h2>{editHostId ? t("dns.editHostsTitle") : t("dns.addHostsTitle")}</h2>
              <button
                type="button"
                className="icon-btn"
                disabled={busy}
                aria-label={t("common.close")}
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
                <span>{t("dns.domainLabel")}</span>
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
                <span>{t("dns.ipLabel")}</span>
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
                <span>{t("rules.enabled")}</span>
                <GlassSwitchControl
                  checked={editHostEnabled}
                  title={t("rules.enabled")}
                  onChange={setEditHostEnabled}
                />
              </label>
              <footer className="modal-footer">
                <GlassButton disabled={busy} onClick={resetHostForm}>
                  {t("common.cancel")}
                </GlassButton>
                <GlassButton
                  type="submit"
                  variant="primary"
                  disabled={busy || !newHostDomain.trim() || !newHostAddr.trim()}
                >
                  {busy ? t("common.saving") : editHostId ? t("common.save") : t("common.add")}
                </GlassButton>
              </footer>
            </form>
          </div>
        </div>
      )}
    </div>
  );
}
