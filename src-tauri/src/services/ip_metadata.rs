//! 固定 HTTPS 数据源，查询指定出口的组织/ASN/风险；缺字段保持未知。
use super::network_probe::{build_client, ProbeRoute};
use serde::Serialize;
use serde_json::Value;
use std::future::{poll_fn, Future};
use std::net::IpAddr;
use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IpMetadata {
    pub ip: String,
    pub organization: Option<String>,
    pub isp: Option<String>,
    pub asn: Option<String>,
    pub network_type: &'static str,
    pub residential_inferred: bool,
    pub risk_score: Option<u8>,
    pub quality: &'static str,
    pub is_vpn: Option<bool>,
    pub is_proxy: Option<bool>,
    pub is_tor: Option<bool>,
    pub source: String,
    pub risk_source: Option<String>,
    pub ip_quality: &'static str,
    pub is_abuser: Option<bool>,
    pub checked_at: i64,
}

pub fn validate_ip(raw: &str) -> Result<IpAddr, String> {
    let ip: IpAddr = raw.parse().map_err(|_| "出口 IP 格式无效".to_string())?;
    let public = match ip {
        IpAddr::V4(v) => {
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_multicast()
                || v.is_broadcast()
                || v.is_documentation()
                || v.octets()[0] == 0
                || v.octets()[0] >= 240
                || (v.octets()[0] == 100 && (64..128).contains(&v.octets()[1]))
                || (v.octets()[0] == 198 && (18..20).contains(&v.octets()[1])))
        }
        IpAddr::V6(v) => {
            let segments = v.segments();
            !(v.is_loopback()
                || v.is_unspecified()
                || v.is_multicast()
                || segments[0] & 0xfe00 == 0xfc00
                || segments[0] & 0xffc0 == 0xfe80
                || (segments[0] == 0x2001 && segments[1] == 0xdb8)
                || v.to_ipv4_mapped().is_some())
        }
    };
    if !public {
        return Err("只查询公网出口 IP".into());
    }
    Ok(ip)
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(512).collect())
}

fn normalize_asn(value: &Value) -> Option<String> {
    let raw = value
        .as_u64()
        .map(|v| v.to_string())
        .or_else(|| text(value))?;
    let digits = raw.trim().trim_start_matches("AS").trim_start_matches("as");
    digits
        .parse::<u32>()
        .ok()
        .filter(|n| *n > 0)
        .map(|n| format!("AS{n}"))
}

fn compute_ip_quality(kind: &str, tor: Option<bool>, abuse: Option<bool>) -> &'static str {
    if tor == Some(true) || abuse == Some(true) {
        return "risky";
    }
    match kind {
        "residential" | "mobile" | "business" => "good",
        "datacenter" => "normal",
        _ => "unknown",
    }
}

fn declared_type(value: &Value) -> &'static str {
    match value
        .as_str()
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("residential") => "residential",
        Some("hosting" | "datacenter") => "datacenter",
        Some("wireless" | "mobile") => "mobile",
        Some("business" | "commercial" | "education" | "government" | "banking") => "business",
        Some("isp") => "isp",
        _ => "unknown",
    }
}

fn refresh_quality(data: &mut IpMetadata) {
    data.ip_quality = compute_ip_quality(data.network_type, data.is_tor, data.is_abuser);
    data.quality = if data.is_tor == Some(true) {
        "high"
    } else {
        match data.risk_score {
            Some(0..=20) => "low",
            Some(21..=50) => "moderate",
            Some(_) => "high",
            None => "unknown",
        }
    };
}

