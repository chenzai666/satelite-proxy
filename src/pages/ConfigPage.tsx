import { useCallback, useEffect, useState } from "react";
import {
  activateSubscription,
  addSubscriptionFile,
  addSubscriptionUrl,
  getSettings,
  getSubscription,
  listSubscriptions,
  refreshSubscription,
  removeSubscription,
  setMixMode,
  updateSubscription,
} from "../api";
import {
  AddConfigModal,
  type ConfigFormValues,
} from "../components/AddConfigModal";
import { GlassButton } from "../components/GlassButton";
import { GlassSwitch } from "../components/GlassSwitch";
import { useImportIntent } from "../ImportIntentContext";
import { useI18n } from "../i18n";
import type { SubscriptionTraffic, SubscriptionView } from "../types";

function formatTime(ts: number) {
  if (!ts) return "—";
  try {
    return new Date(ts * 1000).toLocaleString();
  } catch {
    return String(ts);
  }
}

/** Relative time for "Last Update" (e.g. 5 minutes ago). */
function formatRelative(
  ts: number,
  t: (key: import("../i18n").MessageKey, vars?: Record<string, string | number>) => string,
) {
  if (!ts) return "—";
  const sec = Math.max(0, Math.floor(Date.now() / 1000 - ts));
  if (sec < 60) return t("common.justNow");
  if (sec < 3600) return t("common.minutesAgo", { n: Math.floor(sec / 60) });
  if (sec < 86400) return t("common.hoursAgo", { n: Math.floor(sec / 3600) });
  if (sec < 86400 * 30) return t("common.daysAgo", { n: Math.floor(sec / 86400) });
  return formatTime(ts);
}

function formatExpireDate(ts: number) {
  try {
    return new Date(ts * 1000).toLocaleDateString(undefined, {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
    });
  } catch {
    return String(ts);
  }
}

