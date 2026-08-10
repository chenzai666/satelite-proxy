use crate::domain::{
    ParseResult, ProxyNode, Subscription, SubscriptionFormat, SubscriptionSource,
    SubscriptionTraffic,
};
use crate::error::{AppError, AppResult};
use crate::subscription::parse_subscription;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

pub struct ImportOutcome {
    pub subscription: Subscription,
    pub nodes: Vec<ProxyNode>,
}

/// `via_proxy`: fetch through local mixed HTTP proxy (127.0.0.1:mixed_port).
/// `mixed_port`: required when via_proxy is true.
pub async fn import_from_url_with_id(
    name: Option<String>,
    url: String,
    existing_id: Option<String>,
    via_proxy: bool,
    mixed_port: Option<u16>,
) -> AppResult<ImportOutcome> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err(AppError::Fetch("url is empty".into()));
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(AppError::Fetch(
            "url must start with http:// or https://".into(),
        ));
    }

    // Many panels only attach `subscription-userinfo` when UA looks like Clash.
    // FlClash default: `{app}/v{ver} clash-verge Platform/{os}` — we mirror that.
    let ua = subscription_user_agent();

    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent(ua);

    if via_proxy {
        let port = mixed_port.unwrap_or(2080);
        let proxy_url = format!("http://127.0.0.1:{port}");
        let proxy = reqwest::Proxy::all(&proxy_url)
            .map_err(|e| AppError::Fetch(format!("invalid proxy {proxy_url}: {e}")))?;
        builder = builder.proxy(proxy);
    } else {
        builder = builder.no_proxy();
    }

    let client = builder
        .build()
        .map_err(|e| AppError::Fetch(e.to_string()))?;

    let response = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "*/*")
        .send()
        .await
        .map_err(|e| {
            if via_proxy {
                AppError::Fetch(format!(
                    "via proxy failed ({e}). 请确认已启动代理核心，且 mixed 端口正确"
                ))
            } else {
                AppError::Fetch(e.to_string())
            }
        })?;

    if !response.status().is_success() {
        return Err(AppError::Fetch(format!(
            "http status {}",
            response.status()
        )));
    }

    let traffic = parse_subscription_userinfo(response.headers());
    // Default label from Content-Disposition (RFC 5987 filename*), same as FlClash.
    let disposition_name = parse_content_disposition_filename(response.headers());

    let bytes = response
        .bytes()
        .await
        .map_err(|e| AppError::Fetch(e.to_string()))?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err(AppError::Fetch(format!(
            "body too large ({} bytes, max {})",
            bytes.len(),
            MAX_BODY_BYTES
        )));
    }

    let content = String::from_utf8_lossy(&bytes).into_owned();
    let body_traffic = parse_userinfo_from_content(&content);
    let parsed = parse_subscription(&content)?;
    // Name priority: user input > Content-Disposition filename* > URL host
    let display_name = name
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .or(disposition_name)
        .unwrap_or_else(|| name_from_url(&url));

    let mut outcome = build_outcome(
        display_name,
        SubscriptionSource::Url { url },
        parsed,
        existing_id,
    );
    outcome.subscription.via_proxy = via_proxy;
    // Priority: HTTP header > body comment > remark node names
    outcome.subscription.traffic = SubscriptionTraffic::merge(
        traffic,
        SubscriptionTraffic::merge(body_traffic, outcome.subscription.traffic),
    );
    Ok(outcome)
}

/// UA used when fetching subscriptions (must look like Clash for many panels).
fn subscription_user_agent() -> String {
    let os = std::env::consts::OS;
    // Same shape as FlClash `PackageInfo.ua`: include `clash-verge`.
    format!("SateliteProxy/0.1 clash-verge Platform/{os}")
}

