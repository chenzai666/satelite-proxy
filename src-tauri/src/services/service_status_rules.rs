//! 根据公开协议字段独立实现规则；仅参考 FlClash 的检测目标与状态含义。
//! 所有请求均为固定 HTTPS GET，不携带账号凭据，不创建设备或交换访客令牌。

use super::{http_status, Service, ServiceStatus};
use regex::Regex;
use serde_json::Value;
use std::time::Duration;

#[path = "service_video_rules.rs"]
mod video;

const SMALL_LIMIT: usize = 64 * 1024;
const HTML_LIMIT: usize = 512 * 1024;
const LARGE_LIMIT: usize = 8 * 1024 * 1024;
const CHATGPT_TRACE: &str = "https://chat.openai.com/cdn-cgi/trace";
const CHATGPT_COMPLIANCE: &str = "https://api.openai.com/compliance/cookie_requirements";
const NETFLIX_SECOND: &str = "https://www.netflix.com/title/70143836";
const NETFLIX_REGION: &str = "https://www.netflix.com/title/80018499";
const BILIBILI_SECOND: &str = "https://api.bilibili.com/pgc/player/web/playurl?avid=18281381&cid=29892777&qn=0&type=&otype=json&ep_id=183799&fourk=1&fnver=0&fnval=16&module=bangumi";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Outcome {
    pub status: ServiceStatus,
    pub http_status: Option<u16>,
    pub region: Option<String>,
    pub reason: Option<&'static str>,
    pub region_reason: Option<&'static str>,
}

impl Outcome {
    pub(super) fn new(status: ServiceStatus, http_status: Option<u16>) -> Self {
        Self {
            status,
            http_status,
            region: None,
            reason: None,
            region_reason: None,
        }
    }
}

pub(super) fn check_kind(id: &str) -> &'static str {
    if matches!(id, "google" | "github") {
        "reachability"
    } else {
        "availability"
    }
}

fn body_limit(id: &str) -> usize {
    match id {
        "google" | "github" => 0,
        "youtube" | "gemini" | "prime" | "tiktok" | "netflix" | "disney" => LARGE_LIMIT,
        _ => SMALL_LIMIT,
    }
}

/// 不保留响应头全集、Cookie、token 或原始错误文本。
#[derive(Default)]
struct Reply {
    code: u16,
    body: String,
    location: Option<String>,
    challenge: bool,
}

/// Content-Length 提前拒绝，分块正文也必须逐块计数。越限不解析截断的正文。
struct Body {
    bytes: Vec<u8>,
    limit: usize,
}

impl Body {
    fn new(limit: usize, declared: Option<u64>) -> Option<Self> {
        if declared.is_some_and(|size| size > limit as u64) {
            return None;
        }
        Some(Self {
            bytes: Vec::new(),
            limit,
        })
    }

    fn push(&mut self, chunk: &[u8]) -> bool {
        if chunk.len() > self.limit.saturating_sub(self.bytes.len()) {
            return false;
        }
        self.bytes.extend_from_slice(chunk);
        true
    }
}

fn network_error(error: &reqwest::Error, code: Option<u16>) -> Outcome {
    Outcome::new(
        if error.is_timeout() {
            ServiceStatus::Timeout
        } else {
            ServiceStatus::NetworkError
        },
        code,
    )
}

async fn get(client: &reqwest::Client, url: &str, limit: usize) -> Result<Reply, Outcome> {
    // 客户端由 network_probe::build_client 提供 no_proxy、固定内核入口和禁止跳转。
    let mut response = client
        .get(url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.8")
        .send()
        .await
        .map_err(|error| network_error(&error, None))?;
    crate::services::service_route::observe(&response, url).await;
    let code = response.status().as_u16();
    let mut reply = Reply {
        code,
        location: response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.len() <= 2048)
            .map(str::to_owned),
        challenge: response
            .headers()
            .get("cf-mitigated")
            .is_some_and(|value| value == "challenge"),
        ..Reply::default()
    };
    if limit == 0 {
        return Ok(reply);
    }
    let failed = |reason| {
        let mut outcome = Outcome::new(ServiceStatus::Failed, Some(code));
        outcome.reason = Some(reason);
        outcome
    };
    if response
        .headers()
        .get(reqwest::header::CONTENT_ENCODING)
        .is_some_and(|encoding| encoding != "identity")
    {
        return Err(failed("unsupported_encoding"));
    }
    let mut body =
        Body::new(limit, response.content_length()).ok_or_else(|| failed("response_too_large"))?;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| network_error(&error, Some(code)))?
    {
        if !body.push(&chunk) {
            return Err(failed("response_too_large"));
        }
    }
    reply.body = String::from_utf8(body.bytes).map_err(|_| failed("invalid_response"))?;
    if matches!(url, "https://claude.ai/cdn-cgi/trace" | CHATGPT_TRACE)
        && trace_region(&reply).is_some()
    {
        if let Some(ip) = reply.body.lines().find_map(|line| line.strip_prefix("ip=")) {
            crate::services::service_route::record_service_ip(url, ip.trim());
        }
    }
    Ok(reply)
}

