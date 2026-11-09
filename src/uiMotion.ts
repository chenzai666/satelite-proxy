import { useSyncExternalStore } from "react";

export type UiMotion = "standard" | "system" | "reduced";
export const UI_MOTION_KEY = "satelite.uiMotion";
export function normalizeUiMotion(value: unknown): UiMotion {
  return value === "system" || value === "reduced" ? value : "standard";
}
export function shouldReduceMotion(mode: UiMotion, systemReduced: boolean): boolean {
  return mode === "reduced" || (mode === "system" && systemReduced);
}
const listeners = new Set<() => void>();
let mode: UiMotion = "standard";
if (typeof window !== "undefined") {
  try { mode = normalizeUiMotion(localStorage.getItem(UI_MOTION_KEY)); } catch { /* 使用进程内偏好。 */ }
}
const media = typeof window === "undefined" ? null : window.matchMedia("(prefers-reduced-motion: reduce)");
function apply() {
  if (typeof document === "undefined") return;
  if (document.documentElement.dataset.uiMotion !== mode) document.documentElement.dataset.uiMotion = mode;
  const reduced = String(shouldReduceMotion(mode, media?.matches ?? false));
  if (document.documentElement.dataset.motionReduced !== reduced) document.documentElement.dataset.motionReduced = reduced;
  document.documentElement.style.setProperty("--page-enter-ms", shouldReduceMotion(mode, media?.matches ?? false) ? "0ms" : "200ms");
}
export function setUiMotion(next: UiMotion) {
  mode = normalizeUiMotion(next);
  try { localStorage.setItem(UI_MOTION_KEY, mode); } catch { /* 当前会话仍立即生效。 */ }
  apply(); listeners.forEach(listener => listener());
}
apply();
media?.addEventListener("change", apply);
if (typeof window !== "undefined") window.addEventListener("storage", event => {
  if (event.key !== UI_MOTION_KEY && event.key !== null) return;
  try { mode = normalizeUiMotion(localStorage.getItem(UI_MOTION_KEY)); } catch { return; }
  apply(); listeners.forEach(listener => listener());
});
export function useUiMotion() {
  const value = useSyncExternalStore(listener => { listeners.add(listener); return () => { listeners.delete(listener); }; }, () => mode, () => "standard" as UiMotion);
  return { motion: value, setMotion: setUiMotion };
}