pub fn parse_metadata(ip: IpAddr, value: &Value) -> Result<IpMetadata, String> {
    let answered: IpAddr = value["ip"]
        .as_str()
        .ok_or("IP 信息缺少地址")?
        .parse()
        .map_err(|_| "IP 信息返回地址无效")?;
    if answered != ip {
        return Err("IP 信息与查询地址不一致".into());
    }
    let hosting = value["risk"]["is_datacenter"].as_bool();
    let mobile = value["risk"]["is_mobile"].as_bool();
    let network_type = match (hosting, mobile) {
        (Some(true), _) => "datacenter",
        (_, Some(true)) => "mobile",
        _ => "unknown",
    };
    let score = value["risk"]["risk_score"]
        .as_u64()
        .filter(|s| *s <= 100)
        .map(|s| s as u8);
    let tor = value["risk"]["is_tor"].as_bool();
    let quality = if tor == Some(true) {
        "high"
    } else {
        match score {
            Some(0..=20) => "low",
            Some(21..=50) => "moderate",
            Some(_) => "high",
            None => "unknown",
        }
    };
    Ok(IpMetadata {
        ip: answered.to_string(),
        organization: text(&value["isp"]["org"]).or_else(|| text(&value["isp"]["isp"])),
        isp: text(&value["isp"]["isp"]),
        asn: normalize_asn(&value["isp"]["asn"]),
        network_type,
        residential_inferred: false,
        risk_score: score,
        quality,
        is_vpn: value["risk"]["is_vpn"].as_bool(),
        is_proxy: value["risk"]["is_proxy"].as_bool(),
        is_tor: tor,
        source: "IPQuery".into(),
        risk_source: (score.is_some() || tor == Some(true)).then(|| "IPQuery".into()),
        ip_quality: compute_ip_quality(network_type, tor, value["is_abuser"].as_bool()),
        is_abuser: value["is_abuser"].as_bool(),
        checked_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64,
    })
}

async fn get_json(client: &reqwest::Client, url: String) -> Result<Value, String> {
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "IP 信息查询超时"
            } else {
                "IP 信息查询连接失败"
            }
            .to_string()
        })?;
    if !response.status().is_success() {
        return Err(format!(
            "IP 信息服务返回 HTTP {}",
            response.status().as_u16()
        ));
    }
    let bytes =
        super::http_body::read_limited(response, 64 * 1024, "IP 信息响应过大".into()).await?;
    serde_json::from_slice(&bytes).map_err(|_| "IP 信息格式无效".to_string())
}

fn apply_declared_type(data: &mut IpMetadata, value: &Value) -> bool {
    if !matches!(value["status"].as_str(), Some("ok" | "warning")) {
        return false;
    }
    let Some(entry) = value.as_object().and_then(|entries| {
        entries.iter().find_map(|(key, value)| {
            (key.parse::<IpAddr>()
                .ok()
                .map(|ip| ip.to_string())
                .as_deref()
                == Some(data.ip.as_str()))
            .then_some(value)
        })
    }) else {
        return false;
    };
    let kind = match entry["network"]["type"]
        .as_str()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("residential") => "residential",
        Some("hosting" | "datacenter") => "datacenter",
        Some("wireless" | "mobile") => "mobile",
        Some("business" | "commercial" | "education" | "government" | "banking") => "business",
        Some("isp") => "isp",
        _ => "unknown",
    };
    // A declared type for the exact queried IP is the only source of a
    // residential/commercial verdict; absence of hosting is not evidence.
    if kind != "unknown" {
        data.network_type = kind;
    }
    if data.organization.is_none() {
        data.organization = text(&entry["network"]["organisation"]);
    }
    data.residential_inferred = false;
    data.source = if data.source == "IPQuery" {
        "IPQuery / proxycheck.io"
    } else {
        "proxycheck.io"
    }
    .into();
    data.asn = data
        .asn
        .take()
        .or_else(|| normalize_asn(&entry["network"]["asn"]));
    data.is_tor = merge_flag(data.is_tor, entry["detections"]["tor"].as_bool());
    data.is_proxy = merge_flag(data.is_proxy, entry["detections"]["proxy"].as_bool());
    data.is_vpn = merge_flag(data.is_vpn, entry["detections"]["vpn"].as_bool());
    data.is_abuser = merge_flag(data.is_abuser, entry["detections"]["abuser"].as_bool());
    if entry["detections"]["hosting"].as_bool() == Some(true) {
        data.network_type = "datacenter";
    }
    refresh_quality(data);
    true
}

