//! 固定服务端点的独立协议实现；参考 FlClash 的检测目标与状态语义，未移植其实现。
//! 不使用账号、公共客户端密钥或访客令牌；不记录正文、Cookie 和重定向原文。

use super::{Outcome, Reply, Service, ServiceStatus};
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeSet;
use std::time::Duration;

const NETFLIX_SECOND: &str = "https://www.netflix.com/title/70143836";
const NETFLIX_REGION: &str = "https://www.netflix.com/title/80018499";
const SPOTIFY_API: &str = "/api/content/v1/country-selector";
const SPOTIFY_LIMIT: usize = 512 * 1024;

pub(super) async fn probe(
    client: &reqwest::Client,
    service: Service,
    first: Reply,
    result: Outcome,
    budget: Duration,
) -> Outcome {
    // 所有 await 仍处于父调用的单次 4s / 总计 8s 截止内，不增加重试层。
    match service.id {
        "netflix" => netflix(client, first, budget).await,
        "spotify" => spotify(client, service.url, first).await,
        "disney" => disney(&first),
        _ => result,
    }
}

fn outcome(status: ServiceStatus, code: u16) -> Outcome {
    Outcome::new(status, Some(code))
}

fn challenge(reply: &Reply) -> bool {
    super::challenged(reply)
}

fn netflix_pair(first: &Reply, second: &Reply) -> Outcome {
    for reply in [first, second] {
        if challenge(reply) {
            let mut result = outcome(ServiceStatus::Restricted, reply.code);
            result.reason = Some("challenge");
            return result;
        }
        if reply.code == 403 {
            return outcome(ServiceStatus::UnsupportedRegion, reply.code);
        }
    }
    if first.code == 404 && second.code == 404 {
        return outcome(ServiceStatus::OriginalsOnly, 404);
    }
    // 这是两个已知片目端点的协议判定，不要求页面恰好包含某种 JSON-LD 排版。
    for reply in [first, second] {
        if matches!(reply.code, 200 | 301) {
            return outcome(ServiceStatus::Available, reply.code);
        }
    }
    super::judge("netflix", first)
}

fn netflix_region(reply: &Reply) -> Option<String> {
    if !matches!(reply.code, 301 | 302 | 303 | 307 | 308) || challenge(reply) {
        return None;
    }
    let base = reqwest::Url::parse(NETFLIX_REGION).ok()?;
    let url = base.join(reply.location.as_deref()?).ok()?;
    if !fixed_host(&url, "www.netflix.com") || url.fragment().is_some() {
        return None;
    }
    let segments: Vec<_> = url.path_segments()?.collect();
    if segments.len() != 3 || segments[1] != "title" || segments[2] != "80018499" {
        return None;
    }
    locale_region(segments[0])
}

async fn netflix(client: &reqwest::Client, first: Reply, budget: Duration) -> Outcome {
    if challenge(&first) || first.code == 403 {
        return netflix_pair(&first, &first);
    }
    // 地区是辅助信息，与第二片目并发；慢地区接口不会覆盖已确定的片目状态。
    let (second, geo) = tokio::join!(
        super::get(client, NETFLIX_SECOND, 0),
        tokio::time::timeout(
            budget
                .saturating_sub(Duration::from_millis(50))
                .min(Duration::from_millis(1500)),
            super::get(client, NETFLIX_REGION, 0),
        ),
    );
    let second = match second {
        Ok(reply) => reply,
        Err(error) => return error,
    };
    let mut result = netflix_pair(&first, &second);
    if matches!(
        result.status,
        ServiceStatus::Available | ServiceStatus::OriginalsOnly
    ) {
        // 只接受 Netflix 的请求地区字段，不用影片产地 country 或 CDN 地区替代。
        result.region = super::field_region(&first.body, "requestCountry")
            .or_else(|| super::field_region(&first.body, "currentCountry"))
            .or_else(|| geo.as_ref().ok()?.as_ref().ok().and_then(netflix_region));
        if result.region.is_none() {
            result.region_reason = Some(if geo.is_err() {
                "lookup_timeout"
            } else {
                "not_provided"
            });
        }
    }
    result
}

