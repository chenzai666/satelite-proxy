use serde::{Deserialize, Serialize};

/// How a subscription was imported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SubscriptionSource {
    Url { url: String },
    File { path: String },
}

/// Traffic quota from `subscription-userinfo` header and/or remark node names.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubscriptionTraffic {
    /// Upload used (bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload: Option<u64>,
    /// Download used (bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<u64>,
    /// Total quota (bytes). 0 or missing means unlimited / unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// Explicit remaining (bytes) from remark nodes like `剩余流量：2.41 TB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_remaining: Option<u64>,
    /// Expire time as Unix timestamp (seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expire: Option<i64>,
    /// Human-readable expire when not a timestamp (e.g. `长期有效`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expire_text: Option<String>,
}

impl SubscriptionTraffic {
    pub fn used(&self) -> u64 {
        self.upload
            .unwrap_or(0)
            .saturating_add(self.download.unwrap_or(0))
    }

    pub fn remaining(&self) -> Option<u64> {
        if let Some(r) = self.quota_remaining {
            return Some(r);
        }
        let total = self.total.filter(|&t| t > 0)?;
        Some(total.saturating_sub(self.used()))
    }

    /// Used ratio 0.0–1.0 when total is known and > 0.
    pub fn used_ratio(&self) -> Option<f64> {
        let total = self.total.filter(|&t| t > 0)? as f64;
        if let Some(rem) = self.quota_remaining {
            let used = (total - rem as f64).max(0.0);
            return Some((used / total).clamp(0.0, 1.0));
        }
        Some((self.used() as f64 / total).clamp(0.0, 1.0))
    }

    pub fn is_empty(&self) -> bool {
        self.upload.is_none()
            && self.download.is_none()
            && self.total.is_none()
            && self.quota_remaining.is_none()
            && self.expire.is_none()
            && self.expire_text.is_none()
    }