/// Parse `Content-Disposition` for a display name.
/// Supports `filename*=UTF-8''%E8%89%AF%E5%BF%83%E4%BA%91` (percent-encoded) and
/// plain `filename="foo.yaml"`. Matches FlClash `getFileNameForDisposition`.
pub fn parse_content_disposition_filename(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let raw = headers
        .get(reqwest::header::CONTENT_DISPOSITION)
        .or_else(|| headers.get("content-disposition"))
        .and_then(header_value_to_string)?;
    parse_disposition_filename_str(&raw)
}

fn parse_disposition_filename_str(disposition: &str) -> Option<String> {
    // Prefer RFC 5987: filename*=charset'lang'value  or  filename*=UTF-8''urlencoded
    if let Some(star) = find_disposition_param(disposition, "filename*") {
        let decoded = decode_filename_star(&star)?;
        let cleaned = clean_disposition_name(&decoded);
        if !cleaned.is_empty() {
            return Some(cleaned);
        }
    }
    // Fallback: filename="..." / filename=...
    if let Some(plain) = find_disposition_param(disposition, "filename") {
        let unquoted = plain
            .trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .to_string();
        // Some servers still percent-encode plain filename
        let decoded = urlencoding::decode(&unquoted)
            .map(|c| c.into_owned())
            .unwrap_or(unquoted);
        let cleaned = clean_disposition_name(&decoded);
        if !cleaned.is_empty() {
            return Some(cleaned);
        }
    }
    None
}

/// Extract parameter value from Content-Disposition; handles `filename*=...` vs `filename=...`.
fn find_disposition_param(disposition: &str, key: &str) -> Option<String> {
    let lower_key = key.to_ascii_lowercase();
    for part in disposition.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        if k.trim().eq_ignore_ascii_case(&lower_key) {
            return Some(v.trim().to_string());
        }
    }
    None
}

