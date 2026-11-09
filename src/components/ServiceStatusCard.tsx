import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useId, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { useI18n, type MessageKey } from "../i18n";
import {
  createServiceStatusMonitor, MAX_VISIBLE_SERVICES, readVisibleServiceIds,
  SERVICE_STATUS_CATALOG, SERVICE_STATUS_STORAGE_KEY,
  type ServiceId, type ServiceStatus, type ServiceStatusBatch, type ServiceStatusResult,
} from "../serviceStatus";
import { GlassButton } from "./GlassButton";
import "./ServiceStatusCard.css";

const statusLabels: Record<ServiceStatus, MessageKey> = {
  reachable: "serviceStatus.reachable",
  redirect: "serviceStatus.redirect",
  restricted: "serviceStatus.restricted",
  http_error: "serviceStatus.httpError",
  timeout: "serviceStatus.timeout",
  network_error: "serviceStatus.networkError",
};

// 手绘小标，无远程图片、字体或品牌资源请求。
function ServiceGlyph({ id }: { id?: ServiceId }) {
  const paths: Partial<Record<ServiceId, string>> = {
    google: "M17 7a7 7 0 1 0 1 9v-4h-6",
    github: "M8 18c-4 1-4-2-5-2m13 4v-3c0-1 0-2-1-2 4 0 6-2 6-5 0-2-1-3-2-4 0-1 0-2-1-3l-4 2h-4L6 3C5 4 5 5 5 6c-1 1-2 2-2 4 0 3 2 5 6 5-1 0-1 1-1 2v3",
    youtube: "M4 6h16v12H4zM10 9l5 3-5 3z",
    chatgpt: "m12 3 8 5v8l-8 5-8-5V8zM8 7l8 10M16 7 8 17M4 12h16",
    claude: "M12 3v18M3 12h18M6 6l12 12M18 6 6 18",
    gemini: "M12 2c0 7 3 10 10 10-7 0-10 3-10 10 0-7-3-10-10-10 7 0 10-3 10-10Z",
    netflix: "M7 20V4l10 16V4",
    disney: "M5 6c12-5 18 13 0 12V6m4 4c7-1 7 5 0 5",
    prime: "m9 5 9 6-9 6zM4 19c5 3 11 3 16-1",
    spotify: "M4 8c5-3 11-3 16 0M6 12c4-2 8-2 12 0M8 16c3-1 5-1 8 0",
    tiktok: "M14 3v13a4 4 0 1 1-4-4m4-9c1 4 3 5 6 5",
    discord: "m8 6-3 1-3 11 5 2 2-3h6l2 3 5-2-3-11-3-1M8 12v2m8-2v2M8 6h8",
    bilibili: "M4 7h16v13H4zM8 3l3 4m5-4-3 4M8 12v3m8-3v3",
  };
  return <svg className="service-status-glyph" viewBox="0 0 24 24" aria-hidden="true">
    {id ? <path d={paths[id]} /> : <><circle cx="12" cy="12" r="9" /><path d="m6 12 4 4 8-8" /></>}
  </svg>;
}

function initialSelection() {
  try {
    return readVisibleServiceIds(window.localStorage.getItem(SERVICE_STATUS_STORAGE_KEY));
  } catch {
    return readVisibleServiceIds(null);
  }
}