fn fixed_host(url: &reqwest::Url, host: &str) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some(host)
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
}

fn locale_region(locale: &str) -> Option<String> {
    let mut parts = locale.split('-');
    let country = super::region(parts.next()?)?;
    if let Some(language) = parts.next() {
        if !(2..=3).contains(&language.len()) || !language.bytes().all(|c| c.is_ascii_alphabetic())
        {
            return None;
        }
    }
    (parts.next().is_none()).then_some(country)
}

/// 仅允许原服务 HTTPS 主机上的本地化 API，校验后重建固定查询参数。
fn spotify_redirect(base: &reqwest::Url, location: &str) -> Option<(reqwest::Url, String)> {
    if location.len() > 2048 || location.chars().any(char::is_control) {
        return None;
    }
    let mut url = base.join(location).ok()?;
    if !fixed_host(&url, "www.spotify.com") || url.fragment().is_some() {
        return None;
    }
    let path = url.path().trim_end_matches('/');
    let (locale, rest) = path.strip_prefix('/')?.split_once('/')?;
    let country = locale_region(locale)?;
    if format!("/{rest}") != SPOTIFY_API {
        return None;
    }
    let mut seen = BTreeSet::new();
    for (key, value) in url.query_pairs() {
        if !matches!(
            (key.as_ref(), value.as_ref()),
            ("platform", "web") | ("format", "json")
        ) || !seen.insert(key.into_owned())
        {
            return None;
        }
    }
    url.set_query(Some("platform=web&format=json"));
    Some((url, country))
}

fn spotify_result(reply: &Reply, localized_region: Option<String>) -> Outcome {
    if challenge(reply) {
        let mut result = outcome(ServiceStatus::Restricted, reply.code);
        result.reason = Some("challenge");
        return result;
    }
    if matches!(reply.code, 403 | 451) {
        return outcome(ServiceStatus::UnsupportedRegion, reply.code);
    }
    let mut result = super::judge("spotify", reply);
    if reply.code != 200 || result.status == ServiceStatus::UnsupportedRegion {
        if result.status != ServiceStatus::Failed {
            result.reason = None;
        }
        return result;
    }
    let payload = serde_json::from_str::<Value>(&reply.body).ok();
    let structured = payload.as_ref().is_some_and(|value| match value {
        Value::Object(fields) => !fields.is_empty(),
        Value::Array(items) => !items.is_empty(),
        _ => false,
    });
    // 官方优先采用实际重定向地区；无重定向时才采用 API 的 countryCode。
    result.region = localized_region.or_else(|| {
        payload
            .as_ref()?
            .get("countryCode")?
            .as_str()
            .and_then(super::region)
    });
    if structured && result.region.is_some() {
        result.status = ServiceStatus::Available;
        result.reason = None;
    } else {
        result.status = ServiceStatus::Failed;
        result.reason = Some("unrecognized_response");
    }
    result
}

/// 仅接收上方白名单验证后产生的本地化 URL，复用统一限制与连接证据。
async fn get_spotify(client: &reqwest::Client, url: reqwest::Url) -> Result<Reply, Outcome> {
    super::get(client, url.as_str(), SPOTIFY_LIMIT).await
}

