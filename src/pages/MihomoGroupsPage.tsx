import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  closeAllConnections,
  getProxyStatus,
  getMihomoTemplate,
  saveMihomoTemplate,
  fetchMihomoTemplate,
  reorderMihomoGroups,
  listMihomoProxyGroups,
  onProxySnapshot,
  selectMihomoProxyGroup,
  setOutboundMode,
} from "../api";
import { ErrorModal } from "../components/ErrorModal";
import { GlassButton } from "../components/GlassButton";
import { GlassSeg } from "../components/GlassSeg";
import { useVisibleInterval } from "../hooks/useVisibleInterval";
import { useNodeDragSort } from "../hooks/useNodeDragSort";
import { moveMihomoGroup } from "../mihomoGroupOrder";
import { useI18n } from "../i18n";
import type { MihomoProxyGroup, MihomoTemplateView } from "../types";

interface Props {
  embedded?: boolean;
  /** Use the dashboard card layout instead of the Settings page spacing. */
  compact?: boolean;
  /** The dashboard already owns the global/rule/direct switch. */
  showMode?: boolean;
}

function groupLabel(group: MihomoProxyGroup) {
  if (group.labels?.[group.name]) return group.labels[group.name];
  if (group.name === "GLOBAL") return "🌐 GLOBAL";
  if (group.name === "proxy") return "🚀 节点选择";
  if (group.name === "auto") return "📈 自动选择";
  return group.name;
}

function memberLabel(group: MihomoProxyGroup, member: string) {
  return group.labels?.[member] ?? member;
}

function wait(ms: number) {
  return new Promise<void>((resolve) => window.setTimeout(resolve, ms));
}

/** Restarting Mihomo replaces its Clash API listener. A successful restart
 * can therefore be a few hundred milliseconds ahead of GET /proxies. */
async function loadGroupsWithRetry() {
  let lastError: unknown = null;
  for (const delay of [0, 180, 360, 720]) {
    if (delay) await wait(delay);
    try {
      return await listMihomoProxyGroups();
    } catch (reason) {
      lastError = reason;
    }
  }
  throw lastError ?? new Error("Mihomo policy groups unavailable");
}

