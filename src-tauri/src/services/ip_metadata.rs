//! 固定 HTTPS 数据源，查询指定出口的组织/ASN/风险；缺字段保持未知。
use super::network_probe::{build_client, ProbeRoute};
use serde::Serialize;
use serde_json::Value;
use std::net::IpAddr;
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
    pub source: &'static str,
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
        asn: text(&value["isp"]["asn"]),
        network_type,
        residential_inferred: false,
        risk_score: score,
        quality,
        is_vpn: value["risk"]["is_vpn"].as_bool(),
        is_proxy: value["risk"]["is_proxy"].as_bool(),
        is_tor: tor,
        source: "IPQuery",
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
    if value["status"].as_str() != Some("ok") {
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
    };
    true
}

pub async fn query(ip: IpAddr, route: ProbeRoute) -> Result<IpMetadata, String> {
    let client = build_client(route, Duration::from_secs(7))?;
    let (primary, declared) = tokio::join!(
        get_json(&client, format!("https://api.ipquery.io/{ip}?format=json")),
        get_json(&client, format!("https://proxycheck.io/v3/{ip}")),
    );
    let primary = primary.and_then(|value| parse_metadata(ip, &value));
    let mut data = match &primary {
        Ok(data) => data.clone(),
        Err(_) => {
            let mut data = parse_metadata(ip, &serde_json::json!({"ip":ip.to_string()}))?;
            data.source = "";
            data
        }
    };
    let has_declared = declared
        .as_ref()
        .is_ok_and(|value| apply_declared_type(&mut data, value));
    if primary.is_err() && !has_declared {
        return Err("IP 信息服务暂不可用".into());
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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