function fmtBytes(n: number) {
  if (!Number.isFinite(n) || n < 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  const digits = i === 0 ? 0 : v >= 100 ? 0 : v >= 10 ? 1 : 2;
  return `${v.toFixed(digits)} ${units[i]}`;
}

type TrafficView = {
  used: number;
  total: number | null;
  remaining: number | null;
  ratio: number | null;
  expire: number | null;
  expireText: string | null;
};

/** used = upload + download; remaining = total - used (or explicit remaining). */
function trafficStats(t: SubscriptionTraffic | null | undefined): TrafficView | null {
  if (!t) return null;
  const upload = t.upload ?? 0;
  const download = t.download ?? 0;
  const usedFromParts = upload + download;
  const total = t.total && t.total > 0 ? t.total : null;
  const remainingExplicit =
    t.quota_remaining != null && t.quota_remaining >= 0
      ? t.quota_remaining
      : null;

  let used = usedFromParts;
  let remaining: number | null = remainingExplicit;

  if (total != null) {
    if (usedFromParts > 0) {
      remaining = Math.max(0, total - usedFromParts);
    } else if (remaining != null) {
      used = Math.max(0, total - remaining);
    } else {
      remaining = total;
      used = 0;
    }
  }

  let ratio: number | null = null;
  if (total != null && total > 0) {
    ratio = Math.min(1, Math.max(0, used / total));
  }

  const expire = t.expire && t.expire > 0 ? t.expire : null;
  const expireText = t.expire_text?.trim() || null;

  if (
    total == null &&
    remaining == null &&
    used === 0 &&
    expire == null &&
    !expireText
  ) {
    return null;
  }
  return { used, total, remaining, ratio, expire, expireText };
}

/** Compact FlClash-style traffic: thin bar + "used / total · expire". */
function TrafficBlock({ traffic }: { traffic?: SubscriptionTraffic | null }) {
  const { t } = useI18n();
  const tr = trafficStats(traffic);
  if (!tr) return null;

  const expireLabel = tr.expireText
    ? tr.expireText
    : tr.expire != null
      ? formatExpireDate(tr.expire)
      : null;

  // Full userinfo: progress = used/total (hide when total==0, same as FlClash)
  if (tr.total != null && tr.total > 0 && tr.ratio != null) {
    const barWidth = Math.min(100, Math.max(0, tr.ratio * 100));
    const level =
      tr.ratio >= 0.9 ? "critical" : tr.ratio >= 0.7 ? "warn" : "ok";
    const pct = Math.round(tr.ratio * 100);
    return (
      <div className="traffic-block">
        <div
          className="traffic-bar"
          role="progressbar"
          aria-valuenow={pct}
          aria-valuemin={0}
          aria-valuemax={100}
          title={`${fmtBytes(tr.used)} / ${fmtBytes(tr.total)} · ${pct}%`}
        >
          <div
            className={`traffic-bar-fill ${level}`}
            style={{ width: `${barWidth}%` }}
          />
        </div>
        <div className="traffic-line">
          <span>
            {fmtBytes(tr.used)} / {fmtBytes(tr.total)}
          </span>
          {expireLabel && <span className="dot-sep">·</span>}
          {expireLabel && <span>{expireLabel}</span>}
        </div>
      </div>
    );
  }

  // Remaining-only fallback
  if (tr.remaining != null || expireLabel) {
    return (
      <div className="traffic-block">
        <div className="traffic-line">
          {tr.remaining != null && (
            <span>{t("common.remaining", { n: fmtBytes(tr.remaining) })}</span>
          )}
          {tr.remaining != null && expireLabel && (
            <span className="dot-sep">·</span>
          )}
          {expireLabel && <span>{expireLabel}</span>}
        </div>
      </div>
    );
  }

  return null;
}

export function ConfigPage() {
  const { t } = useI18n();
  const { prefill, token, consume, dismiss } = useImportIntent();
  const [items, setItems] = useState<SubscriptionView[]>([]);
  const [loading, setLoading] = useState(true);
  const [listError, setListError] = useState<string | null>(null);
  const [mixMode, setMixModeState] = useState(false);

  const [modalOpen, setModalOpen] = useState(false);
  const [editId, setEditId] = useState<string | null>(null);
  const [editInitial, setEditInitial] = useState<ConfigFormValues | null>(null);
  const [importing, setImporting] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);

  const [actionId, setActionId] = useState<string | null>(null);
  const [refreshingAll, setRefreshingAll] = useState(false);
  const [menuId, setMenuId] = useState<string | null>(null);

  const busy = refreshingAll || actionId != null;

  const reload = useCallback(async () => {
    setListError(null);
    try {
      const [list, settings] = await Promise.all([
        listSubscriptions(),
        getSettings(),
      ]);
      setItems(list);
      setMixModeState(!!settings.mix_mode);
    } catch (e) {
      setListError(typeof e === "string" ? e : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // One-click subscribe deep link → open add modal with URL/name filled.
  useEffect(() => {
    if (!token || !prefill) return;
    setEditId(null);
    setImportError(null);
    setEditInitial({
      name: prefill.name ?? "",
      kind: "url",
      url: prefill.url,
      autoUpdate: true,
      autoUpdateIntervalMin: 1440,
    });
    setModalOpen(true);
    consume();
  }, [token, prefill, consume]);

  useEffect(() => {
    if (!menuId) return;
    function onDocPointerDown(e: PointerEvent) {
      const t = e.target as HTMLElement | null;
      if (t?.closest?.("[data-sub-menu]")) return;
      setMenuId(null);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setMenuId(null);
    }
    document.addEventListener("pointerdown", onDocPointerDown, true);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onDocPointerDown, true);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuId]);

  async function onActivate(id: string) {
    if (busy) return;
    setListError(null);
    try {
      const list = await activateSubscription(id);
      setItems(list);
    } catch (e) {
      setListError(typeof e === "string" ? e : String(e));
    }
  }

  async function onToggleMix() {
    if (busy) return;
    setListError(null);
    try {
      const next = !mixMode;
      const settings = await setMixMode(next);
      setMixModeState(!!settings.mix_mode);
      // policy may collapse multi-enabled → reload list
      const list = await listSubscriptions();
      setItems(list);
    } catch (e) {
      setListError(typeof e === "string" ? e : String(e));
    }
  }

  function openAdd() {
    setEditId(null);
    setEditInitial(null);
    setImportError(null);
    setModalOpen(true);
  }

  async function openEdit(id: string) {
    setImportError(null);
    setActionId(id);
    try {
      const d = await getSubscription(id);
      setEditId(id);
      setEditInitial({
        name: d.name,
        kind: d.source_kind === "file" ? "file" : "url",
        url: d.url ?? "",
        path: d.path ?? "",
        viaProxy: d.via_proxy,
        autoUpdate: !!d.auto_update,
        autoUpdateIntervalMin: d.auto_update_interval_min ?? 1440,
      });
      setModalOpen(true);
    } catch (e) {
      setListError(typeof e === "string" ? e : String(e));
    } finally {
      setActionId(null);
    }
  }

  async function handleSubmit(payload: ConfigFormValues) {
    setImporting(true);
    setImportError(null);
    try {
      const name = payload.name || null;
      const autoUpdate = !!payload.autoUpdate;
      const autoUpdateIntervalMin = payload.autoUpdateIntervalMin ?? 1440;
      if (editId) {
        await updateSubscription({
          id: editId,
          name,
          kind: payload.kind,
          url: payload.url ?? null,
          path: payload.path ?? null,
          viaProxy: payload.viaProxy ?? false,
          autoUpdate,
          autoUpdateIntervalMin,
        });
      } else if (payload.kind === "url") {
        await addSubscriptionUrl(
          name,
          payload.url ?? "",
          !!payload.viaProxy,
          autoUpdate,
          autoUpdateIntervalMin,
        );
      } else {
        await addSubscriptionFile(
          name,
          payload.path ?? "",
          autoUpdate,
          autoUpdateIntervalMin,
        );
      }
      setModalOpen(false);
      setEditId(null);
      setEditInitial(null);
      dismiss();
      await reload();
    } catch (e) {
      setImportError(typeof e === "string" ? e : String(e));
    } finally {
      setImporting(false);
    }
  }

  async function onRefresh(id: string) {
    setActionId(id);
    setListError(null);
    try {
      await refreshSubscription(id);
      await reload();
    } catch (e) {
      setListError(typeof e === "string" ? e : String(e));
    } finally {
      setActionId(null);
    }
  }

  /** Concurrently refresh every subscription (URL fetch / file re-read). */
  async function onRefreshAll() {
    if (items.length === 0 || refreshingAll) return;
    setRefreshingAll(true);
    setListError(null);
    try {
      const results = await Promise.allSettled(
        items.map((item) => refreshSubscription(item.id)),
      );
      const failed: string[] = [];
      results.forEach((r, i) => {
        if (r.status === "rejected") {
          const name = items[i]?.name ?? items[i]?.id ?? "?";
          const reason =
            typeof r.reason === "string"
              ? r.reason
              : r.reason != null
                ? String(r.reason)
                : "unknown";
          failed.push(`${name}: ${reason}`);
        }
      });
      await reload();
      if (failed.length > 0) {
        setListError(failed.slice(0, 5).join("；"));
      }
    } catch (e) {
      setListError(typeof e === "string" ? e : String(e));
    } finally {
      setRefreshingAll(false);
    }
  }

  async function onRemove(id: string) {
    if (!confirm(t("config.confirmDelete"))) return;
    setActionId(id);
    setListError(null);
    try {
      await removeSubscription(id);
      await reload();
    } catch (e) {
      setListError(typeof e === "string" ? e : String(e));
    } finally {
      setActionId(null);
    }
  }

  return (
    <div className="page config-page">
      <header className="page-header">
        <div>
          <h1>{t("config.title")}</h1>
          <p className="page-desc">{t("config.desc")}</p>
        </div>
        <div className="header-actions">
          <GlassSwitch
            checked={mixMode}
            onChange={() => void onToggleMix()}
            label={t("config.mix")}
            title={mixMode ? t("config.mixEnabled") : t("config.mixDisabled")}
            capsule
            size="sm"
          />
          <GlassButton
            icon="↻"
            disabled={busy || items.length === 0}
            onClick={() => void onRefreshAll()}
            title={t("config.refreshAll")}
          >
            {refreshingAll ? t("config.refreshing") : t("config.refreshAll")}
          </GlassButton>
          <GlassButton
            variant="primary"
            icon="+"
            disabled={busy}
            onClick={openAdd}
            title={t("config.add")}
          >
            {t("config.add")}
          </GlassButton>
        </div>
      </header>

      {listError && <div className="banner error">{listError}</div>}

      {loading ? (
        <div className="empty">{t("common.loading")}</div>
      ) : items.length === 0 ? (
        <div className="empty card">
          <p>{t("config.empty")}</p>
          <p className="muted">{t("config.emptyHint")}</p>
          <GlassButton variant="primary" icon="+" onClick={openAdd}>
            {t("config.add")}
          </GlassButton>
        </div>
      ) : (
        <div className="sub-grid">
          {items.map((item) => (
            <article
              key={item.id}
              className={`sub-card${item.enabled ? " enabled" : ""}`}
              role="button"
              tabIndex={0}
              title={
                mixMode
                  ? item.enabled
                    ? t("config.clickDisable")
                    : t("config.clickEnable")
                  : item.enabled
                    ? t("config.using")
                    : t("config.clickUse")
              }
              onClick={() => void onActivate(item.id)}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  void onActivate(item.id);
                }
              }}
            >
              <div className="sub-card-main">
                <div className="sub-card-top">
                  <h3 title={item.name}>{item.name}</h3>
                  <div className="sub-card-top-right">
                    {item.enabled && (
                      <span className="pill active-pill">{t("config.inUse")}</span>
                    )}
                    <div
                      className="sub-menu"
                      data-sub-menu
                      onClick={(e) => e.stopPropagation()}
                      onKeyDown={(e) => e.stopPropagation()}
                    >
                      <button
                        type="button"
                        className="sub-menu-trigger"
                        aria-label={t("common.actions")}
                        aria-haspopup="menu"
                        aria-expanded={menuId === item.id}
                        disabled={busy && menuId !== item.id}
                        onClick={() =>
                          setMenuId((id) => (id === item.id ? null : item.id))
                        }
                      >
                        {actionId === item.id ||
                        (refreshingAll && menuId === item.id)
                          ? "…"
                          : "⋮"}
                      </button>
                      {menuId === item.id && (
                        <div className="sub-menu-pop" role="menu">
                          <button
                            type="button"
                            role="menuitem"
                            className="sub-menu-item"
                            disabled={busy}
                            onClick={() => {
                              setMenuId(null);
                              void openEdit(item.id);
                            }}
                          >
                            {t("config.menuEdit")}
                          </button>
                          <button
                            type="button"
                            role="menuitem"
                            className="sub-menu-item"
                            disabled={busy}
                            onClick={() => {
                              setMenuId(null);
                              void onRefresh(item.id);
                            }}
                          >
                            {t("config.menuUpdate")}
                          </button>
                          <button
                            type="button"
                            role="menuitem"
                            className="sub-menu-item danger"
                            disabled={busy}
                            onClick={() => {
                              setMenuId(null);
                              void onRemove(item.id);
                            }}
                          >
                            {t("config.menuDelete")}
                          </button>
                        </div>
                      )}
                    </div>
                  </div>
                </div>
                <div className="sub-card-meta">
                  <span>{t("config.nodes", { n: item.node_count })}</span>
                  {item.skipped_count > 0 && (
                    <span className="warn">
                      {t("config.skipped", { n: item.skipped_count })}
                    </span>
                  )}
                  {item.auto_update && (
                    <span
                      className="muted"
                      title={t("config.autoUpdateHint", {
                        n: item.auto_update_interval_min ?? 1440,
                      })}
                    >
                      {t("config.autoUpdateBadge", {
                        n: item.auto_update_interval_min ?? 1440,
                      })}
                    </span>
                  )}
                  <span className="muted" title={item.source_display}>
                    {item.source_kind === "url"
                      ? t("config.url")
                      : t("config.file")}
                  </span>
                </div>
                <TrafficBlock traffic={item.traffic} />
                <div
                  className="sub-card-foot muted"
                  title={formatTime(item.last_update)}
                >
                  {formatRelative(item.last_update, t)}
                </div>
              </div>
            </article>
          ))}
        </div>
      )}

      <AddConfigModal
        open={modalOpen}
        busy={importing}
        error={importError}
        isEdit={!!editId}
        initial={editInitial}
        onClose={() => {
          if (importing) return;
          setModalOpen(false);
          setEditId(null);
          setEditInitial(null);
          dismiss();
        }}
        onSubmit={(p) => void handleSubmit(p)}
      />
    </div>
  );
}