async fn spotify(client: &reqwest::Client, original: &str, mut reply: Reply) -> Outcome {
    let Ok(mut url) = reqwest::Url::parse(original) else {
        return outcome(ServiceStatus::Failed, reply.code);
    };
    let mut country = None;
    let mut visited = BTreeSet::from([url.as_str().to_owned()]);
    // 客户端保持禁止自动跳转；最多两跳，每跳重新检查完整目标。
    for _ in 0..2 {
        if !matches!(reply.code, 301 | 302 | 303 | 307 | 308) || challenge(&reply) {
            return spotify_result(&reply, country);
        }
        let target = reply
            .location
            .as_deref()
            .and_then(|location| spotify_redirect(&url, location));
        let Some((next, region)) = target else {
            let mut result = outcome(ServiceStatus::Redirect, reply.code);
            result.reason = Some("unsafe_redirect");
            return result;
        };
        if !visited.insert(next.as_str().to_owned()) {
            let mut result = outcome(ServiceStatus::Redirect, reply.code);
            result.reason = Some("redirect_loop");
            return result;
        }
        reply = match get_spotify(client, next.clone()).await {
            Ok(reply) => reply,
            Err(error) => return error,
        };
        url = next;
        country = Some(region);
    }
    let mut result = spotify_result(&reply, country);
    if result.status == ServiceStatus::Redirect {
        result.reason = Some("redirect_limit");
    }
    result
}

