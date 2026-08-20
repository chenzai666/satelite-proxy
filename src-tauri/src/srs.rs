//! Structural parser for sing-box binary rule-sets (`.srs`).
//!
//! Mirrors the wire formats of sing-box `common/srs` and
//! `sagernet/sing common/domain` (LOUDS succinct tries), so rule-sets that
//! embed AdGuard DNS filter rules can be validated, counted and listed.
//! Those are rejected by `sing-box rule-set decompile` ("unable to
//! decompile binary AdGuard rules to rule-set") even though the kernel
//! loads them fine, so they must be parsed here instead of via the core.

use std::io::Read;

const MAGIC: &[u8; 3] = b"SRS";
/// sing-box `constant.RuleSetVersionCurrent` (v1.13).
const MAX_VERSION: u8 = 4;
/// Downloads are capped at 32 MB compressed; bound inflation so a crafted
/// archive cannot exhaust memory during validation.
const MAX_DECOMPRESSED: usize = 512 * 1024 * 1024;
const MAX_LOGICAL_DEPTH: u8 = 8;

const RULE_TYPE_DEFAULT: u8 = 0;
const RULE_TYPE_LOGICAL: u8 = 1;

const ITEM_QUERY_TYPE: u8 = 0;
const ITEM_NETWORK: u8 = 1;
const ITEM_DOMAIN: u8 = 2;
const ITEM_DOMAIN_KEYWORD: u8 = 3;
const ITEM_DOMAIN_REGEX: u8 = 4;
const ITEM_SOURCE_IP_CIDR: u8 = 5;
const ITEM_IP_CIDR: u8 = 6;
const ITEM_SOURCE_PORT: u8 = 7;
const ITEM_SOURCE_PORT_RANGE: u8 = 8;
const ITEM_PORT: u8 = 9;
const ITEM_PORT_RANGE: u8 = 10;
const ITEM_PROCESS_NAME: u8 = 11;
const ITEM_PROCESS_PATH: u8 = 12;
const ITEM_PACKAGE_NAME: u8 = 13;
const ITEM_WIFI_SSID: u8 = 14;
const ITEM_WIFI_BSSID: u8 = 15;
const ITEM_ADGUARD_DOMAIN: u8 = 16;
const ITEM_PROCESS_PATH_REGEX: u8 = 17;
const ITEM_NETWORK_TYPE: u8 = 18;
const ITEM_NETWORK_IS_EXPENSIVE: u8 = 19;
const ITEM_NETWORK_IS_CONSTRAINED: u8 = 20;
const ITEM_NETWORK_INTERFACE_ADDRESS: u8 = 21;
const ITEM_DEFAULT_INTERFACE_ADDRESS: u8 = 22;
const ITEM_FINAL: u8 = 0xFF;

// Marker bytes inside succinct domain tries (sing common/domain).
const LABEL_PREFIX: u8 = b'\r';
const LABEL_ROOT: u8 = b'\n';
const LABEL_SUFFIX: char = '\u{8}';

/// A structurally validated binary rule-set.
pub(crate) struct ParsedSrs {
    pub version: u8,
    /// Any rule carries AdGuard domain items (not decompilable by the core).
    pub has_adguard: bool,
    /// Row count the viewer will show. Regular sets match source-set
    /// semantics (`crate::domain::remote_rule_display_count`); AdGuard
    /// sets count their filter lines regardless of logical nesting,
    /// which is compiler structure rather than user-visible rules.
    pub display_count: u32,
    /// Source-style rules rebuilt from the binary items. `None` unless the
    /// caller asked for them; only AdGuard rule-sets need the reconstruction.
    pub rules: Option<Vec<serde_json::Value>>,
}

impl ParsedSrs {
    /// Source-style JSON for the read-only viewer. AdGuard rule-sets are
    /// flattened so every filter line becomes its own expandable row.
    pub(crate) fn display_source(&self) -> serde_json::Value {
        let Some(rules) = &self.rules else {
            return serde_json::json!({ "version": self.version, "rules": [] });
        };
        if !self.has_adguard {
            return serde_json::json!({ "version": self.version, "rules": rules });
        }
        let mut lines = Vec::new();
        for rule in rules {
            collect_adguard_lines(rule, &mut lines);
        }
        serde_json::json!({ "version": self.version, "rules": [{ "ad_guard_domain": lines }] })
    }
}

fn collect_adguard_lines(rule: &serde_json::Value, lines: &mut Vec<String>) {
    let Some(object) = rule.as_object() else {
        return;
    };
    if let Some(values) = object
        .get("ad_guard_domain")
        .and_then(serde_json::Value::as_array)
    {
        lines.extend(
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string)),
        );
    }
    if let Some(subrules) = object.get("rules").and_then(serde_json::Value::as_array) {
        for subrule in subrules {
            collect_adguard_lines(subrule, lines);
        }
    }
}

