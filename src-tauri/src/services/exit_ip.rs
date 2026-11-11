//! 仪表盘出口 IP 探测。
//!
//! 核心运行时通过当前 HTTP/mixed 入站及实际规则；核心停止时直接发出。
//! 按 FlClash 的来源顺序错峰探测；返回第一个有效的公网地址及实际来源。

use serde::Serialize;
use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::task::JoinSet;
use tokio::time::{sleep_until, Instant};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExitIpInfo {
    pub ip: String,
    pub country_code: Option<String>,
    /// 是否通过核心 mixed 入站获取；false 表示本次探测为直连。
    pub via_proxy: bool,
    /// 胜出的公开接口地址，仅用于诊断。
    pub source: String,
    /// 本次探测取得结果的时间，缓存复用时保留原时间。
    pub checked_at: u64,
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
        country_field: "/country_code",
    },
    Source {
        url: "https://ipinfo.io/json",
        ip_field: "ip",
        country_field: "/country",
    },
    Source {
        url: "https://ipwho.is/",
        ip_field: "ip",
        country_field: "/country_code",
    },
    Source {
        url: "https://get.geojs.io/v1/ip/geo.json",
        ip_field: "ip",
        country_field: "/country_code",
    },
    Source {
        url: "https://api.country.is",
        ip_field: "ip",
        country_field: "/country",
    },
    Source {
        url: "https://api.ipquery.io/?format=json",
        ip_field: "ip",
        country_field: "/location/country_code",
    },
    Source {
        url: "https://ident.me/json",
        ip_field: "ip",
        country_field: "/cc",
    },
];

const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";
const RACE_TIMEOUT: Duration = Duration::from_secs(6);
const SOURCE_STAGGER: Duration = Duration::from_millis(800);
const MAX_IN_FLIGHT: usize = 3;
const MAX_BODY: usize = 4096;
type SourceAnswer = Result<(String, Option<String>), String>;

async fn probe_source(source: &Source, client: reqwest::Client) -> SourceAnswer {
    let response = client
        .get(source.url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|_| "出口 IP 接口连接失败".to_string())?;
    if response.status() != reqwest::StatusCode::OK {
        return Err("出口 IP 接口响应异常".into());
    }
    let body =
        super::http_body::read_limited(response, MAX_BODY, "出口 IP 响应过大".into()).await?;
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
        let ip = super::ip_metadata::validate_ip(field("ip")?)
            .ok()?
            .to_string();
        return Some((ip, field("loc").and_then(normalize_country)));
    }
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    if value.get("success").and_then(serde_json::Value::as_bool) == Some(false) {
        return None;
    }
    let ip = super::ip_metadata::validate_ip(value.get(source.ip_field)?.as_str()?.trim())
        .ok()?
        .to_string();
    let country = value
        .pointer(source.country_field)
        .and_then(serde_json::Value::as_str)
        .and_then(normalize_country);
    Some((ip, country))
}

fn normalize_country(value: &str) -> Option<String> {
    let code = value.trim();
    (code.len() == 2 && code.bytes().all(|b| b.is_ascii_alphabetic()))
        .then(|| code.to_ascii_uppercase())
}

/// 先查首个来源，慢时错峰增加备用来源；失败立刻补位，最多同时三路。
pub async fn probe(
    route: super::network_probe::ProbeRoute,
) -> std::result::Result<ExitIpInfo, String> {
    let client = super::network_probe::build_client(route, RACE_TIMEOUT)?;
    let (source, (ip, country_code)) = race_sources(SOURCE_STAGGER, RACE_TIMEOUT, move |source| {
        let client = client.clone();
        async move { probe_source(&source, client).await }
    })
    .await?;
    Ok(ExitIpInfo {
        ip,
        country_code,
        via_proxy: route.via_proxy(),
        source: source.url.to_string(),
        checked_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    })
}