/// ISO 3166-1 两字母白名单；不将空值、机房区域 EU-TTP2 或任意三字母值输出。
fn region(raw: &str) -> Option<String> {
    const ISO: &str = "AD AE AF AG AI AL AM AO AQ AR AS AT AU AW AX AZ BA BB BD BE BF BG BH BI BJ BL BM BN BO BQ BR BS BT BV BW BY BZ CA CC CD CF CG CH CI CK CL CM CN CO CR CU CV CW CX CY CZ DE DJ DK DM DO DZ EC EE EG EH ER ES ET FI FJ FK FM FO FR GA GB GD GE GF GG GH GI GL GM GN GP GQ GR GS GT GU GW GY HK HM HN HR HT HU ID IE IL IM IN IO IQ IR IS IT JE JM JO JP KE KG KH KI KM KN KP KR KW KY KZ LA LB LC LI LK LR LS LT LU LV LY MA MC MD ME MF MG MH MK ML MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ NA NC NE NF NG NI NL NO NP NR NU NZ OM PA PE PF PG PH PK PL PM PN PR PS PT PW PY QA RE RO RS RU RW SA SB SC SD SE SG SH SI SJ SK SL SM SN SO SR SS ST SV SX SY SZ TC TD TF TG TH TJ TK TL TM TN TO TR TT TV TW TZ UA UG UM US UY UZ VA VC VE VG VI VN VU WF WS YE YT ZA ZM ZW";
    let code = raw.trim().to_ascii_uppercase();
    if code.len() == 2
        && code.bytes().all(|c| c.is_ascii_uppercase())
        && ISO.split_ascii_whitespace().any(|known| known == code)
    {
        Some(code)
    } else {
        None
    }
}

fn capture(body: &str, pattern: &str) -> Option<String> {
    Regex::new(pattern)
        .ok()?
        .captures(body)?
        .get(1)
        .map(|value| value.as_str().to_owned())
}