pub(crate) fn parse(bytes: &[u8]) -> Result<ParsedSrs, String> {
    parse_inner(bytes, false)
}

/// Parse and additionally rebuild display rules for every item. Only used
/// for AdGuard rule-sets, where this is the only way to list entries.
pub(crate) fn parse_with_rules(bytes: &[u8]) -> Result<ParsedSrs, String> {
    parse_inner(bytes, true)
}

fn parse_inner(bytes: &[u8], collect_rules: bool) -> Result<ParsedSrs, String> {
    if bytes.len() < 4 || &bytes[..3] != MAGIC {
        return Err("缺少 SRS 文件头".into());
    }
    let version = bytes[3];
    if version == 0 || version > MAX_VERSION {
        return Err(format!("不支持的 SRS 版本: {version}"));
    }
    let decompressed = inflate(&bytes[4..])?;
    let mut walker = Walker {
        cursor: Cursor::new(&decompressed),
        has_adguard: false,
        collect_rules,
        rules: Vec::new(),
        adguard_rows: 0,
    };
    let count = walker.cursor.uvarint()?;
    if count == 0 {
        return Err("SRS rules 为空".into());
    }
    // Every rule costs at least 3 bytes (type + final + invert).
    if count as usize > walker.cursor.remaining() / 3 + 1 {
        return Err("SRS 规则数量超出数据长度".into());
    }
    let mut display_rows = 0usize;
    for _ in 0..count {
        let rows = walker.read_rule(0)?;
        display_rows = display_rows
            .checked_add(rows)
            .ok_or_else(|| "SRS 条目数量过多".to_string())?;
    }
    if walker.cursor.remaining() != 0 {
        return Err("SRS 数据末尾存在多余字节".into());
    }
    let rows = if walker.has_adguard {
        walker.adguard_rows.max(1)
    } else {
        display_rows
    };
    let display_count =
        u32::try_from(rows).map_err(|_| "SRS 条目数量过多".to_string())?;
    Ok(ParsedSrs {
        version,
        has_adguard: walker.has_adguard,
        display_count,
        rules: collect_rules.then_some(walker.rules),
    })
}

fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut reader = flate2::read::ZlibDecoder::new(data).take(MAX_DECOMPRESSED as u64 + 1);
    reader
        .read_to_end(&mut out)
        .map_err(|error| format!("SRS zlib 数据损坏: {error}"))?;
    if out.len() > MAX_DECOMPRESSED {
        return Err("SRS 解压后数据过大".into());
    }
    Ok(out)
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    fn u8(&mut self) -> Result<u8, String> {
        let byte = *self
            .data
            .get(self.pos)
            .ok_or_else(|| "SRS 数据已截断".to_string())?;
        self.pos += 1;
        Ok(byte)
    }

    fn uvarint(&mut self) -> Result<u64, String> {
        let mut value: u64 = 0;
        let mut shift = 0u32;
        loop {
            let byte = self.u8()?;
            let bits = u64::from(byte & 0x7F);
            if shift >= 64 || (bits << shift) >> shift != bits {
                return Err("SRS 数据损坏: uvarint 溢出".into());
            }
            value |= bits << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
        }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| "SRS 数据损坏: 长度溢出".to_string())?;
        if end > self.data.len() {
            return Err("SRS 数据已截断".into());
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    /// Count-guarded list length: every entry consumes at least `unit` bytes.
    fn list_len(&mut self, unit: usize) -> Result<usize, String> {
        let count = self.uvarint()? as usize;
        if count > self.remaining() / unit + 1 {
            return Err("SRS 数据损坏: 列表长度超出数据范围".into());
        }
        Ok(count)
    }

    fn u16s(&mut self) -> Result<Vec<u16>, String> {
        let count = self.list_len(2)?;
        let bytes = self.take(count * 2)?;
        Ok(bytes
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect())
    }

    fn byte_slice(&mut self) -> Result<&'a [u8], String> {
        let count = self.list_len(1)?;
        self.take(count)
    }

    fn u64s(&mut self) -> Result<Vec<u64>, String> {
        let count = self.list_len(8)?;
        let bytes = self.take(count * 8)?;
        Ok(bytes
            .chunks_exact(8)
            .map(|word| u64::from_be_bytes(word.try_into().unwrap()))
            .collect())
    }

    fn strings(&mut self) -> Result<Vec<String>, String> {
        let count = self.list_len(1)?;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let len = self.uvarint()? as usize;
            out.push(String::from_utf8_lossy(self.take(len)?).into_owned());
        }
        Ok(out)
    }
}

