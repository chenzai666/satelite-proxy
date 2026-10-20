import { clipboardNodePayload } from "./nodeClipboard.ts";
import { parseImportDeepLink } from "./deepLink.ts";

export type QrImportPayload = { kind: "text"; content: string } | { kind: "url"; url: string; name?: string };

/** Recognition only: no fetch, opening external links, or implicit import. */
export function qrImportPayload(raw: string): QrImportPayload | null {
  if (raw.length > 256 * 1024) return null;
  const text = raw.trim();
  if (!text) return null;
  const deepLink = parseImportDeepLink(text);
  const candidate = deepLink?.url ?? text;
  const nodes = !deepLink && clipboardNodePayload(text);
  if (nodes) return { kind: "text", content: nodes };
  try {
    const url = new URL(candidate);
    if (!["http:", "https:"].includes(url.protocol) || !url.hostname || url.username || url.password) return null;
    return { kind: "url", url: candidate, name: deepLink?.name };
  } catch { return null; }
}

export function shouldScanQrScreen(event: Pick<KeyboardEvent,
  "key" | "repeat" | "defaultPrevented" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">,
  editing: boolean, blocked: boolean): boolean {
  return event.key.toLowerCase() === "s" && (event.ctrlKey || event.metaKey) &&
    !event.altKey && !event.shiftKey && !event.repeat && !event.defaultPrevented && !editing && !blocked;
}