/// Decode `UTF-8''%E8%89%AF...` or bare percent-encoded string.
fn decode_filename_star(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_matches(|c| c == '"' || c == '\'');
    // charset'lang'value  — lang often empty: UTF-8''xxx
    let value = if let Some(idx) = raw.find("''") {
        &raw[idx + 2..]
    } else if let Some((_, rest)) = raw.split_once('\'') {
        // charset'value without empty lang
        rest.trim_start_matches('\'')
    } else {
        raw
    };
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    // RFC 5987 uses percent-encoding
    match urlencoding::decode(value) {
        Ok(cow) => {
            let s = cow.into_owned();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
        Err(_) => {
            // Fallback: try percent_decode via from_utf8 lossy
            let bytes: Vec<u8> = {
                let mut out = Vec::new();
                let b = value.as_bytes();
                let mut i = 0;
                while i < b.len() {
                    if b[i] == b'%' && i + 2 < b.len() {
                        let h = std::str::from_utf8(&b[i + 1..i + 3]).ok();
                        if let Some(h) = h {
                            if let Ok(n) = u8::from_str_radix(h, 16) {
                                out.push(n);
                                i += 3;
                                continue;
                            }
                        }
                    }
                    out.push(b[i]);
                    i += 1;
                }
                out
            };
            let s = String::from_utf8_lossy(&bytes).into_owned();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
    }
}

fn clean_disposition_name(name: &str) -> String {
    let mut s = name.trim().to_string();
    // Drop path components if any
    if let Some(base) = s.rsplit(['/', '\\']).next() {
        s = base.to_string();
    }
    // Strip common subscription file extensions for a nicer label
    for ext in [".yaml", ".yml", ".txt", ".conf", ".json"] {
        if s.to_ascii_lowercase().ends_with(ext) {
            s.truncate(s.len() - ext.len());
            break;
        }
    }
    s.trim().to_string()
}

/// Parse Clash-style `subscription-userinfo` header:
/// `upload=…; download=…; total=…; expire=…` (values in bytes / unix seconds).
pub fn parse_subscription_userinfo(
    headers: &reqwest::header::HeaderMap,
) -> Option<SubscriptionTraffic> {
    // 1) Standard name (HeaderMap is case-insensitive).
    if let Some(raw) = header_values_joined(headers, "subscription-userinfo") {
        if let Some(t) = parse_userinfo_str(&raw) {
            return Some(t);
        }
    }
    // 2) Some panels use slightly different names.
    for name in [
        "subscription-userinfo",
        "x-subscription-userinfo",
        "subscription-user-info",
    ] {
        if let Some(raw) = header_values_joined(headers, name) {
            if let Some(t) = parse_userinfo_str(&raw) {
                return Some(t);
            }
        }
    }
    // 3) Scan any header whose name contains "userinfo".
    for (name, value) in headers.iter() {
        let n = name.as_str().to_ascii_lowercase();
        if !n.contains("userinfo") {
            continue;
        }
        if let Some(raw) = header_value_to_string(value) {
            if let Some(t) = parse_userinfo_str(&raw) {
                return Some(t);
            }
        }
    }
    None
}

fn header_values_joined(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    let values: Vec<String> = headers
        .get_all(name)
        .iter()
        .filter_map(header_value_to_string)
        .collect();
    if values.is_empty() {
        None
    } else {
        Some(values.join("; "))
    }
}

fn header_value_to_string(v: &reqwest::header::HeaderValue) -> Option<String> {
    if let Ok(s) = v.to_str() {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        return Some(s.to_string());
    }
    // Non-strict ASCII: still try (some middleboxes corrupt encoding).
    let s = String::from_utf8_lossy(v.as_bytes());
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Some providers put the same key=value list in a leading YAML comment:
/// `# upload=…; download=…; total=…; expire=…`
pub fn parse_userinfo_from_content(content: &str) -> Option<SubscriptionTraffic> {
    for line in content.lines().take(32) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let body = match line.strip_prefix('#') {
            Some(rest) => rest.trim().trim_end_matches(';').trim(),
            None => {
                // Stop after first non-comment content line (except pure whitespace already skipped).
                // Still allow a few leading blank/comment-only lines above.
                if line.starts_with("proxies")
                    || line.starts_with("port:")
                    || line.starts_with("mixed-port")
                    || line.starts_with("---")
                {
                    break;
                }
                continue;
            }
        };
        if body.is_empty() {
            continue;
        }
        let lower = body.to_ascii_lowercase();
        if !(lower.contains("upload=")
            || lower.contains("download=")
            || lower.contains("total=")
            || lower.contains("expire="))
        {
            continue;
        }
        if let Some(t) = parse_userinfo_str(body) {
            return Some(t);
        }
    }
    None
}

fn parse_userinfo_str(raw: &str) -> Option<SubscriptionTraffic> {
    let mut traffic = SubscriptionTraffic::default();
    // FlClash: split by `;`, then `key=value` (also tolerate commas).
    for part in raw.split([';', ',']) {
        let part = part.trim().trim_end_matches(';');
        if part.is_empty() {
            continue;
        }
        let (k, v) = match part.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };
        let key = k.trim().to_ascii_lowercase();
        // Tolerate spaces / quotes: " 1073741824000 "
        let val = v
            .trim()
            .trim_matches(|c: char| c == '"' || c == '\'')
            .trim();
        // Some panels send floats as strings — take integer part.
        let parse_u64 = |s: &str| -> Option<u64> {
            if let Ok(n) = s.parse::<u64>() {
                return Some(n);
            }
            s.parse::<f64>().ok().map(|f| f.max(0.0).round() as u64)
        };
        let parse_i64 = |s: &str| -> Option<i64> {
            if let Ok(n) = s.parse::<i64>() {
                return Some(n);
            }
            s.parse::<f64>().ok().map(|f| f as i64)
        };
        match key.as_str() {
            "upload" => traffic.upload = parse_u64(val),
            "download" => traffic.download = parse_u64(val),
            "total" => traffic.total = parse_u64(val),
            "expire" => traffic.expire = parse_i64(val),
            _ => {}
        }
    }
    if traffic.is_empty() {
        None
    } else {
        Some(traffic)
    }
}

/// Providers often inject fake proxies whose **names** carry quota text, e.g.
/// `剩余流量：2.41 TB` / `套餐到期：长期有效`. Extract traffic and drop them.
fn split_remark_nodes(nodes: Vec<ProxyNode>) -> (Option<SubscriptionTraffic>, Vec<ProxyNode>) {
    let mut traffic = SubscriptionTraffic::default();
    let mut real = Vec::with_capacity(nodes.len());
    for n in nodes {
        if apply_remark_name(&n.name, &mut traffic) {
            continue;
        }
        real.push(n);
    }
    let traffic = if traffic.is_empty() {
        None
    } else {
        Some(traffic)
    };
    (traffic, real)
}

/// Returns true if `name` is a remark / info node (should not be a real proxy).
fn apply_remark_name(name: &str, traffic: &mut SubscriptionTraffic) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return false;
    }

    // 剩余流量：2.41 TB / 流量剩余: 100GB / 余量：…
    if let Some(rest) = strip_label(
        name,
        &[
            "剩余流量",
            "流量剩余",
            "剩余额度",
            "余量",
            "流量余量",
            "剩余",
        ],
    ) {
        if let Some(bytes) = parse_size_to_bytes(rest) {
            if traffic.quota_remaining.is_none() {
                traffic.quota_remaining = Some(bytes);
            }
            return true;
        }
        // "剩余：无限" etc.
        if is_unlimited_text(rest) {
            return true;
        }
    }

    // 已用流量 / 已使用
    if let Some(rest) = strip_label(name, &["已用流量", "已使用流量", "已用", "已使用"])
    {
        if let Some(bytes) = parse_size_to_bytes(rest) {
            if traffic.download.is_none() && traffic.upload.is_none() {
                traffic.download = Some(bytes);
            }
            return true;
        }
    }

    // 总流量 / 套餐流量
    if let Some(rest) = strip_label(name, &["总流量", "套餐流量", "流量总量", "总量"])
    {
        if let Some(bytes) = parse_size_to_bytes(rest) {
            if traffic.total.is_none() {
                traffic.total = Some(bytes);
            }
            return true;
        }
        if is_unlimited_text(rest) {
            return true;
        }
    }

    // 套餐到期 / 到期时间 / 过期时间
    if let Some(rest) = strip_label(
        name,
        &[
            "套餐到期",
            "到期时间",
            "过期时间",
            "到期日",
            "有效期至",
            "有效期",
            "到期",
            "Expire",
            "expire",
        ],
    ) {
        let rest = rest.trim();
        if rest.is_empty() {
            return true;
        }
        if traffic.expire.is_none() {
            if let Some(ts) = parse_expire_timestamp(rest) {
                traffic.expire = Some(ts);
            } else if traffic.expire_text.is_none() {
                traffic.expire_text = Some(rest.to_string());
            }
        } else if traffic.expire_text.is_none() && parse_expire_timestamp(rest).is_none() {
            traffic.expire_text = Some(rest.to_string());
        }
        return true;
    }

    // English-ish
    let lower = name.to_ascii_lowercase();
    if lower.contains("traffic reset")
        || lower.contains("package expired")
        || lower.starts_with("expire")
        || lower.contains("remaining traffic")
        || lower.contains("traffic remaining")
    {
        if let Some(rest) = name.split_once([':', '：']).map(|(_, r)| r.trim()) {
            if let Some(bytes) = parse_size_to_bytes(rest) {
                if traffic.quota_remaining.is_none()
                    && (lower.contains("remaining") || lower.contains("left"))
                {
                    traffic.quota_remaining = Some(bytes);
                }
            }
            if traffic.expire_text.is_none()
                && (lower.contains("expire") || lower.contains("expired"))
            {
                traffic.expire_text = Some(rest.to_string());
            }
        }
        return true;
    }

    // Bare info labels without value (官网 / 更新订阅 / 公告…)
    if is_pure_info_label(name) {
        return true;
    }

    false
}