struct Walker<'a> {
    cursor: Cursor<'a>,
    has_adguard: bool,
    collect_rules: bool,
    rules: Vec<serde_json::Value>,
    /// AdGuard filter lines across the whole set, nesting-independent.
    adguard_rows: usize,
}

impl<'a> Walker<'a> {
    /// Rows this rule contributes to the viewer; subrules of a logical
    /// rule are flattened into it, so the logical rule counts as one row.
    fn read_rule(&mut self, depth: u8) -> Result<usize, String> {
        if depth > MAX_LOGICAL_DEPTH {
            return Err("SRS 数据损坏: 逻辑规则嵌套过深".into());
        }
        match self.cursor.u8()? {
            RULE_TYPE_DEFAULT => {
                let (rule, rows) = self.read_default_rule()?;
                if self.collect_rules {
                    self.rules.push(rule);
                }
                Ok(rows)
            }
            RULE_TYPE_LOGICAL => {
                let mode = self.cursor.u8()?;
                if mode > 1 {
                    return Err(format!("未知的逻辑规则模式: {mode}"));
                }
                let count = self.cursor.uvarint()?;
                if count == 0 {
                    return Err("SRS 数据损坏: 逻辑规则无子规则".into());
                }
                if count as usize > self.cursor.remaining() / 3 + 1 {
                    return Err("SRS 数据损坏: 子规则数量超出数据长度".into());
                }
                let mut subrules = self.collect_rules.then(Vec::new);
                for _ in 0..count {
                    self.read_rule(depth + 1)?;
                    if let Some(rules) = subrules.as_mut() {
                        rules.push(self.rules.pop().expect("subrule was just pushed"));
                    }
                }
                let invert = self.cursor.u8()? != 0;
                if self.collect_rules {
                    let mut rule = serde_json::json!({
                        "type": "logical",
                        "mode": if mode == 0 { "and" } else { "or" },
                        "rules": subrules.unwrap_or_default(),
                    });
                    if invert {
                        rule["invert"] = serde_json::Value::Bool(true);
                    }
                    self.rules.push(rule);
                }
                Ok(1)
            }
            unknown => Err(format!("未知的规则类型: {unknown}")),
        }
    }

    /// Returns the reconstructed rule (when collecting) plus its display
    /// row count, matching `remote_rule_display_count` semantics: a rule
    /// containing object-valued fields stays one row.
    fn read_default_rule(&mut self) -> Result<(serde_json::Value, usize), String> {
        let mut rule = serde_json::Map::new();
        let mut rows = 0usize;
        let mut complex = false;
        loop {
            let item_type = self.cursor.u8()?;
            if item_type == ITEM_FINAL {
                let invert = self.cursor.u8()? != 0;
                if invert {
                    rule.insert("invert".into(), serde_json::Value::Bool(true));
                }
                break;
            }
            let (count, item_complex) = self.read_item(item_type, &mut rule)?;
            rows += count;
            complex |= item_complex;
        }
        if complex {
            rows = 1;
        }
        Ok((serde_json::Value::Object(rule), rows.max(1)))
    }

