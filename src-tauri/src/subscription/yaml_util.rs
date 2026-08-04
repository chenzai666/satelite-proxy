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
                // hysteria up/down sometimes "100 Mbps"
                let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(n) = digits.parse::<u32>() {
                    return Some(n);
                }
            }
        }
    }
    None
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
