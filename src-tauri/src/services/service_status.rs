//! 固定服务目录检测：Google/GitHub 检测连通性，其他服务解析各自的只读响应。

#[path = "service_status_rules.rs"]
mod rules;

use super::service_route::{self, RequestContext, RequestRoute, RouteContext};
use crate::services::network_probe::{build_client, ProbeRoute};
use serde::Serialize;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout, timeout_at, Instant as Deadline};

const MAX_IDS: usize = 13;
const MAX_CONCURRENCY: usize = 4;
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(4);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const RETRY_DELAY: Duration = Duration::from_millis(250);
const BATCH_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Service {
    id: &'static str,
    url: &'static str,
}

const CATALOG: [Service; MAX_IDS] = [
    Service {
        id: "google",
        url: "https://www.google.com/generate_204",
    },
    Service {
        id: "github",
        url: "https://github.com/",
    },
    Service {
        id: "youtube",
        url: "https://www.youtube.com/premium?hl=en",
    },
    Service {
        id: "chatgpt",
        url: "https://ios.chat.openai.com/",
    },
    Service {
        id: "claude",
        url: "https://claude.ai/cdn-cgi/trace",
    },
    Service {
        id: "gemini",
        url: "https://gemini.google.com/",
    },
    Service {
        id: "netflix",
        url: "https://www.netflix.com/title/81280792",
    },
    Service {
        id: "disney",
        url: "https://www.disneyplus.com/",
    },
    Service {
        id: "prime",
        url: "https://www.primevideo.com/",
    },
    Service {
        id: "spotify",
        url: "https://www.spotify.com/api/content/v1/country-selector?platform=web&format=json",
    },
    Service {
        id: "tiktok",
        url: "https://www.tiktok.com/",
    },
    Service {
        id: "discord",
        url: "https://discord.com/api/v9/auth/location-metadata",
    },
    Service {
        id: "bilibili",
        url: "https://api.bilibili.com/pgc/player/web/playurl?avid=82846771&qn=0&type=&otype=json&ep_id=307247&fourk=1&fnver=0&fnval=16&module=bangumi",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    Reachable,
    Redirect,
    Restricted,
    HttpError,
    Timeout,
    NetworkError,
    Available,
    Unavailable,
    Blocked,
    DisallowedIsp,
    UnsupportedRegion,
    OriginalsOnly,
    ComingSoon,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatusResult {
    pub id: String,
    pub status: ServiceStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    /// Unix 毫秒；表示本项完成或被整批截止时间取消的时刻。
    pub checked_at: i64,
    /// 仅表示请求经过内核 HTTP 入站，内核仍按实际规则决定出站。
    pub via_proxy: bool,
    /// 仅取自被检测服务；缺失或非法地区不输出。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    pub check_kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region_reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<RequestRoute>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatusBatch {
    pub results: Vec<ServiceStatusResult>,
    /// Unix 毫秒；表示整批完成的时刻。
    pub checked_at: i64,
    pub via_proxy: bool,
}

fn checked_at() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn select_services(ids: &[String]) -> Result<Vec<Service>, String> {
    // 先限制原始输入，重复 ID 也不能用于绕过批次大小限制。
    if ids.is_empty() || ids.len() > MAX_IDS {
        return Err("服务 ID 数量必须为 1 到 13 个".into());
    }
    let mut selected = Vec::with_capacity(ids.len());
    for id in ids {
        let service = CATALOG
            .iter()
            .find(|service| service.id == id)
            .ok_or_else(|| "包含不支持的服务 ID".to_string())?;
        if !selected.contains(service) {
            selected.push(*service);
        }
    }
    Ok(selected)
}

fn http_status(status: u16) -> ServiceStatus {
    match status {
        200..=299 => ServiceStatus::Reachable,
        300..=399 => ServiceStatus::Redirect,
        401 | 403 | 451 => ServiceStatus::Restricted,
        _ => ServiceStatus::HttpError,
    }
}

// 单次超时及两次请求、间隔共享总预算。注入 future 供 CI 做纯内存回归。
trait RetryOutcome {
    fn status(&self) -> ServiceStatus;
    fn timed_out() -> Self;
}

impl RetryOutcome for rules::Outcome {
    fn status(&self) -> ServiceStatus {
        self.status
    }

    fn timed_out() -> Self {
        Self::new(ServiceStatus::Timeout, None)
    }
}

#[cfg(test)]
impl RetryOutcome for (ServiceStatus, Option<u16>) {
    fn status(&self) -> ServiceStatus {
        self.0
    }

    fn timed_out() -> Self {
        (ServiceStatus::Timeout, None)
    }
}

async fn probe_with_retry<F, Fut, T>(
    mut attempt: F,
    attempt_timeout: Duration,
    total_timeout: Duration,
    retry_delay: Duration,
) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = T>,
    T: RetryOutcome,
{
    timeout(total_timeout, async {
        for index in 0..2 {
            let result = timeout(attempt_timeout, attempt())
                .await
                .unwrap_or_else(|_| T::timed_out());
            if index == 1
                || !matches!(
                    result.status(),
                    ServiceStatus::Timeout | ServiceStatus::NetworkError
                )
            {
                return result;
            }
            sleep(retry_delay).await;
        }
        unreachable!()
    })
    .await
    .unwrap_or_else(|_| T::timed_out())
}

async fn probe_one(
    client: reqwest::Client,
    service: Service,
    via_proxy: bool,
    context: Arc<RouteContext>,
) -> ServiceStatusResult {
    let started = Instant::now();
    let request = RequestContext::new(context, service.url);
    let outcome = service_route::scope(
        request.clone(),
        probe_with_retry(
            || {
                request.reset_attempt();
                rules::probe(&client, service)
            },
            ATTEMPT_TIMEOUT,
            REQUEST_TIMEOUT,
            RETRY_DELAY,
        ),
    )
    .await;
    let latency_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    // 出口慢或不可用只影响证据字段，保留已完成的服务判断及其耗时。
    let _ = timeout(Duration::from_secs(5), request.finish()).await;
    ServiceStatusResult {
        id: service.id.into(),
        status: outcome.status,
        latency_ms: Some(latency_ms),
        http_status: outcome.http_status,
        checked_at: checked_at(),
        via_proxy,
        region: outcome.region,
        check_kind: rules::check_kind(service.id),
        reason: outcome.reason,
        region_reason: outcome.region_reason,
        routes: request.records(),
    }
}

/// 只启动最多四个任务；整批截止时取消并回收在途任务，为未完成项返回 timeout。
/// 注入单项 future 便于用纯内存测试验证调度，不需要访问真实服务。
async fn probe_selected<F, Fut>(
    services: Vec<Service>,
    via_proxy: bool,
    deadline: Deadline,
    probe: F,
) -> Result<ServiceStatusBatch, String>
where
    F: Fn(Service) -> Fut,
    Fut: Future<Output = ServiceStatusResult> + Send + 'static,
{
    let mut tasks = JoinSet::new();
    let mut next = 0;
    let mut results: Vec<Option<ServiceStatusResult>> = vec![None; services.len()];
    loop {
        while next < services.len() && tasks.len() < MAX_CONCURRENCY && Deadline::now() < deadline {
            let index = next;
            let request = probe(services[index]);
            tasks.spawn(async move { (index, request.await) });
            next += 1;
        }
        if tasks.is_empty() {
            break;
        }
        match timeout_at(deadline, tasks.join_next()).await {
            Ok(Some(Ok((index, result)))) => results[index] = Some(result),
            Ok(Some(Err(_))) => {
                tasks.shutdown().await;
                return Err("服务状态检测任务异常结束".into());
            }
            Ok(None) => break,
            Err(_) => {
                tasks.shutdown().await;
                break;
            }
        }
    }
    let checked_at = checked_at();
    let results = services
        .iter()
        .zip(results)
        .map(|(service, result)| {
            result.unwrap_or_else(|| ServiceStatusResult {
                id: service.id.into(),
                status: ServiceStatus::Timeout,
                latency_ms: None,
                http_status: None,
                checked_at,
                via_proxy,
                region: None,
                check_kind: rules::check_kind(service.id),
                reason: None,
                region_reason: None,
                routes: Vec::new(),
            })
        })
        .collect();
    Ok(ServiceStatusBatch {
        results,
        checked_at,
        via_proxy,
    })
}

/// 地址只来自固定目录；客户端显式使用选定路线，失败后不尝试其他路线。
pub async fn probe_services(
    ids: &[String],
    route: ProbeRoute,
) -> Result<ServiceStatusBatch, String> {
    probe_services_with_context(ids, route, RouteContext::new(None, String::new())).await
}

pub async fn probe_services_with_context(
    ids: &[String],
    route: ProbeRoute,
    context: Arc<RouteContext>,
) -> Result<ServiceStatusBatch, String> {
    let services = select_services(ids)?;
    let deadline = Deadline::now() + BATCH_TIMEOUT;
    let client = build_client(route, ATTEMPT_TIMEOUT)
        .map_err(|_| "无法创建服务状态检测客户端".to_string())?;
    let via_proxy = route.via_proxy();
    probe_selected(services, via_proxy, deadline, move |service| {
        probe_one(client.clone(), service, via_proxy, context.clone())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|id| (*id).to_string()).collect()
    }

    fn synthetic_result(service: Service) -> ServiceStatusResult {
        ServiceStatusResult {
            id: service.id.into(),
            status: ServiceStatus::Reachable,
            latency_ms: Some(1),
            http_status: Some(204),
            checked_at: checked_at(),
            via_proxy: true,
            region: None,
            check_kind: rules::check_kind(service.id),
            reason: None,
            region_reason: None,
            routes: Vec::new(),
        }
    }

    #[test]
    fn maps_all_http_status_classes_without_business_claims() {
        for code in 100..=599 {
            let expected = if (200..300).contains(&code) {
                ServiceStatus::Reachable
            } else if (300..400).contains(&code) {
                ServiceStatus::Redirect
            } else if [401, 403, 451].contains(&code) {
                ServiceStatus::Restricted
            } else {
                ServiceStatus::HttpError
            };
            assert_eq!(http_status(code), expected, "HTTP {code}");
        }
    }

    #[test]
    fn accepts_only_catalog_ids_and_deduplicates_in_input_order() {
        let selected = select_services(&ids(&["github", "google", "github"])).unwrap();
        assert_eq!(selected, vec![CATALOG[1], CATALOG[0]]);
        for invalid in [
            "https://github.com/",
            "http://127.0.0.1/",
            "GOOGLE",
            " google",
            "",
            "unknown",
        ] {
            assert!(select_services(&ids(&["github", invalid])).is_err());
        }
        assert!(select_services(&[]).is_err());
        assert!(select_services(&vec!["google".into(); MAX_IDS + 1]).is_err());
        assert_eq!(
            select_services(&vec!["google".into(); MAX_IDS])
                .unwrap()
                .len(),
            1
        );
        let all = CATALOG
            .iter()
            .map(|service| service.id.into())
            .collect::<Vec<_>>();
        assert_eq!(select_services(&all).unwrap().len(), MAX_IDS);
        assert!(CATALOG
            .iter()
            .all(|service| service.url.starts_with("https://")));
        let error = select_services(&ids(&["unrecognized-sensitive-input"])).unwrap_err();
        assert!(!error.contains("unrecognized-sensitive-input"));
    }

    #[test]
    fn serializes_camel_case_fields_and_exact_status_names() {
        for (status, name) in [
            (ServiceStatus::Reachable, "reachable"),
            (ServiceStatus::Redirect, "redirect"),
            (ServiceStatus::Restricted, "restricted"),
            (ServiceStatus::HttpError, "http_error"),
            (ServiceStatus::Timeout, "timeout"),
            (ServiceStatus::NetworkError, "network_error"),
            (ServiceStatus::Available, "available"),
            (ServiceStatus::Unavailable, "unavailable"),
            (ServiceStatus::Blocked, "blocked"),
            (ServiceStatus::DisallowedIsp, "disallowed_isp"),
            (ServiceStatus::UnsupportedRegion, "unsupported_region"),
            (ServiceStatus::OriginalsOnly, "originals_only"),
            (ServiceStatus::ComingSoon, "coming_soon"),
            (ServiceStatus::Failed, "failed"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), name);
        }
        let mut result = synthetic_result(CATALOG[0]);
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(value["latencyMs"], 1);
        assert_eq!(value["httpStatus"], 204);
        assert_eq!(value["viaProxy"], true);
        assert_eq!(value["checkKind"], "reachability");
        assert!(value.get("region").is_none());
        result.region = Some("JP".into());
        result.check_kind = "availability";
        assert_eq!(serde_json::to_value(&result).unwrap()["region"], "JP");
        assert!(value["checkedAt"].is_i64());
        result.latency_ms = None;
        result.http_status = None;
        let batch = ServiceStatusBatch {
            results: vec![result],
            checked_at: 123,
            via_proxy: true,
        };
        let value = serde_json::to_value(batch).unwrap();
        assert_eq!(value["checkedAt"], 123);
        assert_eq!(value["viaProxy"], true);
        assert!(value["results"][0].get("httpStatus").is_none());
        assert!(value["results"][0].get("latencyMs").is_none());
    }

    #[test]
    fn attempts_share_eight_seconds_and_batch_remains_bounded() {
        assert_eq!(MAX_CONCURRENCY, 4);
        assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(8));
        assert_eq!(ATTEMPT_TIMEOUT, Duration::from_secs(4));
        assert_eq!(RETRY_DELAY, Duration::from_millis(250));
        assert_eq!(CATALOG[1].url, "https://github.com/");
        assert_eq!(BATCH_TIMEOUT, Duration::from_secs(25));
        let waves = MAX_IDS.div_ceil(MAX_CONCURRENCY) as u32;
        assert_eq!(waves, 4);
        // 最坏四波重试由整批截止时间取消。
        assert!(REQUEST_TIMEOUT * waves > BATCH_TIMEOUT);
    }

    #[tokio::test]
    async fn retries_only_transient_failures_once_and_preserves_final_classification() {
        for transient in [ServiceStatus::Timeout, ServiceStatus::NetworkError] {
            for final_result in [
                (ServiceStatus::Reachable, Some(200)),
                (ServiceStatus::Restricted, Some(403)),
                (ServiceStatus::Timeout, None),
                (ServiceStatus::NetworkError, None),
            ] {
                let calls = AtomicUsize::new(0);
                let started = Deadline::now();
                let result = probe_with_retry(
                    || {
                        let call = calls.fetch_add(1, Ordering::SeqCst);
                        if call == 1 {
                            assert!(started.elapsed() >= RETRY_DELAY);
                        }
                        std::future::ready(if call == 0 {
                            (transient, None)
                        } else {
                            final_result
                        })
                    },
                    ATTEMPT_TIMEOUT,
                    REQUEST_TIMEOUT,
                    RETRY_DELAY,
                )
                .await;
                assert_eq!(result, final_result);
                assert_eq!(calls.load(Ordering::SeqCst), 2);
            }
        }
        // 所有 HTTP 状态都直接保留，包括 403、重定向和 5xx。
        for code in 100..=599 {
            let calls = AtomicUsize::new(0);
            let result = probe_with_retry(
                || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready((http_status(code), Some(code)))
                },
                ATTEMPT_TIMEOUT,
                REQUEST_TIMEOUT,
                RETRY_DELAY,
            )
            .await;
            assert_eq!(result, (http_status(code), Some(code)));
            assert_eq!(calls.load(Ordering::SeqCst), 1, "HTTP {code}");
        }
    }

    #[tokio::test]
    async fn retry_outcome_keeps_region_without_retrying_http_denials() {
        for status in [
            ServiceStatus::Restricted,
            ServiceStatus::HttpError,
            ServiceStatus::UnsupportedRegion,
            ServiceStatus::Blocked,
            ServiceStatus::Failed,
            ServiceStatus::Available,
        ] {
            let calls = AtomicUsize::new(0);
            let result = probe_with_retry(
                || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(rules::Outcome {
                        status,
                        http_status: Some(403),
                        region: Some("US".into()),
                        reason: None,
                        region_reason: None,
                    })
                },
                ATTEMPT_TIMEOUT,
                REQUEST_TIMEOUT,
                RETRY_DELAY,
            )
            .await;
            assert_eq!(result.status, status);
            assert_eq!(result.http_status, Some(403));
            assert_eq!(result.region.as_deref(), Some("US"));
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn total_budget_cancels_second_attempt_and_includes_retry_delay() {
        let calls = AtomicUsize::new(0);
        let active = Arc::new(AtomicUsize::new(0));
        let started = Deadline::now();
        let result = probe_with_retry(
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                let active = active.clone();
                async move {
                    active.fetch_add(1, Ordering::SeqCst);
                    let _guard = Active(active);
                    std::future::pending::<(ServiceStatus, Option<u16>)>().await
                }
            },
            Duration::from_millis(100),
            Duration::from_millis(180),
            Duration::from_millis(20),
        )
        .await;
        assert_eq!(result, (ServiceStatus::Timeout, None));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert!(started.elapsed() >= Duration::from_millis(180));
        assert!(started.elapsed() < Duration::from_secs(1));

        let calls = AtomicUsize::new(0);
        let result = probe_with_retry(
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready((ServiceStatus::NetworkError, None))
            },
            Duration::from_millis(100),
            Duration::from_millis(20),
            RETRY_DELAY,
        )
        .await;
        assert_eq!(result, (ServiceStatus::Timeout, None));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    struct Active(Arc<AtomicUsize>);

    impl Drop for Active {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn limits_actual_inflight_work_to_four_and_preserves_order() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let first_wave = Arc::new(tokio::sync::Barrier::new(MAX_CONCURRENCY));
        let batch = probe_selected(CATALOG.to_vec(), true, Deadline::now() + BATCH_TIMEOUT, {
            let active = active.clone();
            let peak = peak.clone();
            move |service| {
                let active = active.clone();
                let peak = peak.clone();
                let first_wave = first_wave.clone();
                async move {
                    let _guard = Active(active.clone());
                    let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(count, Ordering::SeqCst);
                    if CATALOG[..MAX_CONCURRENCY].contains(&service) {
                        first_wave.wait().await;
                    }
                    tokio::task::yield_now().await;
                    synthetic_result(service)
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(peak.load(Ordering::SeqCst), MAX_CONCURRENCY);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(
            batch
                .results
                .iter()
                .map(|result| result.id.as_str())
                .collect::<Vec<_>>(),
            CATALOG.iter().map(|service| service.id).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn batch_deadline_cancels_inflight_work_and_marks_pending_items_timeout() {
        let active = Arc::new(AtomicUsize::new(0));
        let batch = probe_selected(
            CATALOG.to_vec(),
            false,
            Deadline::now() + Duration::from_millis(20),
            {
                let active = active.clone();
                move |_service| {
                    let active = active.clone();
                    async move {
                        active.fetch_add(1, Ordering::SeqCst);
                        let _guard = Active(active);
                        std::future::pending::<ServiceStatusResult>().await
                    }
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(batch.results.len(), MAX_IDS);
        assert!(batch
            .results
            .iter()
            .all(|result| result.status == ServiceStatus::Timeout
                && result.http_status.is_none()
                && result.latency_ms.is_none()
                && !result.via_proxy));
    }
}