    fn read_item(
        &mut self,
        item_type: u8,
        rule: &mut serde_json::Map<String, serde_json::Value>,
    ) -> Result<(usize, bool), String> {
        Ok(match item_type {
            ITEM_QUERY_TYPE => {
                let values = self.cursor.u16s()?;
                let count = values.len();
                rule.insert("query_type".into(), serde_json::json!(values));
                (count, false)
            }
            ITEM_NETWORK => {
                let values = self.cursor.strings()?;
                let count = values.len();
                rule.insert("network".into(), serde_json::json!(values));
                (count, false)
            }
            ITEM_DOMAIN => {
                let set = SuccinctSet::read(&mut self.cursor)?;
                let count = set.leaf_count();
                if self.collect_rules {
                    let (domains, suffixes) = set.domain_dump();
                    rule.insert("domain".into(), serde_json::json!(domains));
                    rule.insert("domain_suffix".into(), serde_json::json!(suffixes));
                }
                (count, false)
            }
            ITEM_DOMAIN_KEYWORD => {
                let values = self.cursor.strings()?;
                let count = values.len();
                rule.insert("domain_keyword".into(), serde_json::json!(values));
                (count, false)
            }
            ITEM_DOMAIN_REGEX => {
                let values = self.cursor.strings()?;
                let count = values.len();
                rule.insert("domain_regex".into(), serde_json::json!(values));
                (count, false)
            }
            ITEM_SOURCE_IP_CIDR | ITEM_IP_CIDR => {
                let ranges = self.read_ip_set()?;
                let count = ranges.len();
                let field = if item_type == ITEM_IP_CIDR {
                    "ip_cidr"
                } else {
                    "source_ip_cidr"
                };
                rule.insert(field.into(), serde_json::json!(ranges));
                (count, false)
            }
            ITEM_SOURCE_PORT | ITEM_PORT => {
                let values = self.cursor.u16s()?;
                let count = values.len();
                let field = if item_type == ITEM_PORT {
                    "port"
                } else {
                    "source_port"
                };
                rule.insert(field.into(), serde_json::json!(values));
                (count, false)
            }
            ITEM_SOURCE_PORT_RANGE => {
                let values = self.cursor.strings()?;
                let count = values.len();
                rule.insert("source_port_range".into(), serde_json::json!(values));
                (count, false)
            }
            ITEM_PORT_RANGE => {
                let values = self.cursor.strings()?;
                let count = values.len();
                rule.insert("port_range".into(), serde_json::json!(values));
                (count, false)
            }
            ITEM_PROCESS_NAME => {
                let values = self.cursor.strings()?;
                let count = values.len();
                rule.insert("process_name".into(), serde_json::json!(values));
                (count, false)
            }
            ITEM_PROCESS_PATH => {
                let values = self.cursor.strings()?;
                let count = values.len();
                rule.insert("process_path".into(), serde_json::json!(values));
                (count, false)
            }
            ITEM_PACKAGE_NAME => {
                let values = self.cursor.strings()?;
                let count = values.len();
                rule.insert("package_name".into(), serde_json::json!(values));
                (count, false)
            }
            ITEM_WIFI_SSID => {
                let values = self.cursor.strings()?;
                let count = values.len();
                rule.insert("wifi_ssid".into(), serde_json::json!(values));
                (count, false)
            }
            ITEM_WIFI_BSSID => {
                let values = self.cursor.strings()?;
                let count = values.len();
                rule.insert("wifi_bssid".into(), serde_json::json!(values));
                (count, false)
            }
            ITEM_ADGUARD_DOMAIN => {
                self.has_adguard = true;
                let set = SuccinctSet::read(&mut self.cursor)?;
                let count = set.leaf_count();
                self.adguard_rows += count;
                if self.collect_rules {
                    rule.insert(
                        "ad_guard_domain".into(),
                        serde_json::json!(set.adguard_dump()),
                    );
                }
                (count, false)
            }
            ITEM_PROCESS_PATH_REGEX => {
                let values = self.cursor.strings()?;
                let count = values.len();
                rule.insert("process_path_regex".into(), serde_json::json!(values));
                (count, false)
            }
            ITEM_NETWORK_TYPE => {
                let values = self.cursor.byte_slice()?.to_vec();
                let count = values.len();
                rule.insert("network_type".into(), serde_json::json!(values));
                (count, false)
            }
            ITEM_NETWORK_IS_EXPENSIVE => {
                rule.insert(
                    "network_is_expensive".into(),
                    serde_json::json!([true]),
                );
                (1, false)
            }
            ITEM_NETWORK_IS_CONSTRAINED => {
                rule.insert(
                    "network_is_constrained".into(),
                    serde_json::json!([true]),
                );
                (1, false)
            }
            ITEM_NETWORK_INTERFACE_ADDRESS => {
                let count = self.cursor.uvarint()? as usize;
                if count > self.cursor.remaining() + 1 {
                    return Err("SRS 数据损坏: 地址映射超出数据范围".into());
                }
                let mut entries = self.collect_rules.then(Vec::new);
                let mut total = 0usize;
                for _ in 0..count {
                    let key = self.cursor.u8()?;
                    let prefix_count = self.cursor.uvarint()? as usize;
                    if prefix_count > self.cursor.remaining() + 1 {
                        return Err("SRS 数据损坏: 地址数量超出数据范围".into());
                    }
                    let mut prefixes = self.collect_rules.then(Vec::new);
                    for _ in 0..prefix_count {
                        let prefix = self.read_prefix()?;
                        if let Some(list) = prefixes.as_mut() {
                            list.push(prefix);
                        }
                    }
                    total += prefix_count.max(1);
                    if let Some(entries) = entries.as_mut() {
                        entries.push(serde_json::json!({ format!("{key}"): prefixes.unwrap_or_default() }));
                    }
                }
                if let Some(entries) = entries {
                    rule.insert("network_interface_address".into(), serde_json::json!(entries));
                }
                (total, true)
            }
            ITEM_DEFAULT_INTERFACE_ADDRESS => {
                let count = self.cursor.uvarint()? as usize;
                if count > self.cursor.remaining() + 1 {
                    return Err("SRS 数据损坏: 地址数量超出数据范围".into());
                }
                let mut prefixes = self.collect_rules.then(Vec::new);
                for _ in 0..count {
                    let prefix = self.read_prefix()?;
                    if let Some(list) = prefixes.as_mut() {
                        list.push(prefix);
                    }
                }
                if let Some(prefixes) = prefixes {
                    rule.insert(
                        "default_interface_address".into(),
                        serde_json::json!(prefixes),
                    );
                }
                (count.max(1), false)
            }
            unknown => return Err(format!("未知的规则条目类型: {unknown}")),
        })
    }

