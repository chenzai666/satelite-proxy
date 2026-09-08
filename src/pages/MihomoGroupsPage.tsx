import { useCallback, useEffect, useMemo, useState } from "react";
import {
  closeAllConnections,
  getProxyStatus,
  listMihomoProxyGroups,
  selectMihomoProxyGroup,
  setOutboundMode,
} from "../api";
import { ErrorModal } from "../components/ErrorModal";
import { GlassButton } from "../components/GlassButton";
import { GlassSeg } from "../components/GlassSeg";
import { useVisibleInterval } from "../hooks/useVisibleInterval";
import { useI18n } from "../i18n";
import type { MihomoProxyGroup } from "../types";

interface Props {
  embedded?: boolean;
  /** Use the dashboard card layout instead of the Settings page spacing. */
  compact?: boolean;
  /** The dashboard already owns the global/rule/direct switch. */
  showMode?: boolean;
}

function groupLabel(group: MihomoProxyGroup) {
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

  const reload = useCallback(async (quiet = false) => {
    if (!quiet) setLoading(true);
    try {
      const status = await getProxyStatus();
      const outbound = status.outbound_mode?.toLowerCase();
      if (outbound === "global" || outbound === "direct" || outbound === "rule") {
        setMode(outbound);
      }
      const active = status.running && status.core_type === "mihomo";
      setRunning(active);
      if (!active) {
        setGroups([]);
        return;
      }
      setGroups(await loadGroupsWithRetry());
    } catch (reason) {
      if (!quiet) setError(typeof reason === "string" ? reason : String(reason));
    } finally {
      if (!quiet) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);
  useVisibleInterval(() => reload(true), running ? 5000 : null);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return groups;
    return groups.filter((group) => {
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
  }, [groups, query]);

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
              <section className="card mihomo-group-row" key={group.name}>
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
