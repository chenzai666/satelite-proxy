//! 仪表盘出口 IP 探测。
//!
//! 核心运行时通过当前 HTTP/mixed 入站及实际规则；核心停止时直接发出。
//! 多个 HTTPS 公开接口并行竞争，首个可解析
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
        url: "https://www.cloudflare.com/cdn-cgi/trace",
        ip_field: "",
        country_field: "",
    },
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
        url: "https://api.myip.com",
        ip_field: "ip",
        country_field: "cc",
    },
];

const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";
const RACE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(12);

async fn probe_source(
    source: &Source,
    client: reqwest::Client,
) -> std::result::Result<(String, Option<String>), String> {
    let response = client
        .get(source.url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|_| "出口 IP 接口连接失败".to_string())?;
    if !response.status().is_success() {
        return Err("出口 IP 接口响应异常".into());
    }
    let body =
        super::http_body::read_limited(response, 32 * 1024, "出口 IP 响应过大".into()).await?;
    let body = std::str::from_utf8(&body).map_err(|_| "出口 IP 响应编码无效")?;
    parse_source(source, body).ok_or_else(|| "出口 IP 响应格式无效".into())
}

fn parse_source(source: &Source, body: &str) -> Option<(String, Option<String>)> {
    if source.ip_field.is_empty() {
        let field = |key: &str| {
            body.lines().find_map(|line| {
                let (name, value) = line.split_once('=')?;
                (name == key).then_some(value.trim())
            })
        };
        let ip = field("ip")?.parse::<std::net::IpAddr>().ok()?.to_string();
        return Some((ip, field("loc").and_then(normalize_country)));
    }
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let ip = value
        .get(source.ip_field)?
        .as_str()?
        .trim()
        .parse::<std::net::IpAddr>()
        .ok()?
        .to_string();
    let country = value
        .get(source.country_field)
        .and_then(serde_json::Value::as_str)
        .and_then(normalize_country);
    Some((ip, country))
}

fn normalize_country(value: &str) -> Option<String> {
    let code = value.trim();
    (code.len() == 2 && code.bytes().all(|b| b.is_ascii_alphabetic()))
        .then(|| code.to_ascii_uppercase())
}

/// 并行探测多个公开 IP 接口。只返回首个成功结果，超时总上限为 12 秒。
pub async fn probe(
    route: super::network_probe::ProbeRoute,
) -> std::result::Result<ExitIpInfo, String> {
    let client = super::network_probe::build_client(route, std::time::Duration::from_secs(9))?;
    let mut tasks = tokio::task::JoinSet::new();
    for source in SOURCES {
        let client = client.clone();
        tasks.spawn(async move { (source.url, probe_source(source, client).await) });
    }

    let started = std::time::Instant::now();
    let mut last_error = "all exit-ip sources failed".to_string();
    loop {
        let remaining = RACE_TIMEOUT.saturating_sub(started.elapsed());
        match tokio::time::timeout(remaining, tasks.join_next()).await {
            Ok(Some(Ok((url, Ok((ip, country_code)))))) => {
                tasks.shutdown().await;
                return Ok(ExitIpInfo {
                    ip,
                    country_code,
                    via_proxy: route.via_proxy(),
                    source: url.to_string(),
                });
            }
            Ok(Some(Ok((_, Err(error))))) => last_error = error,
            Ok(Some(Err(_))) => last_error = "出口检测任务结束".into(),
            Ok(None) | Err(_) => {
                tasks.shutdown().await;
                return Err(last_error);
            }
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
                &source("https://ipwho.is/"),
                r#"{"ip":"5.6.7.8","country_code":"DE"}"#,
            ),
            Some(("5.6.7.8".into(), Some("DE".into())))
        );
    }

    #[test]
    fn rejects_missing_or_empty_ip() {
        assert!(parse_source(&source("https://api.myip.com"), "not json").is_none());
        assert!(parse_source(&source("https://api.myip.com"), r#"{"ip":"","cc":"US"}"#,).is_none());
    }

    #[test]
    fn parses_lightweight_trace_without_trusting_html_or_invalid_country() {
        let trace = source("https://www.cloudflare.com/cdn-cgi/trace");
        assert_eq!(
            parse_source(&trace, "fl=123\nip=8.8.8.8\nloc=us\ntls=TLSv1.3\n"),
            Some(("8.8.8.8".into(), Some("US".into())))
        );
        assert_eq!(
            parse_source(&trace, "ip=2001:4860:4860::8888\nloc=UNKNOWN"),
            Some(("2001:4860:4860::8888".into(), None))
        );
        assert!(parse_source(&trace, "<html>403</html>").is_none());
        assert!(parse_source(&trace, "ip=invalid\nloc=US").is_none());
    }
}