    fn read_ip_set(&mut self) -> Result<Vec<String>, String> {
        if self.cursor.u8()? != 1 {
            return Err("SRS 数据损坏: 不支持的 IP 集版本".into());
        }
        let bytes = self.cursor.take(8)?;
        let count = u64::from_be_bytes(bytes.try_into().unwrap()) as usize;
        if count > self.cursor.remaining() / 4 + 1 {
            return Err("SRS 数据损坏: IP 段数量超出数据范围".into());
        }
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let from = self.read_address()?;
            let to = self.read_address()?;
            out.push(if from == to {
                from
            } else {
                format!("{from}-{to}")
            });
        }
        Ok(out)
    }

    fn read_address(&mut self) -> Result<String, String> {
        let len = self.cursor.uvarint()? as usize;
        let bytes = self.cursor.take(len)?;
        Ok(match bytes {
            [a, b, c, d] => format!("{a}.{b}.{c}.{d}"),
            other if other.len() == 16 => {
                let octets: [u8; 16] = other.try_into().unwrap();
                std::net::Ipv6Addr::from(octets).to_string()
            }
            other => hex::encode(other),
        })
    }

    fn read_prefix(&mut self) -> Result<String, String> {
        let address = self.read_address()?;
        let bits = self.cursor.u8()?;
        Ok(format!("{address}/{bits}"))
    }
}

/// A LOUDS succinct trie as stored by `sing/common/domain`. Only the
/// structural fields are serialized; rank/select indexes are rebuilt.
struct SuccinctSet {
    leaves: Vec<u64>,
    label_bitmap: Vec<u64>,
    labels: Vec<u8>,
    ones_count: usize,
}

impl SuccinctSet {
    fn read(cursor: &mut Cursor<'_>) -> Result<Self, String> {
        let version = cursor.u8()?;
        if version > 0 {
            return Err(format!("不支持的域名匹配器版本: {version}"));
        }
        let leaves = cursor.u64s()?;
        let label_bitmap = cursor.u64s()?;
        let labels = cursor.byte_slice()?.to_vec();
        // Same invariants sing-box enforces in readSuccinctSet.
        let ones_count: usize = label_bitmap.iter().map(|w| w.count_ones() as usize).sum();
        let last_one = label_bitmap
            .iter()
            .rposition(|&w| w != 0)
            .map(|index| index * 64 + (63 - label_bitmap[index].leading_zeros() as usize))
            .ok_or_else(|| "SRS 域名匹配器数据损坏".to_string())?;
        let zeros_count = last_one + 1 - ones_count;
        if ones_count != zeros_count + 1 || labels.len() != zeros_count {
            return Err("SRS 域名匹配器数据损坏".into());
        }
        Ok(Self {
            leaves,
            label_bitmap,
            labels,
            ones_count,
        })
    }

    fn leaf_count(&self) -> usize {
        (0..self.ones_count)
            .filter(|&node| self.bit(&self.leaves, node))
            .count()
    }

    fn bit(&self, words: &[u64], index: usize) -> bool {
        words
            .get(index >> 6)
            .is_some_and(|word| word >> (index & 63) & 1 == 1)
    }

    /// Enumerate all stored keys (mirrors `succinctSet.keys`).
    fn keys(&self) -> Vec<Vec<u8>> {
        let mut result = Vec::new();
        let mut current: Vec<u8> = Vec::new();
        // Per-word cumulative popcount + positions of all one bits make
        // rank/select O(1) during the walk.
        let mut ones_prefix = Vec::with_capacity(self.label_bitmap.len() + 1);
        let mut ones = 0usize;
        let mut one_positions = Vec::new();
        for (word, &bits) in self.label_bitmap.iter().enumerate() {
            ones_prefix.push(ones);
            ones += bits.count_ones() as usize;
            for bit in 0..64 {
                if bits >> bit & 1 == 1 {
                    one_positions.push(word * 64 + bit);
                }
            }
        }
        ones_prefix.push(ones);
        let rank_ones = |end: usize| -> usize {
            let word = end >> 6;
            let mut total = ones_prefix[word.min(ones_prefix.len() - 1)];
            let rem = end & 63;
            if rem > 0 {
                if let Some(&bits) = self.label_bitmap.get(word) {
                    total += (bits & ((1u64 << rem) - 1)).count_ones() as usize;
                }
            }
            total
        };
        let terminates = |index: usize| -> bool {
            // Beyond the bitmap every position counts as a terminator,
            // keeping the walk finite on malformed-but-accepted data.
            self.label_bitmap
                .get(index >> 6)
                .is_none_or(|bits| bits >> (index & 63) & 1 == 1)
        };

        if self.bit(&self.leaves, 0) {
            result.push(Vec::new());
        }
        let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
        while let Some(&(node, bm)) = stack.last() {
            if terminates(bm) {
                stack.pop();
                if let Some(&mut (_, ref mut parent_bm)) = stack.last_mut() {
                    current.pop();
                    *parent_bm += 1;
                }
                continue;
            }
            // With the structural invariants verified above the edge index
            // stays inside `labels`; guard anyway instead of panicking.
            let Some(&label) = self.labels.get(bm.checked_sub(node).unwrap_or(usize::MAX)) else {
                break;
            };
            current.push(label);
            let next_node = bm + 1 - rank_ones(bm + 1);
            let Some(&terminator) = one_positions.get(next_node.wrapping_sub(1)) else {
                break;
            };
            let next_bm = terminator + 1;
            if self.bit(&self.leaves, next_node) {
                result.push(current.clone());
            }
            stack.push((next_node, next_bm));
        }
        result
    }

