/** Accept node share links (or a v2rayN base64 link list), never fetch a pasted URL. */
const shareSchemes = new Set([
  "ss", "vmess", "vless", "trojan", "hysteria2", "hy2", "tuic",
  "socks", "socks5", "hysteria", "hy", "shadowtls", "ssh",
  "naive", "naive+https", "naive+quic", "tor", "anytls", "snell",
]);

function isShareLink(line: string): boolean {
  const match = /^([a-z][a-z0-9+.-]*):\/\//i.exec(line);
  if (!match) return false;
  const scheme = match[1].toLowerCase();
  if (shareSchemes.has(scheme)) return true;
  if (scheme !== "http" && scheme !== "https") return false;
  // Plain web/subscription URLs must not turn into proxy nodes on Ctrl+V.
  try {
    const url = new URL(line);
    // URL normalizes :80/:443 away, so inspect the original authority for
    // the explicit proxy port while letting URL validate the host/userinfo.
    const authority = line.slice(match[0].length).split(/[/?#]/, 1)[0];
    const endpoint = authority.slice(authority.lastIndexOf("@") + 1);
    return !!url.username && !!url.hostname && /:\d{1,5}$/.test(endpoint);
  } catch {
    return false;
  }
}

/** Returns importable text only when every non-comment line is a node link. */
export function clipboardNodePayload(text: string): string | null {
  const trimmed = text.trim();
  if (!trimmed || trimmed.length > 8 * 1024 * 1024 ||
      new TextEncoder().encode(trimmed).length > 8 * 1024 * 1024) return null;
  const lines = trimmed.split(/\r?\n/).map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#"));
  if (lines.length && lines.every(isShareLink)) return lines.join("\n");

  // Existing Rust import parser also accepts v2rayN's whole-body base64 URI
  // list. Decode only to recognize its shape; the Rust parser remains the
  // authority for protocol validation and persistence.
  const compact = trimmed.replace(/\s+/g, "");
  if (compact.length >= 24 && /^[A-Za-z0-9+/_=-]+$/.test(compact)) {
    try {
      const standard = compact.replace(/-/g, "+").replace(/_/g, "/");
      const bytes = Uint8Array.from(atob(standard.padEnd(Math.ceil(standard.length / 4) * 4, "=")),
        (character) => character.charCodeAt(0));
      const decoded = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      const decodedLines = decoded.split(/\r?\n/).map((line) => line.trim())
        .filter((line) => line && !line.startsWith("#"));
      if (decodedLines.length && decodedLines.every(isShareLink)) return trimmed;
    } catch {
      // Not an encoded node list; leave clipboard text untouched.
    }
  }
  return null;
}
