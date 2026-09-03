import { useEffect } from "react";
import { restartProxy } from "../api";

const isMac = /Mac|iPhone|iPad/i.test(navigator.userAgent);

/** 输入框获得焦点时不拦截会影响输入的快捷键。 */
function isTypingTarget(el: Element | null): boolean {
  if (!el) return false;
  const tag = el.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  return (el as HTMLElement).isContentEditable;
}

/**
 * 提供 Cmd/Ctrl+数字切页、Cmd/Ctrl+, 打开设置和 Cmd/Ctrl+R 重启内核。
 * 输入框、文本框和可编辑元素聚焦时不处理，避免影响正常输入。
 */
export function useGlobalShortcuts<K extends string>(
  navMap: Partial<Record<string, K>>,
  onNav: (key: K) => void,
  settingsKey: K,
) {
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const modifier = isMac ? event.metaKey : event.ctrlKey;
      if (!modifier || event.altKey) return;
      if (isTypingTarget(document.activeElement)) return;

      if (event.key === ",") {
        event.preventDefault();
        onNav(settingsKey);
        return;
      }
      if (event.key === "r" || event.key === "R") {
        event.preventDefault();
        void restartProxy();
        return;
      }
      const target = navMap[event.key];
      if (target) {
        event.preventDefault();
        onNav(target);
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [navMap, onNav, settingsKey]);
}