    /// Split trie keys into exact domains and domain suffixes, mirroring
    /// `domain.Matcher.Dump` (prefix label marks exact, everything else
    /// — including the root label — is a suffix).
    fn domain_dump(&self) -> (Vec<String>, Vec<String>) {
        let mut domains = Vec::new();
        let mut suffixes = Vec::new();
        for key in self.keys() {
            if key.is_empty() {
                continue;
            }
            let mut domain = reverse_runes(&key);
            match domain.as_bytes()[0] {
                LABEL_PREFIX => {
                    domain.remove(0);
                    domains.push(domain);
                }
                LABEL_ROOT => {
                    domain.remove(0);
                    suffixes.push(domain);
                }
                _ => suffixes.push(domain),
            }
        }
        (domains, suffixes)
    }

    /// Rebuild original AdGuard filter lines, mirroring
    /// `domain.AdGuardMatcher.Dump`.
    fn adguard_dump(&self) -> Vec<String> {
        self.keys()
            .into_iter()
            .filter_map(|key| {
                if key.is_empty() {
                    return None;
                }
                let mut line = reverse_runes(&key);
                let (is_suffix, has_start) = match line.as_bytes()[0] {
                    LABEL_PREFIX => {
                        line.remove(0);
                        (false, false)
                    }
                    LABEL_ROOT => {
                        line.remove(0);
                        (true, false)
                    }
                    _ => (false, true),
                };
                let has_end = !line.ends_with(LABEL_SUFFIX);
                if !has_end {
                    line.pop();
                }
                let mut line = if is_suffix {
                    format!("||{line}")
                } else if has_start {
                    format!("|{line}")
                } else {
                    line
                };
                if has_end {
                    line.push('^');
                }
                Some(line)
            })
            .collect()
    }
}