    /// Prefer non-empty fields from `primary`, fill gaps from `fallback`.
    pub fn merge(primary: Option<Self>, fallback: Option<Self>) -> Option<Self> {
        match (primary, fallback) {
            (None, None) => None,
            (Some(a), None) | (None, Some(a)) => {
                if a.is_empty() {
                    None
                } else {
                    Some(a)
                }
            }
            (Some(mut a), Some(b)) => {
                if a.upload.is_none() {
                    a.upload = b.upload;
                }
                if a.download.is_none() {
                    a.download = b.download;
                }
                if a.total.is_none() {
                    a.total = b.total;
                }
                if a.quota_remaining.is_none() {
                    a.quota_remaining = b.quota_remaining;
                }
                if a.expire.is_none() {
                    a.expire = b.expire;
                }
                if a.expire_text.is_none() {
                    a.expire_text = b.expire_text;
                }
                if a.is_empty() {
                    None
                } else {
                    Some(a)
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub name: String,
    pub source: SubscriptionSource,
    /// Unix timestamp (seconds).
    pub last_update: i64,
    pub node_count: u32,
    pub enabled: bool,
    /// Detected format label, e.g. clash_yaml / uri_list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Nodes skipped on last import.
    #[serde(default)]
    pub skipped_count: u32,
    /// Fetch subscription URL via local mixed proxy (127.0.0.1:mixed_port).
    #[serde(default)]
    pub via_proxy: bool,
    /// Periodically re-fetch / re-read this profile.
    #[serde(default)]
    pub auto_update: bool,
    /// Auto-update interval in minutes (default 1440 = 24h). Minimum 1.
    #[serde(default = "default_auto_update_interval_min")]
    pub auto_update_interval_min: u32,
    /// Traffic / expire from last URL fetch (`subscription-userinfo`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic: Option<SubscriptionTraffic>,
}

fn default_auto_update_interval_min() -> u32 {
    1440
}

/// Summary returned to UI (URL masked in list).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionView {
    pub id: String,
    pub name: String,
    pub source_kind: String,
    /// Display-only source (URL may be redacted; file = basename).
    pub source_display: String,
    pub last_update: i64,
    pub node_count: u32,
    pub enabled: bool,
    pub format: Option<String>,
    pub skipped_count: u32,
    #[serde(default)]
    pub auto_update: bool,
    #[serde(default = "default_auto_update_interval_min")]
    pub auto_update_interval_min: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic: Option<SubscriptionTraffic>,
}

/// Full fields for edit form (includes raw URL / path).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionDetail {
    pub id: String,
    pub name: String,
    pub source_kind: String,
    pub url: Option<String>,
    pub path: Option<String>,
    pub last_update: i64,
    pub node_count: u32,
    pub enabled: bool,
    pub format: Option<String>,
    pub skipped_count: u32,
    pub via_proxy: bool,
    #[serde(default)]
    pub auto_update: bool,
    #[serde(default = "default_auto_update_interval_min")]
    pub auto_update_interval_min: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic: Option<SubscriptionTraffic>,
}

impl Subscription {
    pub fn to_view(&self) -> SubscriptionView {
        let (source_kind, source_display) = match &self.source {
            SubscriptionSource::Url { url } => ("url".into(), mask_url_for_display(url)),
            SubscriptionSource::File { path } => {
                let name = std::path::Path::new(path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(path)
                    .to_string();
                ("file".into(), name)
            }
        };
        SubscriptionView {
            id: self.id.clone(),
            name: self.name.clone(),
            source_kind,
            source_display,
            last_update: self.last_update,
            node_count: self.node_count,
            enabled: self.enabled,
            format: self.format.clone(),
            skipped_count: self.skipped_count,
            auto_update: self.auto_update,
            auto_update_interval_min: self.auto_update_interval_min.max(1),
            traffic: self.traffic.clone(),
        }
    }

    pub fn to_detail(&self) -> SubscriptionDetail {
        match &self.source {
            SubscriptionSource::Url { url } => SubscriptionDetail {
                id: self.id.clone(),
                name: self.name.clone(),
                source_kind: "url".into(),
                url: Some(url.clone()),
                path: None,
                last_update: self.last_update,
                node_count: self.node_count,
                enabled: self.enabled,
                format: self.format.clone(),
                skipped_count: self.skipped_count,
                via_proxy: self.via_proxy,
                auto_update: self.auto_update,
                auto_update_interval_min: self.auto_update_interval_min.max(1),
                traffic: self.traffic.clone(),
            },
            SubscriptionSource::File { path } => SubscriptionDetail {
                id: self.id.clone(),
                name: self.name.clone(),
                source_kind: "file".into(),
                url: None,
                path: Some(path.clone()),
                last_update: self.last_update,
                node_count: self.node_count,
                enabled: self.enabled,
                format: self.format.clone(),
                skipped_count: self.skipped_count,
                via_proxy: self.via_proxy,
                auto_update: self.auto_update,
                auto_update_interval_min: self.auto_update_interval_min.max(1),
                traffic: self.traffic.clone(),
            },
        }
    }

    pub fn is_auto_update_due(&self, now_secs: i64) -> bool {
        if !self.auto_update {
            return false;
        }
        let interval = (self.auto_update_interval_min.max(1) as i64).saturating_mul(60);
        now_secs.saturating_sub(self.last_update) >= interval
    }
}

/// Hide query string / token-looking tails for UI lists (not full secret storage).
fn mask_url_for_display(url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        let host = parsed.host_str().unwrap_or("?");
        let path = parsed.path();
        let short_path = if path.len() > 24 {
            format!("{}…", &path[..24])
        } else {
            path.to_string()
        };
        if parsed.query().is_some() {
            return format!("{}://{}{}?…", parsed.scheme(), host, short_path);
        }
        return format!("{}://{}{}", parsed.scheme(), host, short_path);
    }
    if url.len() > 48 {
        format!("{}…", &url[..48])
    } else {
        url.to_string()
    }
}