fn merge_flag(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    if a == Some(true) || b == Some(true) {
        Some(true)
    } else {
        a.or(b)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Ident,
    Query,
    Locate,
    ApiIs,
    ProxyCheck,
}

impl Source {
    fn url(self, ip: IpAddr) -> String {
        match self {
            Self::Ident => "https://ident.me/json".into(),
            Self::Query => format!("https://api.ipquery.io/{ip}?format=json"),
            Self::Locate => format!("https://iplocate.io/api/lookup/{ip}"),
            Self::ApiIs => format!("https://api.ipapi.is/?q={ip}"),
            Self::ProxyCheck => format!("https://proxycheck.io/v3/{ip}"),
        }
    }
}

fn parse_source(ip: IpAddr, source: Source, value: &Value) -> Result<IpMetadata, String> {
    if matches!(source, Source::Query) {
        return parse_metadata(ip, value);
    }
    let mut data = parse_metadata(ip, &serde_json::json!({"ip":ip.to_string()}))?;
    data.source.clear();
    if matches!(source, Source::ProxyCheck) {
        if !apply_declared_type(&mut data, value) {
            return Err("IP 信息与查询地址不一致或来源不可用".into());
        }
        return Ok(data);
    }
    let address = &value["ip"];
    if address.as_str().and_then(|v| v.parse::<IpAddr>().ok()) != Some(ip) {
        return Err("IP 信息与查询地址不一致".into());
    }
    let (company, asn, flags) = match source {
        Source::Ident => (&Value::Null, &Value::Null, &Value::Null),
        Source::Locate => (&value["company"], &value["asn"], &value["privacy"]),
        _ => (&value["company"], &value["asn"], value),
    };
    data.source = match source {
        Source::Ident => "ident.me",
        Source::Locate => "IPLocate",
        _ => "ipapi.is",
    }
    .into();
    data.network_type = if matches!(source, Source::Ident) {
        declared_type(&value["type"])
    } else {
        let company_type = declared_type(&company["type"]);
        let asn_type = declared_type(&asn["type"]);
        if flags["is_hosting"].as_bool() == Some(true)
            || flags["is_datacenter"].as_bool() == Some(true)
            || company_type == "datacenter"
            || asn_type == "datacenter"
        {
            "datacenter"
        } else if flags["is_mobile"].as_bool() == Some(true) {
            "mobile"
        } else if company_type != "unknown" && company_type != "isp" {
            company_type
        } else if asn_type != "unknown" {
            asn_type
        } else {
            company_type
        }
    };
    data.organization = text(&value["aso"])
        .or_else(|| text(&company["name"]))
        .or_else(|| text(&asn["name"]))
        .or_else(|| text(&asn["org"]));
    data.isp = text(&asn["name"]).or_else(|| text(&asn["org"]));
    data.asn = normalize_asn(&value["asn"]).or_else(|| normalize_asn(&asn["asn"]));
    data.is_tor = flags["is_tor"].as_bool();
    data.is_proxy = flags["is_proxy"].as_bool();
    data.is_vpn = flags["is_vpn"].as_bool();
    data.is_abuser = flags["is_abuser"].as_bool();
    refresh_quality(&mut data);
    Ok(data)
}

fn merge_metadata(base: &mut Option<IpMetadata>, next: IpMetadata) {
    let Some(current) = base.as_mut() else {
        *base = Some(next);
        return;
    };
    if next.network_type == "datacenter" || matches!(current.network_type, "unknown" | "isp") {
        if next.network_type != "unknown" {
            current.network_type = next.network_type;
        }
    }
    if current.organization.is_none() {
        current.organization = next.organization;
    }
    if current.isp.is_none() {
        current.isp = next.isp;
    }
    if current.asn.is_none() {
        current.asn = next.asn;
    }
    if next.risk_score.is_some() {
        current.risk_score = next.risk_score;
        current.risk_source = next.risk_source;
    }
    current.is_tor = merge_flag(current.is_tor, next.is_tor);
    current.is_proxy = merge_flag(current.is_proxy, next.is_proxy);
    current.is_vpn = merge_flag(current.is_vpn, next.is_vpn);
    current.is_abuser = merge_flag(current.is_abuser, next.is_abuser);
    if !current.source.split(" / ").any(|s| s == next.source) {
        current.source.push_str(" / ");
        current.source.push_str(&next.source);
    }
    refresh_quality(current);
}

const HEDGE_INTERVAL: Duration = Duration::from_secs(1);
const QUERY_DEADLINE: Duration = Duration::from_secs(12);

// 每次轮询收集当前已完成的全部响应，再判断是否返回，保留同轮已取得的风险信号。
// 请求 Future 由本函数直接持有；提前返回或取消查询会一并丢弃剩余请求。
async fn ready_responses<F>(
    pending: &mut Vec<(Source, Pin<Box<F>>)>,
) -> Vec<(Source, Result<Value, String>)>
where
    F: Future<Output = Result<Value, String>>,
{
    poll_fn(|cx| {
        let mut ready = Vec::new();
        let mut index = 0;
        while index < pending.len() {
            match pending[index].1.as_mut().poll(cx) {
                Poll::Ready(result) => {
                    let (source, _) = pending.remove(index);
                    ready.push((source, result));
                }
                Poll::Pending => index += 1,
            }
        }
        if ready.is_empty() {
            Poll::Pending
        } else {
            Poll::Ready(ready)
        }
    })
    .await
}

// wait_until 接收相对本次查询起点的绝对期限，响应到达不会重置追加来源的时间。
// 请求与时钟均可注入，测试无需真实网络、计时器或内核。
async fn query_with<F, Request, C, Wait>(
    ip: IpAddr,
    mut request: F,
    mut wait_until: C,
) -> Result<IpMetadata, String>
where
    F: FnMut(Source) -> Request,
    Request: Future<Output = Result<Value, String>>,
    C: FnMut(Duration) -> Wait,
    Wait: Future<Output = ()>,
{
    let mut data = None;
    let mut pending = vec![
        (Source::Ident, Box::pin(request(Source::Ident))),
        (Source::Query, Box::pin(request(Source::Query))),
    ];
    let fallback = [Source::Locate, Source::ApiIs, Source::ProxyCheck];
    let mut next_source = 0;
    let mut deadline = Box::pin(wait_until(QUERY_DEADLINE));
    let mut hedge = Box::pin(wait_until(HEDGE_INTERVAL));

    loop {
        if pending.is_empty() && next_source == fallback.len() {
            break;
        }
        let has_pending = !pending.is_empty();
        tokio::select! {
            biased;
            responses = ready_responses(&mut pending), if has_pending => {
                for (source, response) in responses {
                    if let Ok(value) = response.and_then(|v| parse_source(ip, source, &v)) {
                        merge_metadata(&mut data, value);
                    }
                }
                if data.as_ref().is_some_and(|value| matches!(
                    value.network_type, "residential" | "mobile" | "business" | "datacenter"
                )) {
                    break;
                }
            }
            _ = &mut deadline => break,
            _ = &mut hedge, if next_source < fallback.len() => {
                let source = fallback[next_source];
                pending.push((source, Box::pin(request(source))));
                next_source += 1;
                if next_source < fallback.len() {
                    hedge = Box::pin(wait_until(HEDGE_INTERVAL * (next_source as u32 + 1)));
                }
            }
        }
    }
    data.ok_or_else(|| "IP 信息服务暂不可用".into())
}

pub async fn query(ip: IpAddr, route: ProbeRoute) -> Result<IpMetadata, String> {
    let client = build_client(route, Duration::from_secs(3))?;
    let started = tokio::time::Instant::now();
    query_with(
        ip,
        |source| get_json(&client, source.url(ip)),
        |at| tokio::time::sleep_until(started + at),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::task::{Context, Wake, Waker};
    use tokio::sync::oneshot;

    type ResponseSender = oneshot::Sender<Result<Value, String>>;

    #[derive(Default)]
    struct ScriptState {
        now: Duration,
        timers: Vec<(Duration, oneshot::Sender<()>)>,
        requests: Vec<(Source, Option<ResponseSender>)>,
        cancelled: Vec<Source>,
    }

    #[derive(Clone, Default)]
    struct Script(Rc<RefCell<ScriptState>>);

    struct RequestGuard {
        script: Script,
        source: Source,
        completed: bool,
    }

    impl Drop for RequestGuard {
        fn drop(&mut self) {
            if !self.completed {
                self.script.0.borrow_mut().cancelled.push(self.source);
            }
        }
    }

    impl Script {
        fn request(&self, source: Source) -> impl Future<Output = Result<Value, String>> {
            let (tx, rx) = oneshot::channel();
            self.0.borrow_mut().requests.push((source, Some(tx)));
            let mut guard = RequestGuard {
                script: self.clone(),
                source,
                completed: false,
            };
            async move {
                let result = rx.await.expect("script response");
                guard.completed = true;
                result
            }
        }

        fn wait_until(&self, at: Duration) -> impl Future<Output = ()> {
            let (tx, rx) = oneshot::channel();
            let mut state = self.0.borrow_mut();
            if at <= state.now {
                tx.send(()).expect("new timer receiver");
            } else {
                state.timers.push((at, tx));
            }
            async move { rx.await.expect("script clock") }
        }

        fn advance_to(&self, millis: u64) {
            let mut state = self.0.borrow_mut();
            let now = Duration::from_millis(millis);
            assert!(now >= state.now);
            state.now = now;
            let mut index = 0;
            while index < state.timers.len() {
                if state.timers[index].0 <= now {
                    let (_, tx) = state.timers.remove(index);
                    let _ = tx.send(());
                } else {
                    index += 1;
                }
            }
        }

        fn respond(&self, source: Source, value: Value) {
            self.complete(source, Ok(value));
        }

        fn complete(&self, source: Source, result: Result<Value, String>) {
            let tx = self
                .0
                .borrow_mut()
                .requests
                .iter_mut()
                .find(|(s, _)| *s == source)
                .expect("source launched")
                .1
                .take()
                .expect("single response");
            assert!(tx.send(result).is_ok());
        }

        fn launched(&self) -> Vec<Source> {
            self.0.borrow().requests.iter().map(|(s, _)| *s).collect()
        }
    }

    struct NoopWake;
    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    fn step<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
        let waker = Waker::from(Arc::new(NoopWake));
        future.poll(&mut Context::from_waker(&waker))
    }

    fn success(result: Poll<Result<IpMetadata, String>>) -> IpMetadata {
        match result {
            Poll::Ready(Ok(value)) => value,
            other => panic!("expected metadata, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fast_primary_returns_without_waiting_and_cancels_sibling() {
        let script = Script::default();
        let mut query = Box::pin(query_with(
            validate_ip("8.8.8.8").unwrap(),
            |source| script.request(source),
            |at| script.wait_until(at),
        ));
        assert!(step(query.as_mut()).is_pending());
        assert_eq!(script.launched(), [Source::Ident, Source::Query]);
        script.respond(Source::Ident, json!({"ip":"8.8.8.8","type":"business"}));
        assert_eq!(success(step(query.as_mut())).network_type, "business");
        assert_eq!(script.0.borrow().cancelled, [Source::Query]);
    }

    #[tokio::test]
    async fn hedges_start_each_second_without_waiting_for_previous_requests() {
        let script = Script::default();
        let mut query = Box::pin(query_with(
            validate_ip("8.8.8.8").unwrap(),
            |source| script.request(source),
            |at| script.wait_until(at),
        ));
        assert!(step(query.as_mut()).is_pending());
        script.advance_to(999);
        assert!(step(query.as_mut()).is_pending());
        assert_eq!(script.launched().len(), 2);
        for (millis, source) in [
            (1000, Source::Locate),
            (2000, Source::ApiIs),
            (3000, Source::ProxyCheck),
        ] {
            script.advance_to(millis);
            assert!(step(query.as_mut()).is_pending());
            assert_eq!(script.launched().last(), Some(&source));
        }
        script.respond(
            Source::ProxyCheck,
            json!({"status":"ok","8.8.8.8":{"network":{"type":"Residential"}}}),
        );
        assert_eq!(success(step(query.as_mut())).network_type, "residential");
        assert_eq!(script.0.borrow().cancelled.len(), 4);
    }

    #[tokio::test]
    async fn pending_primary_can_win_after_hedge_and_preserves_previous_risk() {
        let script = Script::default();
        let mut query = Box::pin(query_with(
            validate_ip("8.8.8.8").unwrap(),
            |source| script.request(source),
            |at| script.wait_until(at),
        ));
        assert!(step(query.as_mut()).is_pending());
        script.respond(
            Source::Query,
            json!({"ip":"8.8.8.8","risk":{"is_tor":true,"risk_score":90}}),
        );
        assert!(step(query.as_mut()).is_pending());
        script.advance_to(1000);
        assert!(step(query.as_mut()).is_pending());
        script.respond(Source::Ident, json!({"ip":"8.8.8.8","type":"business"}));
        let value = success(step(query.as_mut()));
        assert_eq!(value.network_type, "business");
        assert_eq!(value.ip_quality, "risky");
        assert_eq!(value.risk_score, Some(90));
        assert_eq!(script.0.borrow().cancelled, [Source::Locate]);
    }

    #[tokio::test]
    async fn same_poll_ready_risk_and_hosting_override_optimistic_primary() {
        let script = Script::default();
        let mut query = Box::pin(query_with(
            validate_ip("8.8.8.8").unwrap(),
            |source| script.request(source),
            |at| script.wait_until(at),
        ));
        assert!(step(query.as_mut()).is_pending());
        script.respond(Source::Ident, json!({"ip":"8.8.8.8","type":"business"}));
        script.respond(
            Source::Query,
            json!({"ip":"8.8.8.8","risk":{"is_datacenter":true,"is_tor":true}}),
        );
        let value = success(step(query.as_mut()));
        assert_eq!(value.network_type, "datacenter");
        assert_eq!(value.is_tor, Some(true));
        assert_eq!(value.ip_quality, "risky");
    }

    #[tokio::test]
    async fn mismatched_ip_and_isp_do_not_end_query_or_postpone_hedge() {
        let script = Script::default();
        let mut query = Box::pin(query_with(
            validate_ip("8.8.8.8").unwrap(),
            |source| script.request(source),
            |at| script.wait_until(at),
        ));
        assert!(step(query.as_mut()).is_pending());
        script.advance_to(900);
        script.respond(Source::Ident, json!({"ip":"1.1.1.1","type":"residential"}));
        script.respond(
            Source::Query,
            json!({"ip":"8.8.8.8","isp":{"org":"ISP name"},"risk":{"is_datacenter":false}}),
        );
        assert!(step(query.as_mut()).is_pending());
        script.advance_to(1000);
        assert!(step(query.as_mut()).is_pending());
        script.respond(
            Source::Locate,
            json!({"ip":"8.8.8.8","company":{"type":"isp"}}),
        );
        assert!(step(query.as_mut()).is_pending());
        script.advance_to(2000);
        assert!(step(query.as_mut()).is_pending());
        script.respond(Source::ApiIs, json!({"ip":"8.8.8.8","is_mobile":true}));
        let value = success(step(query.as_mut()));
        assert_eq!(value.network_type, "mobile");
        assert!(!value.residential_inferred);
        assert!(!value.source.contains("ident.me"));
        assert_eq!(script.launched().len(), 4);
    }

    #[tokio::test]
    async fn absolute_deadline_returns_partial_or_error_and_cancels_pending() {
        for partial in [false, true] {
            let script = Script::default();
            let mut query = Box::pin(query_with(
                validate_ip("8.8.8.8").unwrap(),
                |source| script.request(source),
                |at| script.wait_until(at),
            ));
            assert!(step(query.as_mut()).is_pending());
            if partial {
                script.respond(Source::Ident, json!({"ip":"8.8.8.8","aso":"Known Org"}));
                assert!(step(query.as_mut()).is_pending());
            }
            for at in [1000, 2000, 3000, 11999] {
                script.advance_to(at);
                assert!(step(query.as_mut()).is_pending());
            }
            script.advance_to(12000);
            let result = step(query.as_mut());
            if partial {
                let value = success(result);
                assert_eq!(value.organization.as_deref(), Some("Known Org"));
                assert_eq!(value.network_type, "unknown");
                assert_eq!(value.ip_quality, "unknown");
            } else {
                assert!(matches!(result, Poll::Ready(Err(_))));
            }
            assert_eq!(
                script.0.borrow().cancelled.len(),
                if partial { 4 } else { 5 }
            );
        }
    }

    #[tokio::test]
    async fn failed_and_malformed_sources_finish_without_waiting_for_deadline() {
        let script = Script::default();
        let mut query = Box::pin(query_with(
            validate_ip("8.8.8.8").unwrap(),
            |source| script.request(source),
            |at| script.wait_until(at),
        ));
        assert!(step(query.as_mut()).is_pending());
        script.complete(Source::Ident, Err("HTTP 429".into()));
        script.respond(Source::Query, json!({"error":"unavailable"}));
        assert!(step(query.as_mut()).is_pending());
        for (at, source) in [(1000, Source::Locate), (2000, Source::ApiIs)] {
            script.advance_to(at);
            assert!(step(query.as_mut()).is_pending());
            script.complete(source, Err("timeout".into()));
            assert!(step(query.as_mut()).is_pending());
        }
        script.advance_to(3000);
        assert!(step(query.as_mut()).is_pending());
        script.respond(Source::ProxyCheck, json!({"status":"error"}));
        assert!(matches!(step(query.as_mut()), Poll::Ready(Err(_))));
        assert!(script.0.borrow().cancelled.is_empty());
    }

    #[tokio::test]
    async fn cancelling_query_drops_every_started_request() {
        let script = Script::default();
        let mut query = Box::pin(query_with(
            validate_ip("8.8.8.8").unwrap(),
            |source| script.request(source),
            |at| script.wait_until(at),
        ));
        assert!(step(query.as_mut()).is_pending());
        script.advance_to(1000);
        assert!(step(query.as_mut()).is_pending());
        drop(query);
        assert_eq!(script.0.borrow().cancelled.len(), 3);
    }

    #[test]
    fn declared_sources_require_exact_ip_and_keep_isp_distinct() {
        let ip = validate_ip("8.8.8.8").unwrap();
        for (source, value, kind) in [
            (
                Source::Ident,
                json!({"ip":"8.8.8.8","type":"business","aso":"Full Organization","asn":15169}),
                "business",
            ),
            (
                Source::Locate,
                json!({"ip":"8.8.8.8","company":{"type":"residential","name":"Home ISP"},"asn":{"asn":15169}}),
                "residential",
            ),
            (
                Source::ApiIs,
                json!({"ip":"8.8.8.8","is_mobile":true,"asn":{"type":"isp","asn":"AS15169"}}),
                "mobile",
            ),
            (
                Source::ProxyCheck,
                json!({"status":"warning","8.8.8.8":{"network":{"type":"Hosting","asn":"AS15169"}}}),
                "datacenter",
            ),
        ] {
            let data = parse_source(ip, source, &value).unwrap();
            assert_eq!(data.network_type, kind);
            assert_eq!(data.asn.as_deref(), Some("AS15169"));
            assert!(parse_source(validate_ip("1.1.1.1").unwrap(), source, &value).is_err());
        }
        let isp = parse_source(ip, Source::Ident, &json!({"ip":"8.8.8.8","type":"isp"})).unwrap();
        assert_eq!(isp.network_type, "isp");
        assert_eq!(isp.ip_quality, "unknown");
    }

    #[test]
    fn quality_matches_network_category_with_risk_precedence() {
        for (kind, quality) in [
            ("residential", "good"),
            ("mobile", "good"),
            ("business", "good"),
            ("datacenter", "normal"),
            ("isp", "unknown"),
            ("unknown", "unknown"),
        ] {
            assert_eq!(compute_ip_quality(kind, None, None), quality);
            assert_eq!(compute_ip_quality(kind, Some(true), Some(false)), "risky");
            assert_eq!(compute_ip_quality(kind, Some(false), Some(true)), "risky");
        }
    }

    #[test]
    fn merges_preserve_positive_risk_and_never_downgrade_datacenter() {
        let ip = validate_ip("8.8.8.8").unwrap();
        let hosting = parse_source(
            ip,
            Source::Query,
            &json!({"ip":"8.8.8.8","risk":{"is_datacenter":true,"is_tor":true,"risk_score":8}}),
        )
        .unwrap();
        let commercial = parse_source(ip, Source::ApiIs, &json!({"ip":"8.8.8.8","company":{"type":"business","name":"Full Org"},"is_tor":false,"is_abuser":false})).unwrap();
        let mut result = Some(hosting);
        merge_metadata(&mut result, commercial);
        let data = result.unwrap();
        assert_eq!(data.network_type, "datacenter");
        assert_eq!(data.is_tor, Some(true));
        assert_eq!(data.ip_quality, "risky");
        assert_eq!(data.quality, "high");
        assert_eq!(data.risk_score, Some(8));
        assert_eq!(data.risk_source.as_deref(), Some("IPQuery"));
        assert_eq!(data.organization.as_deref(), Some("Full Org"));
    }

    #[test]
    fn missing_flags_never_invent_residential_or_low_risk() {
        let ip = validate_ip("8.8.8.8").unwrap();
        let data = parse_source(ip, Source::ApiIs, &json!({"ip":"8.8.8.8","is_datacenter":false,"is_mobile":false,"company":{"name":"ISP Name"}})).unwrap();
        assert_eq!(data.network_type, "unknown");
        assert_eq!(data.ip_quality, "unknown");
        assert_eq!(data.quality, "unknown");
        assert_eq!(data.risk_score, None);
    }

    #[test]
    fn hosting_and_abuse_flags_override_optimistic_declared_type() {
        let ip = validate_ip("8.8.8.8").unwrap();
        let data = parse_source(ip, Source::Locate, &json!({"ip":"8.8.8.8","company":{"type":"business"},"privacy":{"is_hosting":true,"is_abuser":true}})).unwrap();
        assert_eq!(data.network_type, "datacenter");
        assert_eq!(data.ip_quality, "risky");
        assert_eq!(data.quality, "unknown");
    }

    #[test]
    fn fixed_sources_are_https_and_do_not_contain_node_configuration() {
        let ip = validate_ip("8.8.8.8").unwrap();
        for source in [
            Source::Ident,
            Source::Query,
            Source::Locate,
            Source::ApiIs,
            Source::ProxyCheck,
        ] {
            let url = reqwest::Url::parse(&source.url(ip)).unwrap();
            assert_eq!(url.scheme(), "https");
            assert!(url.username().is_empty());
            assert!(url.password().is_none());
        }
        assert_eq!(normalize_asn(&json!(15169)).as_deref(), Some("AS15169"));
        assert_eq!(normalize_asn(&json!("AS15169")).as_deref(), Some("AS15169"));
        assert_eq!(normalize_asn(&json!("ASbad")), None);
        assert_eq!(normalize_asn(&json!(0)), None);
    }
    #[test]
    fn rejects_nonpublic_invalid_and_mapped_addresses() {
        for ip in [
            "localhost",
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "198.18.0.1",
            "::1",
            "fe80::1",
            "fc00::1",
            "::ffff:8.8.8.8",
            "2001:db8::1",
        ] {
            assert!(validate_ip(ip).is_err(), "{ip}");
        }
        assert!(validate_ip("8.8.8.8").is_ok());
        assert!(validate_ip("2606:4700:4700::1111").is_ok());
    }
    #[test]
    fn missing_flags_stay_unknown_and_address_must_match() {
        let ip = validate_ip("8.8.8.8").unwrap();
        let result = parse_metadata(ip, &json!({"ip":"8.8.8.8"})).unwrap();
        assert_eq!(result.network_type, "unknown");
        assert_eq!(result.quality, "unknown");
        assert!(result.is_proxy.is_none());
        assert!(parse_metadata(ip, &json!({"ip":"1.1.1.1"})).is_err());
    }
    #[test]
    fn preserves_full_organization_without_guessing_residential() {
        let name = "China Mobile Communications Group Co., Ltd. - Complete Organization Name";
        let result = parse_metadata(
            validate_ip("8.8.8.8").unwrap(),
            &json!({
                "ip":"8.8.8.8", "isp":{"org":name,"asn":"AS15169"},
                "risk":{"is_datacenter":false,"is_mobile":false,"risk_score":10}
            }),
        )
        .unwrap();
        assert_eq!(result.organization.as_deref(), Some(name));
        assert_eq!(result.network_type, "unknown");
        assert!(!result.residential_inferred);
        assert_eq!(result.quality, "low");
    }

    #[test]
    fn declared_type_changes_with_exact_ip_and_distinguishes_commercial() {
        let mut result =
            parse_metadata(validate_ip("8.8.8.8").unwrap(), &json!({"ip":"8.8.8.8"})).unwrap();
        for (declared, expected) in [
            ("Residential", "residential"),
            ("Business", "business"),
            ("Hosting", "datacenter"),
            ("Wireless", "mobile"),
            ("ISP", "isp"),
        ] {
            assert!(apply_declared_type(
                &mut result,
                &json!({"status":"ok","8.8.8.8":{"network":{"type":declared}}})
            ));
            assert_eq!(result.network_type, expected);
            assert!(!result.residential_inferred);
        }
        assert!(!apply_declared_type(
            &mut result,
            &json!({"status":"ok","1.1.1.1":{"network":{"type":"Residential"}}})
        ));
        assert_eq!(result.network_type, "isp");
    }
    #[test]
    fn hosting_is_not_itself_high_risk_and_bad_scores_are_unknown() {
        let ip = validate_ip("8.8.8.8").unwrap();
        let result = parse_metadata(
            ip,
            &json!({"ip":"8.8.8.8", "risk":{"is_datacenter":true,"risk_score":0}}),
        )
        .unwrap();
        assert_eq!(result.network_type, "datacenter");
        assert_eq!(result.quality, "low");
        for score in [json!(-1), json!(101), json!("10"), json!(0.5)] {
            let result =
                parse_metadata(ip, &json!({"ip":"8.8.8.8","risk":{"risk_score":score}})).unwrap();
            assert!(result.risk_score.is_none());
            assert_eq!(result.quality, "unknown");
        }
    }
}