fn field_region(body: &str, name: &str) -> Option<String> {
    capture(
        body,
        &format!(r#""{}"\s*:\s*"([^"]*)""#, regex::escape(name)),
    )
    .and_then(|raw| region(&raw))
}

fn bool_field(body: &str, name: &str) -> Option<bool> {
    match capture(
        body,
        &format!(r#""{}"\s*:\s*(true|false)\b"#, regex::escape(name)),
    )?
    .as_str()
    {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn trace_region(reply: &Reply) -> Option<String> {
    if reply.code != 200 || challenged(reply) {
        return None;
    }
    reply
        .body
        .lines()
        .find_map(|line| line.strip_prefix("loc=").and_then(region))
}

fn challenged(reply: &Reply) -> bool {
    let body = reply.body.to_ascii_lowercase();
    reply.challenge
        || [
            "<title>just a moment",
            "/cdn-cgi/challenge-platform/",
            "cf-chl-",
            "verify you are human",
        ]
        .iter()
        .any(|marker| body.contains(marker))
}

fn fallback(code: u16) -> ServiceStatus {
    match code {
        200..=299 => ServiceStatus::Failed,
        300..=399 => ServiceStatus::Redirect,
        401 | 429 => ServiceStatus::Restricted,
        // 普通 HTTP 拒绝与 WAF 挑战、服务明确封禁分别显示；保留原始 HTTP 详情。
        403 | 451 => ServiceStatus::HttpError,
        _ => ServiceStatus::Unavailable,
    }
}

fn claude_trace(reply: &Reply) -> Outcome {
    let mut result = Outcome::new(fallback(reply.code), Some(reply.code));
    if reply.code == 403 {
        result.status = if challenged(reply) {
            ServiceStatus::Blocked
        } else {
            ServiceStatus::Restricted
        };
        return result;
    }
    if reply.code != 200 {
        return result;
    }
    // 此 loc 来自 Claude 自身的固定 trace 端点，采用 FlClash 的地区限制语义。
    result.region = trace_region(reply);
    result.status = match result.region.as_deref() {
        Some("AF" | "BY" | "CN" | "CU" | "HK" | "IR" | "KP" | "MO" | "RU" | "SY") => {
            ServiceStatus::UnsupportedRegion
        }
        Some(_) => ServiceStatus::Available,
        None => ServiceStatus::Failed,
    };
    result
}

fn judge(id: &str, reply: &Reply) -> Outcome {
    if check_kind(id) == "reachability" {
        return Outcome::new(http_status(reply.code), Some(reply.code));
    }
    if id == "claude" {
        return claude_trace(reply);
    }
    let mut result = Outcome::new(fallback(reply.code), Some(reply.code));
    if challenged(reply) {
        result.status = ServiceStatus::Restricted;
        result.reason = Some("challenge");
        return result;
    }
    // 网关错误、登录重定向等不能被正文片段升级为可用。
    if !(200..300).contains(&reply.code) && !matches!(reply.code, 403 | 451) {
        return result;
    }
    let lower = reply.body.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| lower.contains(needle));
    if has(&["sorry, you have been blocked"]) {
        result.status = ServiceStatus::Blocked;
        return result;
    }
    let success = (200..300).contains(&reply.code);
    match id {
        "chatgpt" => {
            result.status = if has(&["you may be connected to a disallowed isp"]) {
                ServiceStatus::DisallowedIsp
            } else if has(&[
                "unsupported_country",
                "unsupported country",
                "not available in your country",
            ]) {
                ServiceStatus::UnsupportedRegion
            } else if has(&["request is not allowed. please try again later."])
                && matches!(reply.code, 200 | 403)
            {
                // 此固定 iOS 入口的预期拒绝表明已进入服务逻辑，而非任意 403。
                result.reason = Some("expected_denial");
                ServiceStatus::Available
            } else {
                fallback(reply.code)
            };
        }
        "youtube" => {
            result.region = ["GL", "countryCode", "country_code"]
                .iter()
                .find_map(|field| field_region(&reply.body, field))
                .or_else(|| {
                    capture(
                        &reply.body,
                        r#"(?i)id=["\x27]country-code["\x27][^>]*>\s*([a-z]{2})\s*<"#,
                    )
                    .and_then(|raw| region(&raw))
                });
            result.status = if has(&[
                "premium is not available in your country",
                "premium is not available in your region",
            ]) {
                ServiceStatus::UnsupportedRegion
            } else if success
                && has(&["youtube premium"])
                && has(&["ad-free", "\"browseid\":\"spunlimited\""])
            {
                ServiceStatus::Available
            } else {
                fallback(reply.code)
            };
        }
        "gemini" => {
            let raw = capture(&reply.body, r#",2,1,200,"([A-Z]{3})""#);
            // 私有页面地区标记只在服务响应包含精确结构时采用。未知代码保守失败。
            let known = raw.as_deref().and_then(gemini_region);
            result.region = known.and_then(region);
            if success && result.region.is_some() {
                result.status = if matches!(
                    raw.as_deref(),
                    Some("CHN" | "RUS" | "BLR" | "CUB" | "IRN" | "PRK" | "SYR" | "HKG" | "MAC")
                ) {
                    ServiceStatus::UnsupportedRegion
                } else {
                    ServiceStatus::Available
                };
            }
            if has(&[
                "gemini isn't currently supported in your country",
                "gemini is not available in your country",
            ]) {
                result.status = ServiceStatus::UnsupportedRegion;
            }
        }
        "netflix" => {
            result.region = field_region(&reply.body, "countryCode");
            result.status = if has(&["unblocker or proxy", "using a vpn", "using an unblocker"]) {
                ServiceStatus::Blocked
            } else if has(&[
                "netflix is not available in your country",
                "not available in your region",
            ]) {
                ServiceStatus::UnsupportedRegion
            } else if success
                && (has(&[
                    "\"@type\":\"movie\"",
                    "\"@type\": \"movie\"",
                    "\"@type\":\"tvseries\"",
                    "\"@type\": \"tvseries\"",
                ]) && has(&["netflix"]))
            {
                ServiceStatus::Available
            } else {
                fallback(reply.code)
            };
        }
        "disney" => {
            result.region = field_region(&reply.body, "countryCode");
            result.status = if has(&[
                "disney+ is not available in your region",
                "disney+ is not available in your location",
                "forbidden-location",
            ]) {
                ServiceStatus::UnsupportedRegion
            } else if has(&["disney+ is coming soon", "disney+ coming soon"]) {
                ServiceStatus::ComingSoon
            } else {
                // 只读首页不能重现设备注册/令牌交换/刷新 mutation，营销页不判解锁。
                fallback(reply.code)
            };
        }
        "prime" => {
            result.region = field_region(&reply.body, "currentTerritory");
            result.status = match bool_field(&reply.body, "isServiceRestricted") {
                Some(true) => ServiceStatus::UnsupportedRegion,
                Some(false) if success && result.region.is_some() => ServiceStatus::Available,
                // 服务自己的当前地区为有效证据；不能仅因出现 restricted 字段就判受限。
                None if success && result.region.is_some() => ServiceStatus::Available,
                _ => fallback(reply.code),
            };
        }
        "spotify" => {
            if has(&["not available in your country"]) {
                result.status = ServiceStatus::UnsupportedRegion;
            } else if success {
                if let Ok(value) = serde_json::from_str::<Value>(&reply.body) {
                    result.region = value
                        .get("countryCode")
                        .and_then(Value::as_str)
                        .and_then(region);
                    if result.region.is_some() {
                        result.status = ServiceStatus::Available;
                    }
                }
            }
        }
        "tiktok" => {
            // 仅访客上下文，忽略 serving region: EU-TTP2 等集群字段。
            result.region = capture(
                &reply.body,
                r#""webapp\.app-context"\s*:\s*\{[^{}]*"region"\s*:\s*"([^"]*)""#,
            )
            .and_then(|raw| region(&raw));
            result.status = if has(&["not available in your region", "tiktok is not available"]) {
                ServiceStatus::UnsupportedRegion
            } else if success && result.region.is_some() {
                ServiceStatus::Available
            } else {
                fallback(reply.code)
            };
        }
        "discord" => {
            if success {
                if let Ok(value) = serde_json::from_str::<Value>(&reply.body) {
                    result.region = value
                        .get("country_code")
                        .and_then(Value::as_str)
                        .and_then(region);
                    if result.region.is_some() {
                        result.status = ServiceStatus::Available;
                    }
                }
            }
        }
        "bilibili" => {
            if success {
                if let Ok(value) = serde_json::from_str::<Value>(&reply.body) {
                    result.status = match value.get("code").and_then(Value::as_i64) {
                        Some(-10403) => ServiceStatus::UnsupportedRegion,
                        // 成功 code 还需有播放字段，登录/空数据不冒充可播放。
                        Some(0) if playable(value.get("result").or_else(|| value.get("data"))) => {
                            ServiceStatus::Available
                        }
                        _ => ServiceStatus::Failed,
                    };
                }
            }
        }
        _ => {}
    }
    result
}

fn playable(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        value
            .get("dash")
            .and_then(|dash| dash.get("video"))
            .or_else(|| value.get("durl"))
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
    })
}

fn gemini_region(code: &str) -> Option<&'static str> {
    // 常见服务端 alpha-3；不猜测未识别代码，不引入全局出口查询。
    Some(match code {
        "USA" => "US",
        "GBR" => "GB",
        "JPN" => "JP",
        "KOR" => "KR",
        "TWN" => "TW",
        "HKG" => "HK",
        "MAC" => "MO",
        "CHN" => "CN",
        "SGP" => "SG",
        "AUS" => "AU",
        "NZL" => "NZ",
        "CAN" => "CA",
        "DEU" => "DE",
        "FRA" => "FR",
        "NLD" => "NL",
        "IND" => "IN",
        "IDN" => "ID",
        "MYS" => "MY",
        "THA" => "TH",
        "VNM" => "VN",
        "PHL" => "PH",
        "RUS" => "RU",
        "BLR" => "BY",
        "CUB" => "CU",
        "IRN" => "IR",
        "PRK" => "KP",
        "SYR" => "SY",
        "ESP" => "ES",
        "ITA" => "IT",
        "BRA" => "BR",
        "MEX" => "MX",
        "TUR" => "TR",
        "CHE" => "CH",
        "SWE" => "SE",
        "NOR" => "NO",
        "FIN" => "FI",
        "DNK" => "DK",
        "POL" => "PL",
        "IRL" => "IE",
        "AUT" => "AT",
        "BEL" => "BE",
        "PRT" => "PT",
        "CZE" => "CZ",
        "ZAF" => "ZA",
        "ARE" => "AE",
        "ISR" => "IL",
        "ARG" => "AR",
        "CHL" => "CL",
        "UKR" => "UA",
        _ => return None,
    })
}

fn redirect_region(reply: &Reply) -> Option<String> {
    if !(300..400).contains(&reply.code) {
        return None;
    }
    let location = reply.location.as_deref()?;
    let base = reqwest::Url::parse("https://www.netflix.com/").ok()?;
    let url = base.join(location).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("www.netflix.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    let path = url.path_segments()?.next()?;
    region(path.split('-').next()?)
}

fn netflix_pair(first: &Reply, second: &Reply) -> Outcome {
    let a = judge("netflix", first);
    let b = judge("netflix", second);
    // 挑战/明确拒绝优先，不能被另一标题成功覆盖。
    for item in [&a, &b] {
        if matches!(
            item.status,
            ServiceStatus::Restricted
                | ServiceStatus::Blocked
                | ServiceStatus::UnsupportedRegion
                | ServiceStatus::HttpError
        ) {
            return item.clone();
        }
    }
    if first.code == 404 && second.code == 404 {
        return Outcome::new(ServiceStatus::OriginalsOnly, Some(404));
    }
    if a.status == ServiceStatus::Available {
        return a;
    }
    if b.status == ServiceStatus::Available {
        return b;
    }
    a
}

fn merge_auxiliary(result: &mut Outcome, region: Option<String>, denied: bool) {
    if result.region.is_none() {
        result.region = region;
    }
    if denied && result.status == ServiceStatus::Available {
        result.status = ServiceStatus::UnsupportedRegion;
    }
}

fn auxiliary_budget(started: std::time::Instant, maximum: Duration) -> Duration {
    // 给外层截止留余量，辅助地区查询不得抹掉已收到的主站 403。
    super::ATTEMPT_TIMEOUT
        .saturating_sub(started.elapsed() + Duration::from_millis(25))
        .min(maximum)
}

async fn chatgpt_auxiliary<T, C>(
    trace: T,
    compliance: C,
    budget: Duration,
) -> (Option<String>, bool, Option<&'static str>)
where
    T: std::future::Future<Output = Result<Reply, Outcome>>,
    C: std::future::Future<Output = Result<Reply, Outcome>>,
{
    let deadline = tokio::time::Instant::now() + budget;
    let (trace, compliance) = tokio::join!(
        tokio::time::timeout_at(deadline, trace),
        tokio::time::timeout_at(deadline, compliance),
    );
    let (region, reason) = match trace {
        Ok(Ok(reply)) => match trace_region(&reply) {
            Some(region) => (Some(region), None),
            None => (None, Some("invalid_response")),
        },
        Err(_) => (None, Some("lookup_timeout")),
        _ => (None, Some("invalid_response")),
    };
    let denied = matches!(compliance, Ok(Ok(ref reply)) if judge("chatgpt", reply).status == ServiceStatus::UnsupportedRegion);
    (region, denied, reason)
}

/// 整条服务规则共享外层单次 4s/总 8s 预算，不因多请求延长重试或改换路线。
pub(super) async fn probe(client: &reqwest::Client, service: Service) -> Outcome {
    let started = std::time::Instant::now();
    let first = match get(client, service.url, body_limit(service.id)).await {
        Ok(reply) => reply,
        Err(error) => return error,
    };
    let mut result = judge(service.id, &first);
    match service.id {
        "chatgpt" => {
            // 两项独立并发；慢合规接口不得丢弃已经拿到的地区。
            let (region, denied, region_reason) = chatgpt_auxiliary(
                get(client, CHATGPT_TRACE, SMALL_LIMIT),
                get(client, CHATGPT_COMPLIANCE, SMALL_LIMIT),
                auxiliary_budget(started, Duration::from_millis(2500)),
            )
            .await;
            merge_auxiliary(&mut result, region, denied);
            result.region_reason = if result.region.is_none() {
                region_reason
            } else {
                None
            };
        }
        "netflix" | "disney" | "spotify" => {
            result = video::probe(
                client,
                service,
                first,
                result,
                auxiliary_budget(started, super::ATTEMPT_TIMEOUT),
            )
            .await;
        }
        "bilibili" if result.status == ServiceStatus::UnsupportedRegion => {
            result = match get(client, BILIBILI_SECOND, SMALL_LIMIT).await {
                Ok(reply) => judge("bilibili", &reply),
                Err(error) => error,
            };
            // 两个片库的播放权不代表单个国家，CN/HK/TW 不据此推导。
        }
        _ => {}
    }
    if result.region.is_none() && result.region_reason.is_none() {
        result.region_reason = Some("not_provided");
    }
    if result.status == ServiceStatus::Failed && result.reason.is_none() {
        result.reason = Some("unrecognized_response");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(code: u16, body: &str) -> Reply {
        Reply {
            code,
            body: body.into(),
            ..Reply::default()
        }
    }

    fn fixture(id: &str, code: u16, body: &str, status: ServiceStatus, region: Option<&str>) {
        let actual = judge(id, &reply(code, body));
        assert_eq!(actual.status, status, "{id} HTTP {code}");
        assert_eq!(actual.http_status, Some(code));
        assert_eq!(actual.region.as_deref(), region, "{id} region");
    }

    #[test]
    fn claude_primary_trace_uses_service_loc_and_preserves_403_semantics() {
        let service = super::super::CATALOG
            .iter()
            .find(|service| service.id == "claude")
            .unwrap();
        assert_eq!(service.url, "https://claude.ai/cdn-cgi/trace");
        assert_eq!(body_limit("claude"), SMALL_LIMIT);
        fixture(
            "claude",
            200,
            "fl=test\nloc=US\nip=192.0.2.1\n",
            ServiceStatus::Available,
            Some("US"),
        );
        fixture(
            "claude",
            200,
            "loc=jp\n",
            ServiceStatus::Available,
            Some("JP"),
        );
        for code in ["AF", "BY", "CN", "CU", "HK", "IR", "KP", "MO", "RU", "SY"] {
            fixture(
                "claude",
                200,
                &format!("loc={code}\n"),
                ServiceStatus::UnsupportedRegion,
                Some(code),
            );
        }
        for body in [
            "",
            "fl=test\n",
            "loc=ZZ\n",
            "loc=USA\n",
            "loc=EU-TTP2\n",
            "<title>Claude</title>",
            "loc=US\ncf-chl-test",
        ] {
            fixture("claude", 200, body, ServiceStatus::Failed, None);
        }
        for body in [
            "<title>Just a moment</title>",
            "/cdn-cgi/challenge-platform/test",
            "loc=US\ncf-chl-test",
            "Verify you are human",
        ] {
            fixture("claude", 403, body, ServiceStatus::Blocked, None);
        }
        for body in [
            "Forbidden",
            "",
            "loc=US\n",
            "unsupported_country",
            "Sorry, you have been blocked",
        ] {
            fixture("claude", 403, body, ServiceStatus::Restricted, None);
        }
        let mut header_challenge = reply(403, "loc=US\n");
        header_challenge.challenge = true;
        assert_eq!(
            judge("claude", &header_challenge).status,
            ServiceStatus::Blocked
        );
        assert_eq!(judge("claude", &header_challenge).http_status, Some(403));
        assert_eq!(judge("claude", &header_challenge).region, None);
        fixture("claude", 302, "loc=US\n", ServiceStatus::Redirect, None);
        fixture("claude", 503, "loc=US\n", ServiceStatus::Unavailable, None);
    }
    #[test]
    fn google_and_github_keep_header_only_reachability_contract() {
        for id in ["google", "github"] {
            for code in [200, 204, 301, 403, 451, 500] {
                fixture(id, code, "cf-chl-challenge", http_status(code), None);
            }
            assert_eq!(check_kind(id), "reachability");
            assert_eq!(body_limit(id), 0);
        }
        for service in &super::super::CATALOG[2..] {
            assert_eq!(check_kind(service.id), "availability");
            assert!(service.url.starts_with("https://"));
        }
    }

    #[test]
    fn every_availability_rule_rejects_empty_generic_2xx_errors_and_challenges() {
        for service in &super::super::CATALOG[2..] {
            for body in [
                "",
                "<html>Welcome</html>",
                "{}",
                "{malformed",
                r#"{"error":"internal"}"#,
            ] {
                fixture(service.id, 200, body, ServiceStatus::Failed, None);
            }
            for code in [200, 403] {
                fixture(
                    service.id,
                    code,
                    "<title>Just a moment</title>",
                    if service.id == "claude" {
                        if code == 403 {
                            ServiceStatus::Blocked
                        } else {
                            ServiceStatus::Failed
                        }
                    } else {
                        ServiceStatus::Restricted
                    },
                    None,
                );
            }
            fixture(
                service.id,
                403,
                "Access denied",
                if service.id == "claude" {
                    ServiceStatus::Restricted
                } else {
                    ServiceStatus::HttpError
                },
                None,
            );
            fixture(service.id, 401, "", ServiceStatus::Restricted, None);
            fixture(service.id, 429, "", ServiceStatus::Restricted, None);
            fixture(service.id, 503, "", ServiceStatus::Unavailable, None);
            fixture(service.id, 302, "", ServiceStatus::Redirect, None);
        }
    }

    #[test]
    fn youtube_checks_premium_offer_and_unavailability_with_service_region() {
        fixture(
            "youtube",
            200,
            r#"{"GL":"jp"} YouTube Premium ad-free"#,
            ServiceStatus::Available,
            Some("JP"),
        );
        fixture(
            "youtube",
            200,
            r#"{"countryCode":"DE"} Premium is not available in your country"#,
            ServiceStatus::UnsupportedRegion,
            Some("DE"),
        );
        fixture(
            "youtube",
            200,
            "YouTube Premium",
            ServiceStatus::Failed,
            None,
        );
        fixture(
            "youtube",
            200,
            "<b id='country-code'>US</b> YouTube Premium ad-free",
            ServiceStatus::Available,
            Some("US"),
        );
    }

    #[test]
    fn youtube_region_and_offer_beyond_old_half_megabyte_limit_are_preserved() {
        let mut body = Body::new(body_limit("youtube"), Some(960_623)).unwrap();
        assert!(body.push(&vec![b' '; 600_000]));
        assert!(body.push(br#"{"GL":"US"} YouTube Premium ad-free"#));
        let reply = reply(200, &String::from_utf8(body.bytes).unwrap());
        assert_eq!(judge("youtube", &reply).status, ServiceStatus::Available);
        assert_eq!(judge("youtube", &reply).region.as_deref(), Some("US"));
        assert!(Body::new(body_limit("youtube"), Some(LARGE_LIMIT as u64 + 1)).is_none());
    }

    #[tokio::test]
    async fn chatgpt_fast_region_survives_slow_compliance() {
        let trace = async { Ok(reply(200, "ip=8.8.8.8\nloc=US\n")) };
        let compliance = std::future::pending::<Result<Reply, Outcome>>();
        let (region, denied, reason) =
            chatgpt_auxiliary(trace, compliance, Duration::from_millis(10)).await;
        assert_eq!(region.as_deref(), Some("US"));
        assert!(!denied);
        assert_eq!(reason, None);
    }

    #[tokio::test]
    async fn chatgpt_region_timeout_and_denial_are_independent() {
        let trace = std::future::pending::<Result<Reply, Outcome>>();
        let compliance = async { Ok(reply(403, "unsupported_country")) };
        let (region, denied, reason) =
            chatgpt_auxiliary(trace, compliance, Duration::from_millis(10)).await;
        assert!(region.is_none());
        assert!(denied);
        assert_eq!(reason, Some("lookup_timeout"));
    }

    #[test]
    fn chatgpt_uses_specific_ios_response_and_explicit_restrictions() {
        fixture(
            "chatgpt",
            403,
            "Request is not allowed. Please try again later.",
            ServiceStatus::Available,
            None,
        );
        fixture(
            "chatgpt",
            403,
            "You may be connected to a disallowed ISP",
            ServiceStatus::DisallowedIsp,
            None,
        );
        fixture(
            "chatgpt",
            403,
            "Sorry, you have been blocked",
            ServiceStatus::Blocked,
            None,
        );
        fixture(
            "chatgpt",
            403,
            r#"{"error":{"code":"unsupported_country_region_territory"}}"#,
            ServiceStatus::UnsupportedRegion,
            None,
        );
        fixture(
            "chatgpt",
            503,
            "Request is not allowed. Please try again later.",
            ServiceStatus::Unavailable,
            None,
        );
    }

    #[test]
    fn gemini_requires_precise_service_marker_and_normalizes_recognized_alpha3() {
        fixture(
            "gemini",
            200,
            r#"[0,2,1,200,"USA",0]"#,
            ServiceStatus::Available,
            Some("US"),
        );
        fixture(
            "gemini",
            200,
            r#"[0,2,1,200,"HKG",0]"#,
            ServiceStatus::UnsupportedRegion,
            Some("HK"),
        );
        fixture(
            "gemini",
            200,
            r#"{"country":"USA"}"#,
            ServiceStatus::Failed,
            None,
        );
        for bad in ["ZZZ", "EU1", "U", "usa"] {
            fixture(
                "gemini",
                200,
                &format!(",2,1,200,\"{bad}\""),
                ServiceStatus::Failed,
                None,
            );
        }
    }

    #[test]
    fn netflix_requires_title_evidence_and_combines_catalog_checks_conservatively() {
        let movie =
            r#"{"@type":"Movie","name":"fixture","publisher":"Netflix","countryCode":"gb"}"#;
        fixture("netflix", 200, movie, ServiceStatus::Available, Some("GB"));
        fixture("netflix", 200, "Netflix Home", ServiceStatus::Failed, None);
        fixture(
            "netflix",
            403,
            "You are using an unblocker or proxy.",
            ServiceStatus::Blocked,
            None,
        );
        fixture(
            "netflix",
            403,
            "Netflix is not available in your country",
            ServiceStatus::UnsupportedRegion,
            None,
        );
        assert_eq!(
            netflix_pair(&reply(404, ""), &reply(404, "")).status,
            ServiceStatus::OriginalsOnly
        );
        assert_eq!(
            netflix_pair(&reply(200, movie), &reply(404, "")).status,
            ServiceStatus::Available
        );
        assert_eq!(
            netflix_pair(&reply(404, ""), &reply(200, movie)).status,
            ServiceStatus::Available
        );
        assert_eq!(
            netflix_pair(&reply(200, movie), &reply(403, "cf-chl-test")).status,
            ServiceStatus::Restricted
        );
        assert_eq!(
            netflix_pair(&reply(404, ""), &reply(500, "")).status,
            ServiceStatus::Unavailable
        );
    }

    #[test]
    fn netflix_region_redirect_is_inspected_without_following_untrusted_targets() {
        for (location, expected) in [
            ("https://www.netflix.com/jp-en/title/80018499", Some("JP")),
            ("/gb/title/80018499", Some("GB")),
            ("https://evil.invalid/us/title/1", None),
            ("http://www.netflix.com/us/title/1", None),
            ("https://www.netflix.com:8443/us/title/1", None),
            ("https://user@www.netflix.com/us/title/1", None),
            ("//127.0.0.1/us/title/1", None),
            ("/zz/title/1", None),
            ("/login", None),
        ] {
            let mut response = reply(302, "");
            response.location = Some(location.into());
            assert_eq!(redirect_region(&response).as_deref(), expected);
            response.code = 200;
            assert_eq!(redirect_region(&response), None);
        }
    }

    #[test]
    fn disney_read_only_page_does_not_claim_token_or_device_based_success() {
        fixture(
            "disney",
            200,
            "<title>Disney+</title> Sign up now",
            ServiceStatus::Failed,
            None,
        );
        fixture(
            "disney",
            200,
            r#"{"countryCode":"tw"} Disney+ is coming soon"#,
            ServiceStatus::ComingSoon,
            Some("TW"),
        );
        fixture(
            "disney",
            403,
            "Disney+ is not available in your location",
            ServiceStatus::UnsupportedRegion,
            None,
        );
        fixture(
            "disney",
            200,
            r#"{"countryCode":"US","inSupportedLocation":true}"#,
            ServiceStatus::Failed,
            Some("US"),
        );
    }

    #[test]
    fn prime_uses_territory_and_boolean_value_instead_of_field_presence() {
        fixture(
            "prime",
            200,
            r#"{"currentTerritory":"au","isServiceRestricted":false}"#,
            ServiceStatus::Available,
            Some("AU"),
        );
        fixture(
            "prime",
            200,
            r#"{"currentTerritory":"CN","isServiceRestricted":true}"#,
            ServiceStatus::UnsupportedRegion,
            Some("CN"),
        );
        fixture(
            "prime",
            200,
            r#"{"currentTerritory":"CA"}"#,
            ServiceStatus::Available,
            Some("CA"),
        );
        fixture(
            "prime",
            200,
            r#"{"isServiceRestricted":false}"#,
            ServiceStatus::Failed,
            None,
        );
        fixture(
            "prime",
            403,
            r#"{"currentTerritory":"US","isServiceRestricted":false}"#,
            ServiceStatus::HttpError,
            Some("US"),
        );
    }

    #[test]
    fn spotify_and_discord_require_valid_service_json_and_country_fields() {
        fixture(
            "spotify",
            200,
            r#"{"countryCode":"ca"}"#,
            ServiceStatus::Available,
            Some("CA"),
        );
        fixture(
            "spotify",
            403,
            "Spotify is not available in your country",
            ServiceStatus::UnsupportedRegion,
            None,
        );
        fixture(
            "discord",
            200,
            r#"{"country_code":"kr","consent_required":false}"#,
            ServiceStatus::Available,
            Some("KR"),
        );
        for (id, field) in [("spotify", "countryCode"), ("discord", "country_code")] {
            for raw in ["ZZ", "USA", "", "EU-TTP2", "u1", "<script>"] {
                fixture(
                    id,
                    200,
                    &format!(r#"{{"{field}":"{raw}"}}"#),
                    ServiceStatus::Failed,
                    None,
                );
            }
            fixture(
                id,
                200,
                &format!(r#"<p>"{field}":"US"</p>"#),
                ServiceStatus::Failed,
                None,
            );
        }
    }

    #[test]
    fn tiktok_uses_visitor_context_not_serving_cluster() {
        fixture(
            "tiktok",
            200,
            r#"{"region":"EU-TTP2","webapp.app-context":{"region":"sg","language":"en"}}"#,
            ServiceStatus::Available,
            Some("SG"),
        );
        fixture(
            "tiktok",
            200,
            r#"{"region":"US"}"#,
            ServiceStatus::Failed,
            None,
        );
        fixture(
            "tiktok",
            200,
            r#"{"webapp.app-context":{"region":"EU-TTP2"}}"#,
            ServiceStatus::Failed,
            None,
        );
        fixture(
            "tiktok",
            403,
            "TikTok is not available in your region",
            ServiceStatus::UnsupportedRegion,
            None,
        );
    }

    #[test]
    fn bilibili_requires_playback_data_and_does_not_infer_country_from_catalog() {
        fixture(
            "bilibili",
            200,
            r#"{"code":0,"result":{"dash":{"video":[{"id":1}]}}}"#,
            ServiceStatus::Available,
            None,
        );
        fixture(
            "bilibili",
            200,
            r#"{"code":0,"data":{"durl":[{"url":"https://media.invalid/fixture"}]}}"#,
            ServiceStatus::Available,
            None,
        );
        fixture(
            "bilibili",
            200,
            r#"{"code":-10403,"message":"region denied"}"#,
            ServiceStatus::UnsupportedRegion,
            None,
        );
        for body in [
            r#"{"code":0}"#,
            r#"{"code":-101}"#,
            r#"{"code":"0"}"#,
            r#"{"code":0,"result":{"dash":{"video":[]}}}"#,
        ] {
            fixture("bilibili", 200, body, ServiceStatus::Failed, None);
        }
    }

    #[test]
    fn region_validation_never_emits_non_country_strings() {
        assert_eq!(region(" jp "), Some("JP".into()));
        for raw in [
            "ZZ", "EU", "EU-TTP2", "123", "USA", "日本", "A", "A\0", "UＳ",
        ] {
            assert_eq!(region(raw), None, "{raw}");
        }
    }

    #[test]
    fn body_limits_cover_content_length_chunked_exact_boundary_and_split_markers() {
        assert!(Body::new(SMALL_LIMIT, Some(SMALL_LIMIT as u64 + 1)).is_none());
        assert!(Body::new(SMALL_LIMIT, Some(u64::MAX)).is_none());
        for limit in [SMALL_LIMIT, HTML_LIMIT, LARGE_LIMIT] {
            let mut body = Body::new(limit, None).unwrap();
            assert!(body.push(&vec![b'x'; limit - 1]));
            assert!(body.push(b"x"));
            assert!(!body.push(b"x"));
            assert_eq!(body.bytes.len(), limit);
        }
        let mut body = Body::new(64, Some(0)).unwrap();
        for chunk in [b"cf-".as_slice(), b"chl-", b"challenge"] {
            assert!(body.push(chunk));
        }
        let reply = reply(403, &String::from_utf8(body.bytes).unwrap());
        assert_eq!(judge("claude", &reply).status, ServiceStatus::Blocked);
        let mut lying_length = Body::new(3, Some(1)).unwrap();
        assert!(lying_length.push(b"abc"));
        assert!(!lying_length.push(b"d"));
    }
}