export function ServiceStatusCard({ sessionKey }: { sessionKey: string }) {
  const { t, locale } = useI18n();
  const [visibleIds, setVisibleIds] = useState<ServiceId[]>(initialSelection);
  const [open, setOpen] = useState(false);
  const [storageFailed, setStorageFailed] = useState(false);
  const initialIds = useRef(visibleIds);
  initialIds.current = visibleIds;
  const monitor = useMemo(() => createServiceStatusMonitor(
    ids => invoke<ServiceStatusBatch>("check_service_status", { ids }),
  ), [sessionKey]);
  const { results, pending, errors } = useSyncExternalStore(monitor.subscribe, monitor.getSnapshot, monitor.getSnapshot);
  const dialogRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  const scopeId = useId();
  const selectionHintId = useId();
  const close = useCallback(() => setOpen(false), []);

  useEffect(() => {
    monitor.setEnabled(document.visibilityState !== "hidden" && navigator.onLine !== false);
    monitor.start(initialIds.current);
    const sync = () => monitor.setEnabled(document.visibilityState !== "hidden" && navigator.onLine !== false);
    const wake = () => { sync(); monitor.refreshVisible(); };
    document.addEventListener("visibilitychange", wake);
    window.addEventListener("online", wake);
    window.addEventListener("offline", sync);
    window.addEventListener("focus", wake);
    return () => {
      document.removeEventListener("visibilitychange", wake);
      window.removeEventListener("online", wake);
      window.removeEventListener("offline", sync);
      window.removeEventListener("focus", wake);
      monitor.stop();
    };
  }, [monitor]);

  useEffect(() => monitor.setVisible(visibleIds), [monitor, visibleIds]);

  useEffect(() => {
    if (!open) return;
    const previousFocus = document.activeElement;
    const dialog = dialogRef.current;
    if (!dialog) return;
    closeRef.current?.focus();
    function keepFocus(event: FocusEvent) {
      if (!dialog?.contains(event.target as Node)) closeRef.current?.focus();
    }
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        close();
      } else if (event.key === "Tab" && dialog) {
        const elements = [...dialog.querySelectorAll<HTMLElement>("button:not(:disabled), input:not(:disabled), [tabindex='0']")];
        const index = elements.indexOf(document.activeElement as HTMLElement);
        if (index < 0 || (event.shiftKey && index === 0) || (!event.shiftKey && index === elements.length - 1)) {
          event.preventDefault();
          (event.shiftKey ? elements[elements.length - 1] : elements[0])?.focus();
        }
      }
    }
    document.addEventListener("keydown", onKeyDown, true);
    document.addEventListener("focusin", keepFocus);
    return () => {
      document.removeEventListener("keydown", onKeyDown, true);
      document.removeEventListener("focusin", keepFocus);
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus();
    };
  }, [open, close]);

  function toggleVisible(id: ServiceId) {
    const selected = visibleIds.includes(id);
    if ((!selected && visibleIds.length >= MAX_VISIBLE_SERVICES) || (selected && visibleIds.length === 1)) return;
    const next = selected ? visibleIds.filter(item => item !== id) : [...visibleIds, id];
    setVisibleIds(next);
    try {
      window.localStorage.setItem(SERVICE_STATUS_STORAGE_KEY, JSON.stringify(next));
      setStorageFailed(false);
    } catch {
      setStorageFailed(true);
    }
  }

  function statusText(id: ServiceId) {
    if (pending.includes(id)) return t("serviceStatus.checking");
    return results[id] ? t(statusLabels[results[id].status]) : "—";
  }

  function latency(result?: ServiceStatusResult) {
    return result?.latencyMs === undefined ? "—" : `${Math.round(result.latencyMs)} ms`;
  }

  function cardStatus(id: ServiceId) {
    if (pending.includes(id)) return <span className="service-status-checking"><span className="service-status-progress" aria-hidden="true" />{t("serviceStatus.checking")}</span>;
    const result = results[id];
    if (result && ["reachable", "redirect"].includes(result.status) && result.latencyMs !== undefined) return latency(result);
    return statusText(id);
  }

  function checkedTime(result?: ServiceStatusResult) {
    if (!result) return "—";
    const date = new Date(result.checkedAt);
    return Number.isNaN(date.getTime()) ? "—" : date.toLocaleString(locale === "zh" ? "zh-CN" : "en-US");
  }

  const failed = Object.values(errors).some(Boolean);
  const allIds = SERVICE_STATUS_CATALOG.map(service => service.id);

  return <>
    <button type="button" className="instrument service-status-card glass-btn"
      aria-haspopup="dialog" aria-expanded={open} aria-label={t("serviceStatus.openDetails")} onClick={() => setOpen(true)}>
      <span className="instrument-head">
        <ServiceGlyph />
        <span className="instrument-label">{t("serviceStatus.title")}</span>
        <span className="service-status-card-arrow" aria-hidden="true">↗</span>
      </span>
      <span className="service-status-card-rows">
        {visibleIds.map(id => <span className="service-status-card-row" key={id} data-service-id={id}>
          <span className="service-status-name"><ServiceGlyph id={id} />{SERVICE_STATUS_CATALOG.find(service => service.id === id)?.title}</span>
          <span className="service-status-state" data-status={pending.includes(id) ? undefined : results[id]?.status} title={statusText(id)}>
            {errors[id] && <span className="service-status-error-mark" aria-label={t("serviceStatus.failed")}>!</span>}{cardStatus(id)}
          </span>
        </span>)}
      </span>
      <span className="service-status-card-hint">{t("serviceStatus.httpOnly")}</span>
      {failed && <span className="service-status-error" role="status">{t("serviceStatus.failed")}</span>}
    </button>
    {open && createPortal(<div className="modal-backdrop service-status-backdrop" onClick={event => {
      if (event.target === event.currentTarget) close();
    }}>
      <div className="modal service-status-modal" ref={dialogRef} role="dialog" aria-modal="true"
        aria-labelledby={titleId} aria-describedby={scopeId}>
        <header className="modal-header">
          <h2 id={titleId}>{t("serviceStatus.detailsTitle")}</h2>
          <button className="icon-btn" type="button" ref={closeRef} aria-label={t("serviceStatus.close")} onClick={close}>×</button>
        </header>
        <div className="modal-body service-status-body">
          <p className="service-status-scope" id={scopeId}>{t("serviceStatus.scope")}</p>
          <div className="service-status-toolbar">
            <p id={selectionHintId}>{t("serviceStatus.selectionHint", { count: visibleIds.length })}</p>
            <GlassButton disabled={pending.length > 0} onClick={() => void monitor.request(allIds)}>
              {pending.length ? t("serviceStatus.checking") : t("serviceStatus.checkAll")}
            </GlassButton>
          </div>
          <p className="service-status-feedback" role="status" aria-live="polite">
            {storageFailed ? t("serviceStatus.storageFailed") : failed ? t("serviceStatus.batchFailed") : t("serviceStatus.onDemand")}
          </p>
          <ul className="service-status-list" aria-label={t("serviceStatus.allServices")}>
            {SERVICE_STATUS_CATALOG.map(service => {
              const result = results[service.id];
              const selected = visibleIds.includes(service.id);
              return <li className="service-status-row" data-service-id={service.id} key={service.id}>
                <div className="service-status-row-main">
                  <span className="service-status-name"><ServiceGlyph id={service.id} />{service.title}</span>
                  <span className="service-status-state" data-status={result?.status}>{statusText(service.id)}</span>
                  <span className="service-status-latency" aria-label={t("serviceStatus.latency")}>{latency(result)}</span>
                  <GlassButton disabled={pending.length > 0} aria-label={t("serviceStatus.checkService", { name: service.title })}
                    onClick={() => void monitor.request([service.id])}>
                    {pending.includes(service.id) ? t("serviceStatus.checking") : result ? t("serviceStatus.retest") : t("serviceStatus.check")}
                  </GlassButton>
                </div>
                <div className="service-status-row-meta">
                  <span>{t("serviceStatus.httpStatus")}: {result?.httpStatus ?? "—"}</span>
                  <span>{t("serviceStatus.route")}: {result ? t(result.viaProxy ? "serviceStatus.kernelRules" : "serviceStatus.direct") : "—"}</span>
                  <span>{t("serviceStatus.checkedAt")}: {checkedTime(result)}</span>
                  <label className="service-status-toggle">
                    <input type="checkbox" role="switch" checked={selected} aria-describedby={selectionHintId}
                      aria-label={t("serviceStatus.showService", { name: service.title })}
                      disabled={selected ? visibleIds.length === 1 : visibleIds.length >= MAX_VISIBLE_SERVICES}
                      onChange={() => toggleVisible(service.id)} />
                    {t("serviceStatus.showOnCard")}
                  </label>
                </div>
                {errors[service.id] && <p className="service-status-error">{t(result ? "serviceStatus.failedRetained" : "serviceStatus.failed")}</p>}
              </li>;
            })}
          </ul>
        </div>
      </div>
    </div>, document.body)}
  </>;
}