export function MihomoGroupsPage({
  embedded = false,
  compact = false,
  showMode = true,
}: Props) {
  const { t } = useI18n();
  const [groups, setGroups] = useState<MihomoProxyGroup[]>([]);
  const [running, setRunning] = useState(false);
  const [mode, setMode] = useState<"rule" | "global" | "direct">("rule");
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState("");
  const [busyGroup, setBusyGroup] = useState<string | null>(null);
  const [modeBusy, setModeBusy] = useState(false);
  const [closing, setClosing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const liveStatusKeyRef = useRef<string | null>(null);
  const groupRequestRef = useRef(0);
  const [profile, setProfile] = useState<MihomoTemplateView | null>(null);
  const [groupOrder, setGroupOrder] = useState<string[]>([]);
  const [showTemplate, setShowTemplate] = useState(false);
  const [templateBusy, setTemplateBusy] = useState(false);
  const [orderBusy, setOrderBusy] = useState(false);
  useEffect(() => {
    let disposed = false;
    void getMihomoTemplate().then(next => {
      if (!disposed) { setProfile(next); setGroupOrder(next.group_order); }
    }).catch(reason => { if (!disposed) setError(String(reason)); });
    return () => { disposed = true; };
  }, []);

  const orderedGroups = useMemo(() => {
    const ranks = new Map(groupOrder.map((name, i) => [name, i]));
    return [...groups].sort((a,b) => (ranks.get(a.name) ?? Number.MAX_SAFE_INTEGER) - (ranks.get(b.name) ?? Number.MAX_SAFE_INTEGER));
  }, [groups, groupOrder]);
  const groupDrag = useNodeDragSort((id, target, after) => {
    const next = moveMihomoGroup(orderedGroups.map(g => g.name), id, target, after);
    setOrderBusy(true);
    void reorderMihomoGroups(next).then(setGroupOrder).catch(reason => setError(String(reason)))
      .finally(() => setOrderBusy(false));
  }, "groupName");

  async function saveTemplate() {
    if (!profile || templateBusy) return;
    setTemplateBusy(true);
    try {
      const next = await saveMihomoTemplate(profile.template, profile.use_for_active);
      setProfile(next); setShowTemplate(false);
      ++groupRequestRef.current; setGroups([]);
      // Status events and the normal refresh repopulate after the debounced restart.
    } catch (reason) { setError(String(reason)); }
    finally { setTemplateBusy(false); }
  }

  async function fetchTemplate() {
    if (!profile || templateBusy) return;
    setTemplateBusy(true);
    try {
      const ini = await fetchMihomoTemplate(profile.template.url);
      setProfile(p => p ? { ...p, template: { ...p.template, ini } } : p);
    } catch (reason) { setError(String(reason)); }
    finally { setTemplateBusy(false); }
  }

  const statusKey = (status: Awaited<ReturnType<typeof getProxyStatus>>) =>
    `${status.running ? "running" : "stopped"}:${status.core_type ?? ""}:${status.outbound_mode ?? ""}`;

  const loadLiveGroups = useCallback(async () => {
    const request = ++groupRequestRef.current;
    const next = await loadGroupsWithRetry();
    if (request === groupRequestRef.current) setGroups(next);
  }, []);

  const reload = useCallback(async (quiet = false) => {
    if (!quiet) setLoading(true);
    try {
      const status = await getProxyStatus();
      const outbound = status.outbound_mode?.toLowerCase();
      if (outbound === "global" || outbound === "direct" || outbound === "rule") {
        setMode(outbound);
      }
      const active = status.running && status.core_type === "mihomo";
      liveStatusKeyRef.current = statusKey(status);
      setRunning(active);
      if (!active) {
        ++groupRequestRef.current;
        setGroups([]);
        return;
      }
      await loadLiveGroups();
    } catch (reason) {
      if (!quiet) setError(typeof reason === "string" ? reason : String(reason));
    } finally {
      if (!quiet) setLoading(false);
    }
  }, [loadLiveGroups]);

  useEffect(() => {
    void reload();
  }, [reload]);

  // The dashboard and this embedded page are mounted together. Starting the
  // core updates the shared proxy snapshot, but used to leave this page's
  // `running=false` state untouched until navigation remounted it. React to
  // lifecycle/mode/core changes directly so the live Clash API is queried as
  // soon as Mihomo is ready. The request counter prevents a late response
  // from an old core session from repainting the current page.
  useEffect(() => {
    let disposed = false;
    const unsubscribe = onProxySnapshot((status) => {
      if (disposed) return;
      const key = statusKey(status);
      if (key === liveStatusKeyRef.current) return;
      liveStatusKeyRef.current = key;

      const outbound = status.outbound_mode?.toLowerCase();
      if (outbound === "global" || outbound === "direct" || outbound === "rule") {
        setMode(outbound);
      }
      const active = status.running && status.core_type === "mihomo";
      setRunning(active);
      if (!active) {
        ++groupRequestRef.current;
        setGroups([]);
        return;
      }

      void loadLiveGroups().catch(() => undefined);
    });
    return () => {
      disposed = true;
      unsubscribe();
    };
  }, [loadLiveGroups]);

  // Keep a small fallback window for a start that completes between the
  // initial status request and listener registration. Once running, the
  // normal five-second live refresh remains in effect.
  useVisibleInterval(() => reload(true), running ? 5000 : 1000);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return orderedGroups;
    return orderedGroups.filter((group) => {
      const text = [
        groupLabel(group),
        group.now,
        memberLabel(group, group.now),
        ...group.all.map((member) => memberLabel(group, member)),
      ]
        .join(" ")
        .toLowerCase();
      return text.includes(needle);
    });
  }, [orderedGroups, query]);

  async function switchGroup(group: MihomoProxyGroup, member: string) {
    if (!member || member === group.now) return;
    setBusyGroup(group.name);
    try {
      setGroups(await selectMihomoProxyGroup(group.name, member));
    } catch (reason) {
      setError(typeof reason === "string" ? reason : String(reason));
    } finally {
      setBusyGroup(null);
    }
  }

  async function switchMode(next: string) {
    if (next !== "rule" && next !== "global" && next !== "direct") return;
    if (next === mode) return;
    setModeBusy(true);
    setError(null);
    try {
      const status = await setOutboundMode(next);
      setMode(next);
      const active = status.running && status.core_type === "mihomo";
      setRunning(active);
      // Never leave the old Rule-mode groups painted while the new mode is
      // being applied. The refreshed list must come from Mihomo's live API.
      setGroups([]);
      if (active) setGroups(await loadGroupsWithRetry());
    } catch (reason) {
      setError(typeof reason === "string" ? reason : String(reason));
    } finally {
      setModeBusy(false);
    }
  }

  async function closeConnections() {
    setClosing(true);
    try {
      await closeAllConnections();
    } catch (reason) {
      setError(typeof reason === "string" ? reason : String(reason));
    } finally {
      setClosing(false);
    }
  }

  const body = (
    <>
      <div className={`rules-toolbar page-header mihomo-groups-header${compact ? " is-compact" : ""}`}>
        <div>
          <h1>{t("mihomoGroups.title")}</h1>
          <p className="muted">{t("mihomoGroups.subtitle")}</p>
        </div>
        {showMode ? (
          <GlassSeg
            value={mode}
            ready={!loading}
            disabled={modeBusy}
            ariaLabel={t("mihomoGroups.mode")}
            options={[
              { value: "rule", label: t("dashboard.modeRule") },
              { value: "global", label: t("dashboard.modeGlobal") },
              { value: "direct", label: t("dashboard.modeDirect") },
            ]}
            onChange={(value) => void switchMode(value)}
          />
        ) : null}
      </div>

      {!compact ? (
        <div className="card mihomo-groups-note">
          <strong>{t("mihomoGroups.singboxDisabled")}</strong>
        </div>
      ) : null}

      <div className="card mihomo-groups-tools">
        <GlassButton disabled={!profile} onClick={() => setShowTemplate(true)}>{t("mihomoGroups.template")}</GlassButton>
        <input
          className="rules-filter"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("mihomoGroups.search")}
        />
        <GlassButton icon="↻" disabled={loading} onClick={() => void reload()}>
          {t("common.refresh")}
        </GlassButton>
        <GlassButton
          icon="×"
          disabled={!running || closing}
          onClick={() => void closeConnections()}
        >
          {closing ? t("mihomoGroups.closing") : t("mihomoGroups.closeConnections")}
        </GlassButton>
      </div>

      {loading ? (
        <div className="empty muted">{t("common.loading")}</div>
      ) : !running ? (
        <div className="card empty muted">{t("mihomoGroups.notRunning")}</div>
      ) : filtered.length === 0 ? (
        <div className="card empty muted">{t("mihomoGroups.noGroups")}</div>
      ) : (
        <div className="mihomo-group-list">
          {filtered.map((group) => {
            const selector = group.group_type.toLowerCase() === "selector";
            const busy = busyGroup === group.name;
            return (
              <section className="card mihomo-group-row" key={group.name} data-group-name={group.name}
                onPointerDown={event => groupDrag.onPointerDown(event, group.name, !modeBusy && !busyGroup && !orderBusy && filtered.length > 1)}>
                <div className="mihomo-group-summary">
                  <div className="mihomo-group-title-line">
                    <strong>{groupLabel(group)}</strong>
                    <span className={`pill ${selector ? "" : "ok"}`}>
                      {selector ? "Selector" : group.group_type}
                    </span>
                  </div>
                  <div className="muted mihomo-group-current">
                    {t("mihomoGroups.current")} · {memberLabel(group, group.now) || "—"}
                  </div>
                </div>
                <div className="mihomo-group-control">
                  <span className="pill">{group.all.length}</span>
                  {selector ? (
                    <select
                      value={group.now}
                      disabled={busy}
                      aria-label={`${groupLabel(group)} ${t("mihomoGroups.switch")}`}
                      onChange={(event) => void switchGroup(group, event.target.value)}
                    >
                      {group.all.map((member) => (
                        <option value={member} key={member}>
                          {memberLabel(group, member)}
                        </option>
                      ))}
                    </select>
                  ) : (
                    <span className="muted mihomo-group-auto">{t("mihomoGroups.automatic")}</span>
                  )}
                  {busy ? <span className="lat-spinner" aria-hidden /> : null}
                </div>
              </section>
            );
          })}
        </div>
      )}
      {error ? <ErrorModal message={error} onClose={() => setError(null)} /> : null}
      {showTemplate && profile ? <div className="modal-backdrop"><section className="modal mihomo-template-modal" role="dialog" aria-modal="true" aria-label={t("mihomoGroups.template")}>
        <header className="modal-header"><h2>{t("mihomoGroups.template")}</h2>
          <GlassButton disabled={templateBusy} onClick={() => setShowTemplate(false)}>{t("common.cancel")}</GlassButton></header>
        <div className="modal-body">
          <p className="muted">{t("mihomoGroups.templateHint")}</p>
          <label className="field"><span>{t("mihomoGroups.templateUrl")}</span>
            <input value={profile.template.url} disabled={templateBusy} onChange={e => setProfile({ ...profile, template: { ...profile.template, url: e.target.value } })} /></label>
          <GlassButton disabled={templateBusy} onClick={() => void fetchTemplate()}>{t("mihomoGroups.fetchTemplate")}</GlassButton>
          <label className="field"><span>INI</span><textarea className="config-paste" rows={10} spellCheck={false} disabled={templateBusy}
            value={profile.template.ini} onChange={e => setProfile({ ...profile, template: { ...profile.template, ini: e.target.value } })} /></label>
          <label className="field"><span>{t("mihomoGroups.overrides")}</span><textarea className="config-paste" rows={10} spellCheck={false} disabled={templateBusy}
            value={profile.template.override_yaml} onChange={e => setProfile({ ...profile, template: { ...profile.template, override_yaml: e.target.value } })} /></label>
          <label><input type="checkbox" disabled={templateBusy} checked={profile.use_for_active}
            onChange={e => setProfile({ ...profile, use_for_active: e.target.checked })} /> {t("mihomoGroups.useTemplate")}</label>
        </div>
        <footer className="modal-footer"><GlassButton disabled={templateBusy} onClick={() => void saveTemplate()}>{t("common.save")}</GlassButton></footer>
      </section></div> : null}
    </>
  );

  return embedded ? (
    <div className={`settings-embed rules-embed${compact ? " mihomo-groups-dashboard" : ""}`}>
      {body}
    </div>
  ) : (
    <div className="page rules-page">{body}</div>
  );
}
