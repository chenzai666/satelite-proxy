import { useCallback, useEffect, useMemo, useState, type FormEvent } from "react";
import { getDnsSettings, readSystemHosts, updateDnsSettings } from "../api";
import { GlassButton } from "../components/GlassButton";
import { GlassSwitchControl } from "../components/GlassSwitchControl";
import type { DnsRuleSet, DnsSettings, HostsEntry } from "../types";
import { useI18n } from "../i18n";

const SYSTEM_HOSTS_ID = "system-hosts";

function newId(prefix: string) {
  return `${prefix}-${Math.random().toString(36).slice(2, 10)}`;
}

function isIpLiteral(value: string) {
  const ipv4 = value.split(".");
  if (
    ipv4.length === 4 &&
    ipv4.every((part) => /^\d{1,3}$/.test(part) && Number(part) <= 255)
  ) return true;
  return value.includes(":") && /^[0-9a-f:.]+$/i.test(value);
}

export function HostsPage({ embedded = false }: { embedded?: boolean }) {
  const { t } = useI18n();
  const [dns, setDns] = useState<DnsSettings | null>(null);
  const [viewSetId, setViewSetId] = useState<string | null>(null);
  const [systemHosts, setSystemHosts] = useState<HostsEntry[]>([]);
  const [systemBusy, setSystemBusy] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [newSetOpen, setNewSetOpen] = useState(false);
  const [newSetName, setNewSetName] = useState(t("hosts.setNamePh"));
  const [entryOpen, setEntryOpen] = useState(false);
  const [editEntryId, setEditEntryId] = useState<string | null>(null);
  const [domain, setDomain] = useState("");
  const [addr, setAddr] = useState("");
  const [entryEnabled, setEntryEnabled] = useState(true);

  const hostSets = useMemo(
    () => dns?.rule_sets.filter((set) => set.kind === "hosts") ?? [],
    [dns],
  );
  const viewSet =
    hostSets.find((set) => set.id === viewSetId) ?? hostSets[0] ?? null;

  const reload = useCallback(async () => {
    setError(null);
    try {
      const settings = await getDnsSettings();
      const sets = settings.rule_sets.filter((set) => set.kind === "hosts");
      setDns(settings);
      setViewSetId((current) =>
        current && sets.some((set) => set.id === current)
          ? current
          : (sets[0]?.id ?? null),
      );
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  useEffect(() => {
    if (viewSetId !== SYSTEM_HOSTS_ID) {
      setSystemHosts([]);
      return;
    }
    let cancelled = false;
    setSystemBusy(true);
    readSystemHosts()
      .then((entries) => {
        if (!cancelled) setSystemHosts(entries);
      })
      .catch((err) => {
        if (!cancelled) setError(String(err));
      })
      .finally(() => {
        if (!cancelled) setSystemBusy(false);
      });
    return () => {
      cancelled = true;
    };
  }, [viewSetId]);

  async function save(next: DnsSettings) {
    setBusy(true);
    setError(null);
    try {
      const saved = await updateDnsSettings(next, true);
      setDns(saved);
      return true;
    } catch (err) {
      setError(String(err));
      return false;
    } finally {
      setBusy(false);
    }
  }

  function updateSet(setId: string, update: (set: DnsRuleSet) => DnsRuleSet) {
    if (!dns) return null;
    return {
      ...dns,
      rule_sets: dns.rule_sets.map((set) =>
        set.id === setId ? update(set) : set,
      ),
    };
  }

  function toggleSet(setId: string) {
    const next = updateSet(setId, (set) => ({ ...set, enabled: !set.enabled }));
    if (next) void save(next);
  }

  async function createSet(e: FormEvent) {
    e.preventDefault();
    if (!dns || busy) return;
    const name = newSetName.trim();
    if (!name) return setError(t("hosts.needSetName"));
    if (hostSets.some((set) => set.name.toLowerCase() === name.toLowerCase())) {
      return setError(t("hosts.dupSetName", { name }));
    }
    const set: DnsRuleSet = {
      id: newId("hosts-set"),
      name,
      kind: "hosts",
      builtin: false,
      read_only: false,
      enabled: true,
      dns_rules: [],
      hosts: [],
    };
    if (await save({ ...dns, rule_sets: [...dns.rule_sets, set] })) {
      setViewSetId(set.id);
      setNewSetOpen(false);
    }
  }

  async function deleteSet() {
    if (!dns || !viewSet || viewSet.builtin || busy) return;
    if (!confirm(t("hosts.deleteSetConfirm", { name: viewSet.name }))) return;
    const remaining = dns.rule_sets.filter((set) => set.id !== viewSet.id);
    if (await save({ ...dns, rule_sets: remaining })) {
      const next = remaining.find((set) => set.kind === "hosts");
      setViewSetId(next?.id ?? null);
    }
  }

  async function moveSet(direction: -1 | 1) {
    if (!dns || !viewSet || busy) return;
    const ids = hostSets.map((set) => set.id);
    const index = ids.indexOf(viewSet.id);
    const target = index + direction;
    if (index < 0 || target < 0 || target >= ids.length) return;
    const full = [...dns.rule_sets];
    const left = full.findIndex((set) => set.id === ids[index]);
    const right = full.findIndex((set) => set.id === ids[target]);
    [full[left], full[right]] = [full[right], full[left]];
    await save({ ...dns, rule_sets: full });
  }

  function openAddEntry() {
    setEditEntryId(null);
    setDomain("");
    setAddr("");
    setEntryEnabled(true);
    setEntryOpen(true);
  }

  function openEditEntry(entry: HostsEntry) {
    setEditEntryId(entry.id);
    setDomain(entry.domain);
    setAddr(entry.addr);
    setEntryEnabled(entry.enabled);
    setEntryOpen(true);
  }

  async function saveEntry(e: FormEvent) {
    e.preventDefault();
    if (!dns || !viewSet || viewSet.read_only || busy) return;
    const normalizedDomain = domain.trim().toLowerCase().replace(/\.$/, "");
    const normalizedAddr = addr.trim();
    if (!normalizedDomain) return setError(t("hosts.needDomain"));
    if (!isIpLiteral(normalizedAddr)) return setError(t("hosts.needIp"));
    if (
      viewSet.hosts.some(
        (entry) =>
          entry.id !== editEntryId &&
          entry.domain.toLowerCase() === normalizedDomain,
      )
    ) return setError(t("hosts.dupDomain", { domain: normalizedDomain }));

    const entry: HostsEntry = {
      id: editEntryId ?? newId("host"),
      enabled: entryEnabled,
      domain: normalizedDomain,
      addr: normalizedAddr,
    };
    const next = updateSet(viewSet.id, (set) => ({
      ...set,
      hosts: editEntryId
        ? set.hosts.map((item) => (item.id === editEntryId ? entry : item))
        : [...set.hosts, entry],
    }));
    if (next && await save(next)) setEntryOpen(false);
  }

  function toggleEntry(id: string) {
    if (!viewSet) return;
    const next = updateSet(viewSet.id, (set) => ({
      ...set,
      hosts: set.hosts.map((entry) =>
        entry.id === id ? { ...entry, enabled: !entry.enabled } : entry,
      ),
    }));
    if (next) void save(next);
  }

  function removeEntry(id: string) {
    if (!viewSet || !confirm(t("hosts.deleteEntryConfirm"))) return;
    const next = updateSet(viewSet.id, (set) => ({
      ...set,
      hosts: set.hosts.filter((entry) => entry.id !== id),
    }));
    if (next) void save(next);
  }

  if (!dns && !error) return <div className="settings-embed empty">{t("common.loading")}</div>;

  return (
    <div className={embedded ? "settings-embed dns-page" : "page dns-page"}>
      {error && <div className="banner error">{error}</div>}
      <div className="rules-layout">
        <aside className="card ruleset-list dns-ruleset-nav">
          <GlassButton
            icon="+"
            onClick={() => {
              setNewSetName(t("hosts.setNamePh"));
              setNewSetOpen(true);
              setError(null);
            }}
            disabled={busy}
          >
            {t("hosts.newSetBtn")}
          </GlassButton>
          <div className="ruleset-list-title">
            {t("hosts.listTitle")}
            <span className="ruleset-list-hint">{t("hosts.listHint")}</span>
          </div>
          {hostSets.map((set) => (
            <div
              key={set.id}
              className={`ruleset-item${viewSet?.id === set.id ? " selected" : ""}`}
              role="button"
              tabIndex={0}
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
                  title={set.enabled ? t("hosts.disableSetTooltip") : t("hosts.enableSetTooltip")}
                  disabled={busy}
                  onClick={(e) => {
                    e.stopPropagation();
                  }}
                  onChange={() => toggleSet(set.id)}
                />
              </div>
              <div className="muted" style={{ fontSize: 12 }}>
                {set.read_only ? t("hosts.systemReadonlyMeta") : t("hosts.mappingCount", { n: set.hosts.length })}
                {set.enabled ? ` · ${t("common.enabled")}` : ` · ${t("common.disabled")}`}
              </div>
            </div>
          ))}
        </aside>

        <section className="rules-main">
          <div className="rules-toolbar card">
            <div>
              <strong>{viewSet?.name ?? "—"}</strong>
              <div className="muted" style={{ fontSize: 12, marginTop: 2 }}>
                {viewSet?.read_only
                  ? t("hosts.readDesc")
                  : t("hosts.mapDesc")}
              </div>
            </div>
            {viewSet && <div className="header-actions">
              <button
                type="button"
                className="ghost small"
                disabled={busy || hostSets[0]?.id === viewSet.id}
                onClick={() => void moveSet(-1)}
                title={t("dns.raisePrio")}
              >↑</button>
              <button
                type="button"
                className="ghost small"
                disabled={busy || hostSets[hostSets.length - 1]?.id === viewSet.id}
                onClick={() => void moveSet(1)}
                title={t("dns.lowerPrio")}
              >↓</button>
              {!viewSet.read_only && <GlassButton
                variant="primary"
                icon="+"
                disabled={busy}
                onClick={openAddEntry}
              >{t("hosts.addMappingBtn")}</GlassButton>}
              {!viewSet.builtin && <GlassButton
                variant="danger"
                icon="⌫"
                disabled={busy}
                onClick={() => void deleteSet()}
              >{t("rules.deleteSet")}</GlassButton>}
            </div>}
          </div>

          {!viewSet ? (
            <div className="empty card muted">{t("hosts.emptySets")}</div>
          ) : viewSet.read_only ? (
            systemBusy ? <div className="empty card muted">{t("dns.loadingSystemHosts")}</div>
            : systemHosts.length === 0 ? <div className="empty card muted">{t("dns.emptySystemHosts")}</div>
            : <div className="card dns-rule-set-body"><ul className="dns-list">
              {systemHosts.map((entry) => <li key={entry.id} className="dns-list-item">
                <div className="dns-list-body">
                  <div className="dns-list-title"><span className="pill matcher-pill">{t("dns.readonlyBadge")}</span><span className="dns-list-name">{entry.domain}</span></div>
                  <div className="dns-list-addr muted mono">→ {entry.addr}</div>
                </div>
              </li>)}
            </ul></div>
          ) : viewSet.hosts.length === 0 ? (
            <div className="empty card muted">{t("hosts.emptyMappings")}</div>
          ) : <div className="card dns-rule-set-body"><ul className="dns-list">
            {viewSet.hosts.map((entry) => <li
              key={entry.id}
              className={`dns-list-item${entry.enabled ? "" : " off"}`}
              onClick={() => openEditEntry(entry)}
              title={t("hosts.clickEditMapping")}
            >
              <div className="dns-list-body">
                <div className="dns-list-title"><span className="dns-list-name">{entry.domain}</span></div>
                <div className="dns-list-addr muted mono">→ {entry.addr}</div>
              </div>
              <div className="dns-list-actions" onClick={(e) => e.stopPropagation()}>
                <GlassSwitchControl checked={entry.enabled} size="sm" title={t("hosts.enableMappingTooltip")} disabled={busy} onChange={() => toggleEntry(entry.id)} />
                <button type="button" className="rule-menu-trigger" disabled={busy} aria-label={t("hosts.deleteMappingAria")} onClick={() => removeEntry(entry.id)}>×</button>
              </div>
            </li>)}
          </ul></div>}
        </section>
      </div>

      {newSetOpen && <div className="modal-backdrop" onClick={() => !busy && setNewSetOpen(false)}>
        <div className="modal" onClick={(e) => e.stopPropagation()}>
          <header className="modal-header"><h2>{t("hosts.newSetModalTitle")}</h2><button type="button" className="icon-btn" onClick={() => setNewSetOpen(false)}>×</button></header>
          <form className="modal-body" onSubmit={(e) => void createSet(e)}>
            <label className="field"><span>{t("rules.setName")}</span><input value={newSetName} onChange={(e) => setNewSetName(e.target.value)} autoFocus /></label>
            <footer className="modal-footer"><GlassButton onClick={() => setNewSetOpen(false)}>{t("common.cancel")}</GlassButton><GlassButton type="submit" variant="primary" disabled={busy || !newSetName.trim()}>{busy ? t("common.saving") : t("rules.create")}</GlassButton></footer>
          </form>
        </div>
      </div>}

      {entryOpen && <div className="modal-backdrop" onClick={() => !busy && setEntryOpen(false)}>
        <div className="modal" onClick={(e) => e.stopPropagation()}>
          <header className="modal-header"><h2>{editEntryId ? t("hosts.editMappingTitle") : t("hosts.addMappingTitle")}</h2><button type="button" className="icon-btn" onClick={() => setEntryOpen(false)}>×</button></header>
          <form className="modal-body" onSubmit={(e) => void saveEntry(e)}>
            <label className="field"><span>{t("dns.domainLabel")}</span><input value={domain} onChange={(e) => setDomain(e.target.value)} placeholder="example.com" autoFocus /></label>
            <label className="field"><span>{t("dns.ipLabel")}</span><input value={addr} onChange={(e) => setAddr(e.target.value)} placeholder={t("hosts.ipPlaceholder")} /></label>
            <label className="sys-proxy-row" style={{ border: "none", paddingTop: 0, marginTop: 0 }}><span>{t("rules.enabled")}</span><GlassSwitchControl checked={entryEnabled} title={t("rules.enabled")} onChange={setEntryEnabled} /></label>
            <footer className="modal-footer"><GlassButton onClick={() => setEntryOpen(false)}>{t("common.cancel")}</GlassButton><GlassButton type="submit" variant="primary" disabled={busy || !domain.trim() || !addr.trim()}>{busy ? t("common.saving") : t("common.save")}</GlassButton></footer>
          </form>
        </div>
      </div>}
    </div>
  );
}
