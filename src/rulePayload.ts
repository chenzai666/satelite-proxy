import type { RuleType } from "./types";

/** Batch entries for the match-content textarea: whitespace (spaces or
 *  newlines) separates entries — one rule per entry on save. */
export function parsePayloadEntries(raw: string, type?: RuleType): string[] {
  return raw
    .split(type === "process" || type === "domain_keyword" ? /\r?\n/ : /\s+/)
    .map((s) => s.trim())
    .filter(Boolean);
}

/** Hostname label: alnum (unicode allowed — the backend punycodes it for the
 *  kernel configs), inner hyphens, no leading/trailing hyphen. */
const HOSTNAME_LABEL_RE = /^[\p{L}\p{N}]([\p{L}\p{N}-]*[\p{L}\p{N}])?$/u;

/** Domain / domain-suffix shape: dot-separated labels, no scheme, path,
 *  port or leading dot ("https://a.b" and ".com" both fail here). */
function isValidHostnameShape(v: string): boolean {
  return v.length <= 253 && v.split(".").every((l) => l.length <= 63 && HOSTNAME_LABEL_RE.test(l));
}

/** IPv4: exactly four 0–255 octets, no leading zeros ("0" alone is fine). */
function isValidIpv4(ip: string): boolean {
  const parts = ip.split(".");
  if (parts.length !== 4) return false;
  return parts.every((p) => /^(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)$/.test(p));
}

/** IPv6 via the WHATWG URL parser: `http://[...]` only parses for valid
 *  literals, in both WebView2 and WKWebView. */
function isValidIpv6(ip: string): boolean {
  try {
    new URL(`http://[${ip}]/`);
    return true;
  } catch {
    return false;
  }
}

/** IP-CIDR entry: bare IP or CIDR. Returns the normalized value — bare IPs
 *  gain /32 // /128, since mihomo's IP-CIDR rejects bare IPs — or null when
 *  invalid. Prefix must fit the address family (≤32 v4 / ≤128 v6). */
export function normalizeIpCidrEntry(v: string): string | null {
  const slash = v.indexOf("/");
  if (slash === -1) {
    if (isValidIpv4(v)) return `${v}/32`;
    if (isValidIpv6(v)) return `${v}/128`;
    return null;
  }
  if (v.indexOf("/", slash + 1) !== -1) return null;
  const ip = v.slice(0, slash);
  const prefix = v.slice(slash + 1);
  if (!/^\d{1,3}$/.test(prefix)) return null;
  const bits = Number(prefix);
  if (isValidIpv4(ip)) return bits <= 32 ? v : null;
  if (isValidIpv6(ip)) return bits <= 128 ? v : null;
  return null;
}

/** One invalid entry from the match-content textarea. */
interface PayloadIssue {
  index: number;
  value: string;
  kind: "domain" | "ip" | "process";
}

/** Validate batched match-content entries against the selected rule type;
 *  domain keywords are free-form and never fail. */
export function validatePayloadEntries(
  entries: string[],
  type: RuleType,
): PayloadIssue[] {
  const issues: PayloadIssue[] = [];
  entries.forEach((value, index) => {
    if (type === "domain" || type === "domain_suffix") {
      if (!isValidHostnameShape(value)) issues.push({ index, value, kind: "domain" });
    } else if (type === "ip_cidr") {
      if (normalizeIpCidrEntry(value) === null) issues.push({ index, value, kind: "ip" });
    } else if (type === "process") {
      if (/[/\\:]/.test(value)) issues.push({ index, value, kind: "process" });
    }
  });
  return issues;
}