async fn race_sources<Fut>(
    stagger: Duration,
    timeout: Duration,
    fetch: impl Fn(Source) -> Fut,
) -> Result<(Source, (String, Option<String>)), String>
where
    Fut: Future<Output = SourceAnswer> + Send + 'static,
{
    let mut tasks = JoinSet::new();
    let mut sources = SOURCES.iter().copied().peekable();
    let deadline = Instant::now() + timeout;
    let mut launch_at = Instant::now();
    let mut last_error = "出口 IP 接口均不可用".to_string();
    let answer = loop {
        let now = Instant::now();
        if now >= deadline {
            break Err("出口 IP 检测超时".into());
        }
        if sources.peek().is_some()
            && tasks.len() < MAX_IN_FLIGHT
            && (tasks.is_empty() || now >= launch_at)
        {
            let source = sources.next().unwrap();
            let request = fetch(source);
            tasks.spawn(async move { (source, request.await) });
            launch_at = now + stagger;
        }
        if tasks.is_empty() {
            break Err(last_error);
        }
        tokio::select! {
            _ = sleep_until(deadline) => break Err("出口 IP 检测超时".into()),
            outcome = tasks.join_next() => {
                match outcome {
                    Some(Ok((source, Ok(answer)))) => break Ok((source, answer)),
                    Some(Ok((_, Err(error)))) => last_error = error,
                    _ => last_error = "出口检测任务结束".into(),
                }
                launch_at = Instant::now();
            }
            _ = sleep_until(launch_at), if sources.peek().is_some() && tasks.len() < MAX_IN_FLIGHT => {}
        }
    };
    tasks.shutdown().await;
    answer
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
        let ident = source("https://ident.me/json");
        for invalid in [
            "not json",
            r#"{"ip":"","cc":"US"}"#,
            r#"{"ip":"10.0.0.1","cc":"US"}"#,
        ] {
            assert!(parse_source(&ident, invalid).is_none());
        }
        assert!(parse_source(
            &source("https://ipwho.is/"),
            r#"{"success":false,"ip":"8.8.8.8"}"#
        )
        .is_none());
    }

    #[test]
    fn aligned_fallbacks_parse_actual_country_fields() {
        for (url, body) in [
            (
                "https://ipinfo.io/json",
                r#"{"ip":"8.8.8.8","country":"US"}"#,
            ),
            (
                "https://get.geojs.io/v1/ip/geo.json",
                r#"{"ip":"8.8.8.8","country_code":"US"}"#,
            ),
            (
                "https://api.country.is",
                r#"{"ip":"8.8.8.8","country":"US"}"#,
            ),
            (
                "https://api.ipquery.io/?format=json",
                r#"{"ip":"8.8.8.8","location":{"country_code":"US"}}"#,
            ),
            ("https://ident.me/json", r#"{"ip":"8.8.8.8","cc":"US"}"#),
        ] {
            assert_eq!(
                parse_source(&source(url), body),
                Some(("8.8.8.8".into(), Some("US".into()))),
                "{url}"
            );
        }
    }

    #[tokio::test]
    async fn fast_primary_avoids_starting_fallback_requests() {
        let (winner, _) = race_sources(Duration::from_secs(1), Duration::from_secs(3), |source| {
            assert_eq!(source.url, "https://www.cloudflare.com/cdn-cgi/trace");
            std::future::ready(Ok(("8.8.8.8".into(), None)))
        })
        .await
        .unwrap();
        assert_eq!(winner.url, SOURCES[0].url);
    }

    #[tokio::test]
    async fn stalled_primary_allows_fallback_and_reclaims_pending_task() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        struct Cleanup(Arc<AtomicBool>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let cleaned = Arc::new(AtomicBool::new(false));
        let (winner, _) = race_sources(
            Duration::from_millis(25),
            Duration::from_secs(3),
            |source| {
                let cleanup = (source.url == SOURCES[0].url).then(|| Cleanup(cleaned.clone()));
                async move {
                    if let Some(_guard) = cleanup {
                        return std::future::pending().await;
                    }
                    assert_eq!(source.url, "https://api.ip.sb/geoip");
                    Ok(("8.8.4.4".into(), None))
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(winner.url, SOURCES[1].url);
        assert!(cleaned.load(Ordering::SeqCst));
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
