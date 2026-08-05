/**
 * One-click subscribe deep links (Clash / sing-box).
 *
 * Supported:
 * - clash://install-config?url=<encoded>&name=<optional>
 * - sing-box://import-remote-profile?url=<encoded>#<encodedName>
 * - singbox://… (alias of sing-box)
 */

export interface ImportPrefill {
  url: string;
  name?: string;
}

function tryDecode(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

/** Parse a single deep-link URL into subscription prefill, or null if unrelated. */
export function parseImportDeepLink(raw: string): ImportPrefill | null {
  const href = raw.trim();
  if (!href) return null;

  let u: URL;
  try {
    u = new URL(href);
  } catch {
    return null;
  }

  const scheme = u.protocol.replace(/:$/, "").toLowerCase();
  // hostname is the first path segment for custom schemes (install-config, import-remote-profile).
  const action = (u.hostname || u.pathname.replace(/^\//, "").split("/")[0] || "")
    .toLowerCase();

  const nameFromQuery = u.searchParams.get("name");
  const nameFromHash =
    u.hash.length > 1 ? tryDecode(u.hash.slice(1).replace(/^\//, "")) : "";
  const nameRaw = nameFromQuery || nameFromHash || "";
  const name = nameRaw.trim() ? tryDecode(nameRaw.trim()) : undefined;

  if (scheme === "clash") {
    // clash://install-config?url=…&name=…
    if (
      action === "install-config" ||
      action === "install" ||
      action === "" ||
      u.searchParams.has("url")
    ) {
      const sub = u.searchParams.get("url");
      if (!sub?.trim()) return null;
      return { url: tryDecode(sub.trim()), name };
    }
    return null;
  }

  if (scheme === "sing-box" || scheme === "singbox") {
    // sing-box://import-remote-profile?url=…#name
    if (
      action === "import-remote-profile" ||
      action === "install-config" ||
      action === "import" ||
      action === "" ||
      u.searchParams.has("url")
    ) {
      const sub = u.searchParams.get("url");
      if (!sub?.trim()) return null;
      return { url: tryDecode(sub.trim()), name };
    }
    return null;
  }

  return null;
}

export function parseImportDeepLinks(urls: string[]): ImportPrefill | null {
  for (const raw of urls) {
    const p = parseImportDeepLink(raw);
    if (p) return p;
  }
  return null;
}