fn disney_json(body: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str(body) {
        return Some(value);
    }
    let script =
        Regex::new(r#"(?is)<script\b[^>]*\bid\s*=\s*["']__NEXT_DATA__["'][^>]*>(.*?)</script\s*>"#)
            .ok()?;
    let captures = script.captures(body)?;
    serde_json::from_str(captures.get(1)?.as_str()).ok()
}

#[derive(Default)]
struct DisneyEvidence {
    regions: BTreeSet<String>,
    supported: BTreeSet<bool>,
    denied: bool,
    remaining: usize,
    incomplete: bool,
}

impl DisneyEvidence {
    fn scan(&mut self, value: &Value, depth: usize) {
        if depth > 24 || self.remaining == 0 {
            self.incomplete = true;
            return;
        }
        self.remaining -= 1;
        match value {
            Value::Object(fields) => {
                for name in ["countryCode", "region", "urlRegion"] {
                    if let Some(country) = fields
                        .get(name)
                        .and_then(Value::as_str)
                        .and_then(super::region)
                    {
                        self.regions.insert(country);
                    }
                }
                // 此字段是服务会话的明确可用性结论，不能用 availableLocales 等配置代替。
                if let Some(supported) = fields.get("inSupportedLocation").and_then(Value::as_bool)
                {
                    self.supported.insert(supported);
                }
                self.denied |= fields
                    .get("code")
                    .and_then(Value::as_str)
                    .is_some_and(|code| code == "forbidden-location");
                for child in fields.values() {
                    self.scan(child, depth + 1);
                }
            }
            Value::Array(items) => {
                for child in items {
                    self.scan(child, depth + 1);
                }
            }
            _ => {}
        }
    }
}

fn disney(reply: &Reply) -> Outcome {
    if challenge(reply) || reply.code != 200 {
        return super::judge("disney", reply);
    }
    let mut result = outcome(ServiceStatus::Failed, reply.code);
    let Some(value) = disney_json(&reply.body) else {
        result.reason = Some("availability_requires_session");
        return result;
    };
    // NEXT_DATA 只检查服务端页面上下文，排除路由配置、静态构建清单等顶层对象。
    let context = value.pointer("/props/pageProps").unwrap_or(&value);
    let mut evidence = DisneyEvidence {
        remaining: 16_384,
        ..DisneyEvidence::default()
    };
    evidence.scan(context, 0);
    if evidence.incomplete {
        // 未完成扫描时不采用片段中的可用性，地区只保留顶层明确的请求上下文。
        evidence.supported.clear();
        evidence.denied = false;
        evidence.regions.clear();
        for name in ["countryCode", "region", "urlRegion"] {
            if let Some(country) = context
                .get(name)
                .and_then(Value::as_str)
                .and_then(super::region)
            {
                evidence.regions.insert(country);
            }
        }
    }
    if evidence.regions.len() == 1 {
        result.region = evidence.regions.into_iter().next();
    } else {
        result.region_reason = Some(if evidence.regions.is_empty() {
            "not_provided"
        } else {
            "conflicting_region"
        });
    }
    if evidence.denied {
        result.status = ServiceStatus::UnsupportedRegion;
        result.reason = None;
    } else if evidence.supported.len() == 1 && result.region.is_some() {
        result.status = if evidence.supported.contains(&true) {
            ServiceStatus::Available
        } else {
            ServiceStatus::ComingSoon
        };
        result.reason = None;
    } else {
        // 真实 HTTP 200 营销页只提供地区，不能据此冒充播放/订阅解锁。
        result.reason = Some("availability_requires_session");
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

    #[test]
    fn netflix_fixed_titles_accept_200_without_json_ld() {
        let first = reply(200, "<html>Netflix</html>");
        let result = netflix_pair(&first, &reply(404, ""));
        assert_eq!(result.status, ServiceStatus::Available);
        assert_eq!(result.http_status, Some(200));
        assert_eq!(result.reason, None);
        assert_eq!(
            netflix_pair(&reply(404, ""), &reply(404, "")).status,
            ServiceStatus::OriginalsOnly
        );
        assert_eq!(
            netflix_pair(&first, &reply(403, "")).status,
            ServiceStatus::UnsupportedRegion
        );
        assert_eq!(
            netflix_pair(&reply(301, ""), &reply(404, "")).status,
            ServiceStatus::Available
        );
    }

    #[test]
    fn netflix_challenge_is_not_a_successful_title() {
        let mut first = reply(200, "");
        first.challenge = true;
        let result = netflix_pair(&first, &reply(200, ""));
        assert_eq!(result.status, ServiceStatus::Restricted);
        assert_eq!(result.reason, Some("challenge"));
    }

    #[test]
    fn netflix_region_accepts_only_its_exact_title_redirect() {
        let mut value = reply(302, "");
        for location in [
            "https://www.netflix.com/jp-en/title/80018499",
            "/jp/title/80018499",
        ] {
            value.location = Some(location.into());
            assert_eq!(netflix_region(&value).as_deref(), Some("JP"));
        }
        for location in [
            "https://evil.example/jp/title/80018499",
            "http://www.netflix.com/jp/title/80018499",
            "/jp/login",
            "/ZZ/title/80018499",
        ] {
            value.location = Some(location.into());
            assert_eq!(netflix_region(&value), None);
        }
    }

    #[test]
    fn spotify_real_us_redirect_is_accepted_and_query_is_fixed() {
        let base = reqwest::Url::parse(
            "https://www.spotify.com/api/content/v1/country-selector?platform=web&format=json",
        )
        .unwrap();
        for location in [
            "https://www.spotify.com/us/api/content/v1/country-selector/",
            "/us-en/api/content/v1/country-selector/?format=json&platform=web",
        ] {
            let (url, region) = spotify_redirect(&base, location).unwrap();
            assert_eq!(region, "US");
            assert_eq!(url.query(), Some("platform=web&format=json"));
        }
    }

    #[test]
    fn spotify_rejects_foreign_hosts_downgrades_credentials_and_other_endpoints() {
        let base = reqwest::Url::parse("https://www.spotify.com/").unwrap();
        for location in [
            "https://www.spotify.com.evil.example/us/api/content/v1/country-selector/",
            "https://www.spotify.com@evil.example/us/api/content/v1/country-selector/",
            "https://user@www.spotify.com/us/api/content/v1/country-selector/",
            "https://www.spotify.com:8443/us/api/content/v1/country-selector/",
            "http://www.spotify.com/us/api/content/v1/country-selector/",
            "//evil.example/us/api/content/v1/country-selector/",
            "/us/login",
            "/us/",
            "/zz/api/content/v1/country-selector/",
            "/us/api/content/v1/country-selector/?next=https://evil.example",
            "/us/api/content/v1/country-selector/?format=json&format=json",
            "/us/api/content/v1/country-selector/#fragment",
        ] {
            assert!(
                spotify_redirect(&base, location).is_none(),
                "unsafe target accepted"
            );
        }
    }

    #[test]
    fn spotify_localized_api_does_not_require_redundant_country_field() {
        let value = reply(200, r#"{"countries":[{"id":"US"}]}"#);
        let result = spotify_result(&value, Some("US".into()));
        assert_eq!(result.status, ServiceStatus::Available);
        assert_eq!(result.region.as_deref(), Some("US"));
        assert_eq!(result.reason, None);
        let value = reply(200, r#"{"countryCode":"JP"}"#);
        assert_eq!(
            spotify_result(&value, Some("US".into())).region.as_deref(),
            Some("US")
        );
        assert_eq!(spotify_result(&value, None).region.as_deref(), Some("JP"));
    }

    #[test]
    fn spotify_rejects_marketing_and_preserves_denials() {
        for body in ["", "{}", "[]", "<html>Spotify</html>"] {
            assert_eq!(
                spotify_result(&reply(200, body), Some("US".into())).status,
                ServiceStatus::Failed
            );
        }
        for code in [403, 451] {
            assert_eq!(
                spotify_result(&reply(code, ""), Some("US".into())).status,
                ServiceStatus::UnsupportedRegion
            );
        }
        assert_eq!(
            spotify_result(
                &reply(200, "not available in your country"),
                Some("US".into())
            )
            .status,
            ServiceStatus::UnsupportedRegion
        );
    }

    #[test]
    fn disney_next_data_region_is_not_a_marketing_page_unlock() {
        let value = reply(
            200,
            r#"<script type="application/json" id="__NEXT_DATA__">{"props":{"pageProps":{"region":"us","availableLocales":["en-US"]}}}</script>"#,
        );
        let result = disney(&value);
        assert_eq!(result.region.as_deref(), Some("US"));
        assert_eq!(result.status, ServiceStatus::Failed);
        assert_eq!(result.reason, Some("availability_requires_session"));
    }

    #[test]
    fn disney_explicit_session_evidence_controls_availability() {
        for (supported, expected) in [
            (true, ServiceStatus::Available),
            (false, ServiceStatus::ComingSoon),
        ] {
            let body = format!(
                r#"{{"activeSession":{{"location":{{"countryCode":"JP"}},"inSupportedLocation":{supported}}}}}"#
            );
            let result = disney(&reply(200, &body));
            assert_eq!(result.region.as_deref(), Some("JP"));
            assert_eq!(result.status, expected);
            assert_eq!(result.reason, None);
        }
        assert_eq!(
            disney(&reply(200, r#"{"errors":[{"code":"forbidden-location"}]}"#)).status,
            ServiceStatus::UnsupportedRegion
        );
    }

    #[test]
    fn disney_conflicting_or_invalid_regions_never_claim_unlock() {
        for body in [
            r#"{"region":"EU","inSupportedLocation":true}"#,
            r#"{"region":"US","nested":{"region":"JP"},"inSupportedLocation":true}"#,
            r#"{"region":"US","nested":{"inSupportedLocation":false},"inSupportedLocation":true}"#,
        ] {
            assert_eq!(disney(&reply(200, body)).status, ServiceStatus::Failed);
        }
        let result = disney(&reply(
            503,
            r#"{"countryCode":"US","inSupportedLocation":true}"#,
        ));
        assert_ne!(result.status, ServiceStatus::Available);
    }

    #[test]
    fn disney_partial_tree_cannot_hide_conflicting_evidence() {
        let mut nested = serde_json::json!({"inSupportedLocation": false});
        for _ in 0..30 {
            nested = serde_json::json!({"nested": nested});
        }
        let body = serde_json::json!({
            "region": "US", "inSupportedLocation": true, "nested": nested,
        })
        .to_string();
        let result = disney(&reply(200, &body));
        assert_eq!(result.status, ServiceStatus::Failed);
        assert_eq!(result.region.as_deref(), Some("US"));
        assert_eq!(result.reason, Some("availability_requires_session"));
    }
}
