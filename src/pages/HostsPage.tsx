import { useCallback, useEffect, useMemo, useState, type FormEvent } from "react";
import { getDnsSettings, readSystemHosts, updateDnsSettings } from "../api";
import { GlassButton } from "../components/GlassButton";
import type { DnsRuleSet, DnsSettings, HostsEntry } from "../types";

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
  const [dns, setDns] = useState<DnsSettings | null>(null);
  const [viewSetId, setViewSetId] = useState<string | null>(null);
  const [systemHosts, setSystemHosts] = useState<HostsEntry[]>([]);
  const [systemBusy, setSystemBusy] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [newSetOpen, setNewSetOpen] = useState(false);
  const [newSetName, setNewSetName] = useState("自定义 Hosts");
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
    if (!name) return setError("请输入 Hosts 集名称");
    if (hostSets.some((set) => set.name.toLowerCase() === name.toLowerCase())) {
      return setError(`已存在同名 Hosts 集「${name}」`);
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
    if (!confirm(`删除 Hosts 集「${viewSet.name}」？`)) return;
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
    if (!normalizedDomain) return setError("请输入域名");
    if (!isIpLiteral(normalizedAddr)) return setError("请输入有效的 IPv4 或 IPv6 地址");
    if (
      viewSet.hosts.some(
        (entry) =>
          entry.id !== editEntryId &&
          entry.domain.toLowerCase() === normalizedDomain,
      )
    ) return setError(`该集合已存在域名「${normalizedDomain}」`);

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
    if (!viewSet || !confirm("删除该 Hosts 映射？")) return;
    const next = updateSet(viewSet.id, (set) => ({
      ...set,
      hosts: set.hosts.filter((entry) => entry.id !== id),
    }));
    if (next) void save(next);
  }

  if (!dns && !error) return <div className="settings-embed empty">加载中…</div>;

  return (
    <div className={embedded ? "settings-embed dns-page" : "page dns-page"}>
      {error && <div className="banner error">{error}</div>}
      <div className="rules-layout">
        <aside className="card ruleset-list dns-ruleset-nav">
          <GlassButton
            icon="+"
            onClick={() => {
              setNewSetName("自定义 Hosts");
              setNewSetOpen(true);
              setError(null);
            }}
            disabled={busy}
          >
            新建 Hosts 集
          </GlassButton>
          <div className="ruleset-list-title">
            Hosts 集
            <span className="ruleset-list-hint">顺序即优先级</span>
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
                <button
                  type="button"
                  role="switch"
                  className={`switch small ${set.enabled ? "on" : ""}`}
                  aria-checked={set.enabled}
                  disabled={busy}
                  onClick={(e) => {
                    e.stopPropagation();
                    toggleSet(set.id);
                  }}
                >
                  <span className="switch-thumb" />
                </button>
              </div>
              <div className="muted" style={{ fontSize: 12 }}>
                {set.read_only ? "系统文件 · 只读" : `${set.hosts.length} 条映射`}
                {set.enabled ? " · 已启用" : " · 未启用"}
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
                  ? "读取操作系统 Hosts 文件，内容只读"
                  : "域名 → IP 静态映射，按 Hosts 集顺序匹配"}
              </div>
            </div>
            {viewSet && <div className="header-actions">
              <button
                type="button"
                className="ghost small"
                disabled={busy || hostSets[0]?.id === viewSet.id}
                onClick={() => void moveSet(-1)}
                title="提高优先级"
              >↑</button>
              <button
                type="button"
                className="ghost small"
                disabled={busy || hostSets[hostSets.length - 1]?.id === viewSet.id}
                onClick={() => void moveSet(1)}
                title="降低优先级"
              >↓</button>
              {!viewSet.read_only && <GlassButton
                variant="primary"
                icon="+"
                disabled={busy}
                onClick={openAddEntry}
              >添加映射</GlassButton>}
              {!viewSet.builtin && <GlassButton
                variant="danger"
                icon="⌫"
                disabled={busy}
                onClick={() => void deleteSet()}
              >删除集</GlassButton>}
            </div>}
          </div>

          {!viewSet ? (
            <div className="empty card muted">暂无 Hosts 集</div>
          ) : viewSet.read_only ? (
            systemBusy ? <div className="empty card muted">正在读取系统 Hosts…</div>
            : systemHosts.length === 0 ? <div className="empty card muted">系统 Hosts 中没有可用映射</div>
            : <div className="card dns-rule-set-body"><ul className="dns-list">
              {systemHosts.map((entry) => <li key={entry.id} className="dns-list-item">
                <div className="dns-list-body">
                  <div className="dns-list-title"><span className="pill matcher-pill">只读</span><span className="dns-list-name">{entry.domain}</span></div>
                  <div className="dns-list-addr muted mono">→ {entry.addr}</div>
                </div>
              </li>)}
            </ul></div>
          ) : viewSet.hosts.length === 0 ? (
            <div className="empty card muted">暂无 Hosts 映射</div>
          ) : <div className="card dns-rule-set-body"><ul className="dns-list">
            {viewSet.hosts.map((entry) => <li
              key={entry.id}
              className={`dns-list-item${entry.enabled ? "" : " off"}`}
              onClick={() => openEditEntry(entry)}
              title="点击编辑映射"
            >
              <div className="dns-list-body">
                <div className="dns-list-title"><span className="dns-list-name">{entry.domain}</span></div>
                <div className="dns-list-addr muted mono">→ {entry.addr}</div>
              </div>
              <div className="dns-list-actions" onClick={(e) => e.stopPropagation()}>
                <button type="button" role="switch" aria-checked={entry.enabled} className={`switch small ${entry.enabled ? "on" : ""}`} disabled={busy} onClick={() => toggleEntry(entry.id)}><span className="switch-thumb" /></button>
                <button type="button" className="rule-menu-trigger" disabled={busy} aria-label="删除映射" onClick={() => removeEntry(entry.id)}>×</button>
              </div>
            </li>)}
          </ul></div>}
        </section>
      </div>

      {newSetOpen && <div className="modal-backdrop" onClick={() => !busy && setNewSetOpen(false)}>
        <div className="modal" onClick={(e) => e.stopPropagation()}>
          <header className="modal-header"><h2>新建 Hosts 集</h2><button type="button" className="icon-btn" onClick={() => setNewSetOpen(false)}>×</button></header>
          <form className="modal-body" onSubmit={(e) => void createSet(e)}>
            <label className="field"><span>名称</span><input value={newSetName} onChange={(e) => setNewSetName(e.target.value)} autoFocus /></label>
            <footer className="modal-footer"><button type="button" className="secondary" onClick={() => setNewSetOpen(false)}>取消</button><button type="submit" disabled={busy || !newSetName.trim()}>{busy ? "保存中…" : "创建"}</button></footer>
          </form>
        </div>
      </div>}

      {entryOpen && <div className="modal-backdrop" onClick={() => !busy && setEntryOpen(false)}>
        <div className="modal" onClick={(e) => e.stopPropagation()}>
          <header className="modal-header"><h2>{editEntryId ? "编辑 Hosts 映射" : "添加 Hosts 映射"}</h2><button type="button" className="icon-btn" onClick={() => setEntryOpen(false)}>×</button></header>
          <form className="modal-body" onSubmit={(e) => void saveEntry(e)}>
            <label className="field"><span>域名</span><input value={domain} onChange={(e) => setDomain(e.target.value)} placeholder="example.com" autoFocus /></label>
            <label className="field"><span>IP 地址</span><input value={addr} onChange={(e) => setAddr(e.target.value)} placeholder="127.0.0.1 或 ::1" /></label>
            <label className="sys-proxy-row" style={{ border: "none", paddingTop: 0, marginTop: 0 }}><span>启用</span><button type="button" role="switch" aria-checked={entryEnabled} className={`switch ${entryEnabled ? "on" : ""}`} onClick={() => setEntryEnabled((value) => !value)}><span className="switch-thumb" /></button></label>
            <footer className="modal-footer"><button type="button" className="secondary" onClick={() => setEntryOpen(false)}>取消</button><button type="submit" disabled={busy || !domain.trim() || !addr.trim()}>{busy ? "保存中…" : "保存"}</button></footer>
          </form>
        </div>
      </div>}
    </div>
  );
}
