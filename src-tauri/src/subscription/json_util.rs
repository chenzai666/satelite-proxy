//! Helpers for reading loosely-typed JSON (sing-box / Clash-as-JSON).

use serde_json::{Map, Value};

pub fn as_object(value: &Value) -> Option<&Map<String, Value>> {
    value.as_object()
}

pub fn get_str(obj: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(v) = obj.get(*key) {
            if let Some(s) = value_to_string(v) {
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
    }
    None
}

pub fn get_bool(obj: &Map<String, Value>, keys: &[&str]) -> Option<bool> {
    for key in keys {
        if let Some(v) = obj.get(*key) {
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

pub fn get_u16(obj: &Map<String, Value>, keys: &[&str]) -> Option<u16> {
    for key in keys {
        if let Some(v) = obj.get(*key) {
            if let Some(n) = v.as_u64() {
                if n <= u16::MAX as u64 {
                    return Some(n as u16);
                }
            }
            if let Some(n) = v.as_i64() {
                if (0..=u16::MAX as i64).contains(&n) {
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

pub fn get_u32(obj: &Map<String, Value>, keys: &[&str]) -> Option<u32> {
    for key in keys {
        if let Some(v) = obj.get(*key) {
            if let Some(n) = v.as_u64() {
                if n <= u32::MAX as u64 {
                    return Some(n as u32);
                }
            }
            if let Some(s) = v.as_str() {
                let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(n) = digits.parse::<u32>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

pub fn get_u8(obj: &Map<String, Value>, keys: &[&str]) -> Option<u8> {
    get_u16(obj, keys).and_then(|n| u8::try_from(n).ok())
}

pub fn get_obj<'a>(obj: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Map<String, Value>> {
    for key in keys {
        if let Some(v) = obj.get(*key) {
            if let Some(m) = v.as_object() {
                return Some(m);
            }
        }
    }
    None
}

pub fn get_str_list(obj: &Map<String, Value>, keys: &[&str]) -> Option<Vec<String>> {
    for key in keys {
        if let Some(v) = obj.get(*key) {
            match v {
                Value::Array(items) => {
                    let list: Vec<String> = items.iter().filter_map(value_to_string).collect();
                    if !list.is_empty() {
                        return Some(list);
                    }
                }
                Value::String(s) => {
                    let list: Vec<String> = s
                        .split([',', ';', '\n'])
                        .map(|p| p.trim().to_string())
                        .filter(|p| !p.is_empty())
                        .collect();
                    if !list.is_empty() {
                        return Some(list);
                    }
                }
                _ => {}
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

pub fn map_to_string_map(obj: &Map<String, Value>) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for (k, v) in obj {
        if let Some(vs) = value_to_string(v) {
            out.insert(k.clone(), vs);
        }
    }
    out
}