fn strip_label<'a>(name: &'a str, labels: &[&str]) -> Option<&'a str> {
    for label in labels {
        if let Some(rest) = name.strip_prefix(label) {
            let rest = rest.trim_start_matches(['：', ':', ' ', '\t', '-', '—']);
            return Some(rest);
        }
        // allow "【剩余流量】2.41 TB"
        let wrapped = format!("【{label}】");
        if let Some(rest) = name.strip_prefix(&wrapped) {
            return Some(rest.trim());
        }
        let wrapped2 = format!("[{label}]");
        if let Some(rest) = name.strip_prefix(&wrapped2) {
            return Some(rest.trim());
        }
    }
    None
}

fn is_unlimited_text(s: &str) -> bool {
    let t = s.trim();
    t == "无限" || t == "无限制" || t.eq_ignore_ascii_case("unlimited") || t == "∞"
}

fn is_pure_info_label(name: &str) -> bool {
    let n = name.trim();
    matches!(
        n,
        "官网"
            | "官方网站"
            | "更新"
            | "更新订阅"
            | "公告"
            | "说明"
            | "教程"
            | "测速"
            | "Traffic"
            | "Expire"
    ) || n.starts_with("官网")
        || n.starts_with("http://")
        || n.starts_with("https://")
}

/// Parse `2.41 TB`, `2.41TB`, `1000G`, `512 MB` → bytes (binary 1024).
fn parse_size_to_bytes(s: &str) -> Option<u64> {
    let s = s.trim().replace(',', "");
    if s.is_empty() || is_unlimited_text(&s) {
        return None;
    }
    // number + optional unit
    let bytes = s.as_bytes();
    let mut i = 0;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        i += 1;
    }
    let start = i;
    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
        i += 1;
    }
    if i == start {
        return None;
    }
    let num: f64 = std::str::from_utf8(&bytes[start..i]).ok()?.parse().ok()?;
    if !num.is_finite() || num < 0.0 {
        return None;
    }
    let unit = s[i..].trim().to_ascii_lowercase().replace(' ', "");
    let mult: f64 = match unit.as_str() {
        "" | "b" | "byte" | "bytes" => 1.0,
        "k" | "kb" | "kib" => 1024.0,
        "m" | "mb" | "mib" => 1024.0_f64.powi(2),
        "g" | "gb" | "gib" => 1024.0_f64.powi(3),
        "t" | "tb" | "tib" => 1024.0_f64.powi(4),
        "p" | "pb" | "pib" => 1024.0_f64.powi(5),
        _ => return None,
    };
    Some((num * mult).round() as u64)
}

