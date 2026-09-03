//! 仪表盘出口 IP 探测。
//!
//! 核心运行且不是直连模式时，请求通过 Satelite 的 mixed 入站；核心停止
//! 或路由模式为直连时，请求直接发出。多个公开接口并行竞争，首个可解析
//! 的结果返回，用于识别当前实际出口，而不是只显示节点延迟。

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExitIpInfo {
    pub ip: String,
    pub country_code: Option<String>,
    /// 是否通过核心 mixed 入站获取；false 表示本次探测为直连。
    pub via_proxy: bool,
    /// 胜出的公开接口地址，仅用于诊断。
    pub source: String,
}

#[derive(Clone, Copy)]
struct Source {
    url: &'static str,
    ip_field: &'static str,
    country_field: &'static str,
}

const SOURCES: &[Source] = &[
    Source {
        url: "https://api.ip.sb/geoip",
        ip_field: "ip",
        country_field: "country_code",
    },
    Source {
        url: "https://ipwho.is/",
        ip_field: "ip",
        country_field: "country_code",
    },
    Source {
        url: "http://ip-api.com/json",
        ip_field: "query",
        country_field: "countryCode",
    },
    Source {
        url: "https://api.myip.com",
        ip_field: "ip",
        country_field: "cc",
    },
];

const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";
const RACE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(12);

fn probe_source(
    source: &Source,
    proxy_port: Option<u16>,
) -> std::result::Result<(String, Option<String>), String> {
    let mut builder = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(9));
    if let Some(port) = proxy_port {
        let proxy = ureq::Proxy::new(format!("http://127.0.0.1:{port}"))
            .map_err(|e| format!("proxy: {e}"))?;
        builder = builder.proxy(proxy);
    }
    let response = builder
        .build()
        .get(source.url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/json")
        .call()
        .map_err(|e| format!("{}: {e}", source.url))?;
    let body = response.into_string().map_err(|e| format!("body: {e}"))?;
    parse_source(source, &body).ok_or_else(|| format!("{}: unparsable answer", source.url))
}

fn parse_source(source: &Source, body: &str) -> Option<(String, Option<String>)> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let ip = value.get(source.ip_field)?.as_str()?.trim().to_string();
    if ip.is_empty() {
        return None;
    }
    let country = value
        .get(source.country_field)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    Some((ip, country))
}

/// 并行探测多个公开 IP 接口。只返回首个成功结果，超时总上限为 12 秒。
pub async fn probe(mixed_port: u16, via_proxy: bool) -> std::result::Result<ExitIpInfo, String> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    for source in SOURCES {
        let tx = tx.clone();
        tokio::task::spawn_blocking(move || {
            let result = probe_source(source, via_proxy.then_some(mixed_port));
            let _ = tx.send((source.url, result));
        });
    }
    drop(tx);

    let started = std::time::Instant::now();
    let mut last_error = "all exit-ip sources failed".to_string();
    loop {
        let remaining = RACE_TIMEOUT.saturating_sub(started.elapsed());
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some((url, Ok((ip, country_code))))) => {
                return Ok(ExitIpInfo {
                    ip,
                    country_code,
                    via_proxy,
                    source: url.to_string(),
                });
            }
            Ok(Some((_, Err(error)))) => last_error = error,
            Ok(None) | Err(_) => return Err(last_error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(url: &str) -> Source {
        *SOURCES.iter().find(|source| source.url == url).unwrap()
    }

    #[test]
    fn parses_supported_response_shapes() {
        assert_eq!(
            parse_source(
                &source("https://api.ip.sb/geoip"),
                r#"{"ip":"104.28.7.9","country_code":"US"}"#,
            ),
            Some(("104.28.7.9".into(), Some("US".into())))
        );
        assert_eq!(
            parse_source(
                &source("http://ip-api.com/json"),
                r#"{"query":"5.6.7.8","countryCode":"DE"}"#,
            ),
            Some(("5.6.7.8".into(), Some("DE".into())))
        );
    }

    #[test]
    fn rejects_missing_or_empty_ip() {
        assert!(parse_source(&source("https://api.myip.com"), "not json").is_none());
        assert!(parse_source(&source("https://api.myip.com"), r#"{"ip":"","cc":"US"}"#,).is_none());
    }
}
