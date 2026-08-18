//! Helpers for reading loosely-typed Clash YAML fields.

use serde_yaml::Value;

pub fn as_mapping(value: &Value) -> Option<&serde_yaml::Mapping> {
    value.as_mapping()
}

pub fn get_str(map: &serde_yaml::Mapping, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(v) = map.get(Value::String((*key).into())) {
            if let Some(s) = value_to_string(v) {
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
    }
    None
}

/// Reads a list-of-strings field. Accepts a YAML sequence (e.g. `alpn: [h2, http/1.1]`,
/// the standard mihomo form) or a single comma-separated string.
pub fn get_str_list(map: &serde_yaml::Mapping, keys: &[&str]) -> Option<Vec<String>> {
    for key in keys {
        if let Some(v) = map.get(Value::String((*key).into())) {
            match v {
                Value::Sequence(items) => {
                    let list: Vec<String> = items.iter().filter_map(value_to_string).collect();
                    if !list.is_empty() {
                        return Some(list);
                    }
                }
                _ => {
                    if let Some(s) = value_to_string(v) {
                        let list: Vec<String> = s
                            .split(',')
                            .map(|p| p.trim().to_string())
                            .filter(|p| !p.is_empty())
                            .collect();
                        if !list.is_empty() {
                            return Some(list);
                        }
                    }
                }
            }
        }
    }
    None
}

pub fn get_bool(map: &serde_yaml::Mapping, keys: &[&str]) -> Option<bool> {
    for key in keys {
        if let Some(v) = map.get(Value::String((*key).into())) {
            if let Some(b) = v.as_bool() {
                return Some(b);
            }
            if let Some(s) = v.as_str() {
                match s.to_ascii_lowercase().as_str() {
                    "true" | "1" | "yes" => return Some(true),
                    "false" | "0" | "no" => return Some(false),
                    _ => {}
                }
            }
            if let Some(n) = v.as_i64() {
                return Some(n != 0);
            }
        }
    }
    None
}

pub fn get_u16(map: &serde_yaml::Mapping, keys: &[&str]) -> Option<u16> {
    for key in keys {
        if let Some(v) = map.get(Value::String((*key).into())) {
            if let Some(n) = v.as_u64() {
                if n <= u16::MAX as u64 {
                    return Some(n as u16);
                }
            }
            if let Some(n) = v.as_i64() {
                if n >= 0 && n <= u16::MAX as i64 {
                    return Some(n as u16);
                }
            }
            if let Some(s) = v.as_str() {
                if let Ok(n) = s.parse::<u16>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

pub fn get_u32(map: &serde_yaml::Mapping, keys: &[&str]) -> Option<u32> {
    for key in keys {
        if let Some(v) = map.get(Value::String((*key).into())) {
            if let Some(n) = v.as_u64() {
                if n <= u32::MAX as u64 {
                    return Some(n as u32);
                }
            }
            if let Some(s) = v.as_str() {
                if let Some(mbps) = rate_str_to_mbps(s) {
                    return Some(mbps);
                }
            }
        }
    }
    None
}

/// Parses hysteria-style rate strings ("100", "100Mbps", "50 KBps", "1Gbps") into
/// a whole-number Mbps value, matching clash2singbox's `anyToMbps`. K/M/G/T are
/// powers of 1000 relative to Mbps; an uppercase `B` (bytes) multiplies by 8.
fn rate_str_to_mbps(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Ok(n) = s.parse::<u32>() {
        return Some(n);
    }
    let s = s.strip_suffix("ps")?;
    let (num_part, unit) = match s.strip_suffix(['B', 'b']) {
        Some(rest) => (rest, &s[rest.len()..]),
        None => return None,
    };
    let (num_part, scale) = match num_part.strip_suffix(['K', 'M', 'G', 'T']) {
        Some(rest) => (rest, num_part.as_bytes()[rest.len()] as char),
        None => (num_part, 'M'),
    };
    let num_part = num_part.trim();
    let value: f64 = num_part.parse().ok()?;
    let mut factor = match scale {
        'K' => 1.0 / 1000.0,
        'M' => 1.0,
        'G' => 1000.0,
        'T' => 1_000_000.0,
        _ => return None,
    };
    if unit == "B" {
        factor *= 8.0;
    }
    let mbps = (value * factor) as u32;
    Some(mbps.max(1))
}

pub fn get_map<'a>(map: &'a serde_yaml::Mapping, keys: &[&str]) -> Option<&'a serde_yaml::Mapping> {
    for key in keys {
        if let Some(v) = map.get(Value::String((*key).into())) {
            if let Some(m) = v.as_mapping() {
                return Some(m);
            }
        }
    }
    None
}

pub fn value_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

pub fn map_to_string_map(map: &serde_yaml::Mapping) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for (k, v) in map {
        if let (Some(ks), Some(vs)) = (value_to_string(k), value_to_string(v)) {
            out.insert(ks, vs);
        }
    }
    out
}