fn parse_expire_timestamp(s: &str) -> Option<i64> {
    let s = s.trim();
    // pure unix seconds
    if let Ok(n) = s.parse::<i64>() {
        if n > 1_000_000_000 {
            return Some(n);
        }
    }
    // YYYY-MM-DD / YYYY/MM/DD / YYYY.MM.DD [HH:MM[:SS]]
    let normalized = s.replace(['/', '.'], "-");
    let date_part = normalized.split_whitespace().next().unwrap_or(&normalized);
    let mut parts = date_part.split('-');
    let y: i32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if !(1970..=2100).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Approximate UTC midnight via days since epoch (good enough for display).
    let days = days_from_civil(y, m as i32, d as i32)?;
    Some(days * 86400)
}

/// Howard Hinnant civil-from-days inverse (proleptic Gregorian) → days since 1970-01-01.
fn days_from_civil(y: i32, m: i32, d: i32) -> Option<i64> {
    if m < 1 || m > 12 || d < 1 || d > 31 {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146097 + doe as i64) - 719468)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_userinfo_basic() {
        let t = parse_userinfo_str(
            "upload=1073741824; download=2147483648; total=1073741824000; expire=1893456000",
        )
        .expect("parsed");
        assert_eq!(t.upload, Some(1_073_741_824));
        assert_eq!(t.download, Some(2_147_483_648));
        assert_eq!(t.total, Some(1_073_741_824_000));
        assert_eq!(t.expire, Some(1_893_456_000));
        assert_eq!(t.used(), 3_221_225_472);
        assert!(t.used_ratio().unwrap() < 0.01);
        assert!(t.remaining().unwrap() > 1_000_000_000_000);
    }

    #[test]
    fn parse_userinfo_like_flclash() {
        // Same string shape FlClash tests use
        let t = parse_userinfo_str("upload=10; download=20; total=100; expire=200").unwrap();
        assert_eq!(t.upload, Some(10));
        assert_eq!(t.download, Some(20));
        assert_eq!(t.total, Some(100));
        assert_eq!(t.used(), 30);
        assert!((t.used_ratio().unwrap() - 0.3).abs() < 1e-9);
    }

    #[test]
    fn subscription_ua_contains_clash_verge() {
        let ua = subscription_user_agent();
        assert!(ua.to_ascii_lowercase().contains("clash-verge"), "ua={ua}");
    }

    #[test]
    fn parse_disposition_filename_star_utf8() {
        // 良心云
        let d = "attachment;filename*=UTF-8''%E8%89%AF%E5%BF%83%E4%BA%91";
        let name = parse_disposition_filename_str(d).expect("name");
        assert_eq!(name, "良心云");
    }

    #[test]
    fn parse_disposition_filename_star_with_ext() {
        let d = r#"attachment; filename*=UTF-8''%E6%B5%8B%E8%AF%95.yaml"#;
        let name = parse_disposition_filename_str(d).expect("name");
        assert_eq!(name, "测试");
    }

    #[test]
    fn parse_disposition_filename_plain() {
        let d = r#"attachment; filename="my-sub.yaml""#;
        let name = parse_disposition_filename_str(d).expect("name");
        assert_eq!(name, "my-sub");
    }

    #[test]
    fn parse_disposition_prefers_star_over_plain() {
        let d = r#"attachment; filename="fallback.yaml"; filename*=UTF-8''%E4%BC%98%E5%85%88"#;
        let name = parse_disposition_filename_str(d).expect("name");
        assert_eq!(name, "优先");
    }

    #[test]
    fn parse_userinfo_empty() {
        assert!(parse_userinfo_str("").is_none());
        assert!(parse_userinfo_str("foo=bar").is_none());
    }

    #[test]
    fn parse_userinfo_from_yaml_comment() {
        let yaml = r#"# upload=455727941; download=6174315083; total=1073741824000; expire=1671815872;

proxies:
  - name: a
    type: ss
"#;
        let t = parse_userinfo_from_content(yaml).expect("body comment");
        assert_eq!(t.upload, Some(455_727_941));
        assert_eq!(t.download, Some(6_174_315_083));
        assert_eq!(t.total, Some(1_073_741_824_000));
        assert_eq!(t.expire, Some(1_671_815_872));
    }

    #[test]
    fn parse_size_tb() {
        let b = parse_size_to_bytes("2.41 TB").unwrap();
        assert!((b as f64 - 2.41 * 1024f64.powi(4)).abs() < 1024.0 * 1024.0);
        assert_eq!(parse_size_to_bytes("1000G").unwrap(), 1000 * 1024u64.pow(3));
    }

    #[test]
    fn remark_remaining_and_expire() {
        let mut t = SubscriptionTraffic::default();
        assert!(apply_remark_name("剩余流量：2.41 TB", &mut t));
        assert!(apply_remark_name("套餐到期：长期有效", &mut t));
        assert_eq!(t.expire_text.as_deref(), Some("长期有效"));
        let rem = t.quota_remaining.unwrap();
        assert!(rem > 2 * 1024u64.pow(4));
        assert!(rem < 3 * 1024u64.pow(4));
    }

    #[test]
    fn split_filters_remark_nodes() {
        use crate::domain::{Protocol, ProtocolConfig, ProxyNode};
        let mk = |name: &str| ProxyNode {
            id: name.into(),
            name: name.into(),
            protocol: Protocol::Vless,
            server: "cfyes.example.com".into(),
            port: 443,
            tls: None,
            transport: None,
            udp: None,
            config: ProtocolConfig::Vless {
                uuid: "x".into(),
                flow: None,
                packet_encoding: "xudp".into(),
            },
            source: None,
            latency_ms: None,
            latency_at: None,
        };
        let (traffic, real) = split_remark_nodes(vec![
            mk("剩余流量：2.41 TB"),
            mk("套餐到期：长期有效"),
            mk("HK-01"),
        ]);
        assert_eq!(real.len(), 1);
        assert_eq!(real[0].name, "HK-01");
        let t = traffic.unwrap();
        assert!(t.quota_remaining.is_some());
        assert_eq!(t.expire_text.as_deref(), Some("长期有效"));
    }
}