/// Go's reverseDomain reverses by Unicode scalar, not by byte.
fn reverse_runes(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).chars().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zlib(data: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, data).unwrap();
        encoder.finish().unwrap()
    }

    fn put_uvarint(out: &mut Vec<u8>, mut value: u64) {
        loop {
            let mut byte = (value & 0x7F) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                return;
            }
        }
    }

    fn put_u64s(out: &mut Vec<u8>, words: &[u64]) {
        put_uvarint(out, words.len() as u64);
        for word in words {
            out.extend_from_slice(&word.to_be_bytes());
        }
    }

    fn set_bit(words: &mut Vec<u64>, index: usize) {
        if words.len() <= index >> 6 {
            words.resize(index / 64 + 1, 0);
        }
        words[index / 64] |= 1u64 << (index % 64);
    }

    /// Encode a LOUDS trie exactly like sing's `newSuccinctSet` (BFS over
    /// sorted keys), so the reader is tested against the real layout.
    fn encode_succinct(keys: &[Vec<u8>]) -> Vec<u8> {
        let mut leaves: Vec<u64> = Vec::new();
        let mut label_bitmap: Vec<u64> = Vec::new();
        let mut labels: Vec<u8> = Vec::new();
        let mut queue: Vec<(usize, usize, usize)> = vec![(0, keys.len(), 0)];
        let mut l_idx = 0usize;
        let mut node = 0usize;
        while node < queue.len() {
            let (mut start, end, col) = queue[node];
            node += 1;
            if keys[start].len() == col {
                set_bit(&mut leaves, node - 1);
                start += 1;
            }
            let mut j = start;
            while j < end {
                let from = j;
                while j < end && keys[j][col] == keys[from][col] {
                    j += 1;
                }
                queue.push((from, j, col + 1));
                labels.push(keys[from][col]);
                l_idx += 1;
            }
            set_bit(&mut label_bitmap, l_idx);
            l_idx += 1;
        }
        let mut out = vec![0u8];
        put_u64s(&mut out, &leaves);
        put_u64s(&mut out, &label_bitmap);
        put_uvarint(&mut out, labels.len() as u64);
        out.extend_from_slice(&labels);
        out
    }

    fn reverse_domain(line: &str) -> Vec<u8> {
        line.chars().rev().collect::<String>().into_bytes()
    }

    /// Mirror `NewAdGuardMatcher`'s line → trie-key transform.
    fn adguard_key(line: &str) -> Vec<u8> {
        let mut key = line.to_string();
        let mut is_suffix = false;
        let mut has_start = false;
        if let Some(rest) = key.strip_prefix("||") {
            key = rest.to_string();
            is_suffix = true;
        } else if let Some(rest) = key.strip_prefix('|') {
            key = rest.to_string();
            has_start = true;
        }
        let mut has_end = false;
        if let Some(rest) = key.strip_suffix('^') {
            key = rest.to_string();
            has_end = true;
        }
        if is_suffix {
            key = format!("\n{key}");
        } else if !has_start {
            key = format!("\r{key}");
        }
        if !has_end {
            key = format!("{}\u{8}", key.trim_end_matches('.'));
        }
        reverse_domain(&key)
    }

    /// Wrap serialized rules into a full `.srs` file body.
    fn srs_file(rules_blob: &[u8]) -> Vec<u8> {
        let mut file = Vec::new();
        file.extend_from_slice(b"SRS");
        file.push(2);
        file.extend_from_slice(&zlib(rules_blob));
        file
    }

    fn adguard_srs(lines: &[&str]) -> Vec<u8> {
        let mut keys: Vec<Vec<u8>> = lines.iter().map(|line| adguard_key(line)).collect();
        keys.sort();
        keys.dedup();
        let mut blob = Vec::new();
        put_uvarint(&mut blob, 1); // one rule
        blob.push(RULE_TYPE_DEFAULT);
        blob.push(ITEM_ADGUARD_DOMAIN);
        blob.extend_from_slice(&encode_succinct(&keys));
        blob.push(ITEM_FINAL);
        blob.push(0); // invert
        srs_file(&blob)
    }

    #[test]
    fn parses_adguard_ruleset_and_round_trips_lines() {
        let lines = ["||example.com^", "ads.example.org", "|exact.net^", "||wild.card.me"];
        let parsed = parse_with_rules(&adguard_srs(&lines)).expect("valid adguard srs");
        assert!(parsed.has_adguard);
        assert_eq!(parsed.display_count, lines.len() as u32);
        let rules = parsed.rules.expect("rules collected");
        let mut dumped: Vec<String> = rules[0]["ad_guard_domain"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect();
        dumped.sort();
        let mut expected: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
        expected.sort();
        assert_eq!(dumped, expected);
    }

    #[test]
    fn parses_regular_binary_ruleset_without_adguard() {
        let keys = vec![reverse_domain("\nexample.com")];
        let mut blob = Vec::new();
        put_uvarint(&mut blob, 1);
        blob.push(RULE_TYPE_DEFAULT);
        blob.push(ITEM_DOMAIN);
        blob.extend_from_slice(&encode_succinct(&keys));
        blob.push(ITEM_FINAL);
        blob.push(0);
        let parsed = parse_with_rules(&srs_file(&blob)).expect("valid srs");
        assert!(!parsed.has_adguard);
        assert_eq!(parsed.display_count, 1);
        let rules = parsed.rules.unwrap();
        assert_eq!(rules[0]["domain_suffix"][0], "example.com");
    }

    #[test]
    fn parses_logical_rules() {
        let keys = vec![reverse_domain("\rexact.example")];
        let mut blob = Vec::new();
        put_uvarint(&mut blob, 1);
        blob.push(RULE_TYPE_LOGICAL);
        blob.push(0); // and
        put_uvarint(&mut blob, 2); // two subrules
        blob.push(RULE_TYPE_DEFAULT);
        blob.push(ITEM_DOMAIN);
        blob.extend_from_slice(&encode_succinct(&keys));
        blob.push(ITEM_FINAL);
        blob.push(0);
        blob.push(RULE_TYPE_DEFAULT);
        blob.push(ITEM_PORT);
        put_uvarint(&mut blob, 2);
        blob.extend_from_slice(&80u16.to_be_bytes());
        blob.extend_from_slice(&443u16.to_be_bytes());
        blob.push(ITEM_FINAL);
        blob.push(1); // invert
        blob.push(1); // logical invert
        let parsed = parse_with_rules(&srs_file(&blob)).expect("valid srs");
        assert!(!parsed.has_adguard);
        assert_eq!(parsed.display_count, 1, "logical rules stay one row");
        let rule = &parsed.rules.unwrap()[0];
        assert_eq!(rule["type"], "logical");
        assert_eq!(rule["mode"], "and");
        assert_eq!(rule["rules"][0]["domain"][0], "exact.example");
        assert_eq!(rule["rules"][1]["port"], serde_json::json!([80, 443]));
        assert_eq!(rule["rules"][1]["invert"], true);
    }

    #[test]
    fn counts_rows_like_the_source_display_counter() {
        // Walk-side counting must agree with remote_rule_display_count on
        // the reconstructed rules (used by both refresh and viewer paths).
        let lines = ["||a.example^", "||b.example^", "c.example"];
        let parsed = parse_with_rules(&adguard_srs(&lines)).unwrap();
        let rules = parsed.rules.unwrap();
        let counted: usize = rules
            .iter()
            .map(crate::domain::remote_rule_display_count)
            .sum();
        assert_eq!(counted, parsed.display_count as usize);
    }

    #[test]
    fn accepts_the_known_good_vector_from_remote_rule_auto() {
        const SRS: &[u8] = &[
            0x53, 0x52, 0x53, 0x02, 0x78, 0xda, 0x62, 0x64, 0x60, 0x62, 0x60, 0x64, 0x00, 0x03,
            0x01, 0x08, 0x83, 0x71, 0xd5, 0xaa, 0x55, 0x3c, 0xb9, 0xf9, 0xc9, 0x7a, 0xa9, 0x39,
            0x05, 0xb9, 0x89, 0x15, 0xa9, 0x5c, 0xff, 0x19, 0x00, 0x01, 0x00, 0x00, 0xff, 0xff,
            0x4d, 0xcc, 0x07, 0x83,
        ];
        let parsed = parse(SRS).expect("known-good vector parses");
        assert!(!parsed.has_adguard);
        assert_eq!(parsed.display_count, 1);
    }

    #[test]
    fn rejects_broken_files() {
        let good = adguard_srs(&["||example.com^"]);

        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        assert!(parse(&bad_magic).is_err());

        let mut bad_version = good.clone();
        bad_version[3] = 9;
        assert!(parse(&bad_version).is_err());

        assert!(parse(&good[..good.len() - 5]).is_err(), "truncated");

        // Bytes after the zlib stream are ignored, matching sing-box's own
        // reader (the kernel stops reading at the end of the stream).

        let empty_rules = srs_file(&[0]);
        assert!(parse(&empty_rules).is_err(), "zero rules");

        // A trie whose bitmap violates the ones/zeros/labels invariants.
        let mut blob = Vec::new();
        put_uvarint(&mut blob, 1);
        blob.push(RULE_TYPE_DEFAULT);
        blob.push(ITEM_ADGUARD_DOMAIN);
        blob.push(0); // matcher version
        put_u64s(&mut blob, &[1]);
        put_u64s(&mut blob, &[0b1]); // ones=1, zeros=0 → labels must be empty…
        put_uvarint(&mut blob, 1); // …but one extra label breaks it
        blob.push(b'x');
        blob.push(ITEM_FINAL);
        blob.push(0);
        assert!(parse(&srs_file(&blob)).is_err(), "malformed trie");
    }

    #[test]
    fn round_trips_lines_from_a_real_adguard_set() {
        // Two lines from the real megamori.srs, encoded through the same
        // transform the upstream Go encoder applies.
        let lines = ["||remembering.ca^", "||amendes-justiice-gov.buzz^"];
        let parsed = parse_with_rules(&adguard_srs(&lines)).expect("valid");
        let mut dumped: Vec<String> = parsed.rules.unwrap()[0]["ad_guard_domain"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect();
        dumped.sort();
        let mut expected: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
        expected.sort();
        assert_eq!(dumped, expected);
    }

    /// Manual check against a real-world file:
    /// `SRS_FILE=megamori.srs cargo test --lib srs:: -- --ignored --nocapture`
    #[test]
    #[ignore = "point SRS_FILE at a local .srs to run"]
    fn parses_an_external_srs_file() {
        let path = std::env::var("SRS_FILE").expect("SRS_FILE not set");
        let bytes = std::fs::read(&path).unwrap();
        let parsed = parse_with_rules(&bytes).expect("parses");
        let source = parsed.display_source();
        let sample: Vec<String> = source["rules"][0]
            .get("ad_guard_domain")
            .and_then(|value| value.as_array())
            .map(|lines| {
                lines
                    .iter()
                    .take(5)
                    .map(|line| line.as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default();
        eprintln!(
            "version={} has_adguard={} display_count={} top_rules={} sample={:?}",
            parsed.version,
            parsed.has_adguard,
            parsed.display_count,
            parsed.rules.expect("rules collected").len(),
            sample
        );
        assert_eq!(
            sample.is_empty(),
            !parsed.has_adguard,
            "adguard sets expose their lines through display_source"
        );
    }
}