pub fn import_from_file(name: Option<String>, path: &Path) -> AppResult<ImportOutcome> {
    import_from_file_with_id(name, path, None)
}

pub fn import_from_file_with_id(
    name: Option<String>,
    path: &Path,
    existing_id: Option<String>,
) -> AppResult<ImportOutcome> {
    if !path.exists() {
        return Err(AppError::Io(format!("file not found: {}", path.display())));
    }
    let meta = std::fs::metadata(path)?;
    if meta.len() as usize > MAX_BODY_BYTES {
        return Err(AppError::Io(format!(
            "file too large ({} bytes, max {})",
            meta.len(),
            MAX_BODY_BYTES
        )));
    }
    let content = std::fs::read_to_string(path)?;
    let body_traffic = parse_userinfo_from_content(&content);
    let parsed = parse_subscription(&content)?;
    let path_str = path.to_string_lossy().to_string();
    let display_name = name
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Local config")
                .to_string()
        });

    let mut outcome = build_outcome(
        display_name,
        SubscriptionSource::File { path: path_str },
        parsed,
        existing_id,
    );
    // File: body comment > remark nodes
    outcome.subscription.traffic =
        SubscriptionTraffic::merge(body_traffic, outcome.subscription.traffic);
    Ok(outcome)
}

fn build_outcome(
    name: String,
    source: SubscriptionSource,
    parsed: ParseResult,
    existing_id: Option<String>,
) -> ImportOutcome {
    let id = existing_id.unwrap_or_else(|| subscription_id(&source));
    let format = format_label(parsed.format);
    let skipped = parsed.skipped.len();
    let (remark_traffic, real_nodes) = split_remark_nodes(parsed.nodes);
    let node_count = real_nodes.len() as u32;
    let subscription = Subscription {
        id,
        name,
        source,
        last_update: now_secs(),
        node_count,
        enabled: true,
        format: Some(format),
        skipped_count: skipped as u32,
        via_proxy: false,
        auto_update: false,
        auto_update_interval_min: 1440,
        traffic: remark_traffic,
    };

    // Re-hash node ids with subscription scope for multi-sub stability.
    let sub_id = subscription.id.clone();
    let nodes: Vec<ProxyNode> = real_nodes
        .into_iter()
        .map(|mut n| {
            n.id = ProxyNode::compute_id(
                &format!("{sub_id}|{}", n.name),
                &n.server,
                n.port,
                n.protocol,
            );
            // latency filled later by probe; clear on fresh parse
            n.latency_ms = None;
            n.latency_at = None;
            n
        })
        .collect();
    ImportOutcome {
        subscription,
        nodes,
    }
}

fn subscription_id(source: &SubscriptionSource) -> String {
    let mut hasher = Sha256::new();
    match source {
        SubscriptionSource::Url { url } => {
            hasher.update(b"url|");
            hasher.update(url.as_bytes());
        }
        SubscriptionSource::File { path } => {
            hasher.update(b"file|");
            hasher.update(path.as_bytes());
        }
    }
    let digest = hasher.finalize();
    hex::encode(&digest[..16])
}

fn name_from_url(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| "Subscription".into())
}

fn format_label(f: SubscriptionFormat) -> String {
    match f {
        SubscriptionFormat::ClashYaml => "clash_yaml".into(),
        SubscriptionFormat::UriList => "uri_list".into(),
        SubscriptionFormat::Base64UriList => "base64_uri_list".into(),
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
