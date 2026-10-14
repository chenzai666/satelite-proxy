//! Official Sudoku protocol options as accepted by Mihomo.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct SudokuHttpMask {
    #[serde(default)]
    pub disable: bool,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub tls: bool,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub path_root: String,
    #[serde(default)]
    pub multiplex: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub struct SudokuSettings {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aead_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding_min: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding_max: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_table: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_tables: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiplex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_pure_downlink: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub httpmask: Option<SudokuHttpMask>,
}

impl SudokuSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.key.trim().is_empty() {
            return Err("sudoku: missing key".into());
        }
        if self
            .aead_method
            .as_deref()
            .is_some_and(|s| !["none", "aes-128-gcm", "chacha20-poly1305"].contains(&s))
        {
            return Err("sudoku: invalid aead-method".into());
        }
        if self.padding_min.is_some_and(|v| v > 100)
            || self.padding_max.is_some_and(|v| v > 100)
            || self
                .padding_min
                .zip(self.padding_max)
                .is_some_and(|(min, max)| min > max)
        {
            return Err("sudoku: padding must satisfy 0 <= min <= max <= 100".into());
        }
        if self.table_type.as_deref().is_some_and(|s| {
            ![
                "prefer_ascii",
                "prefer_entropy",
                "up_ascii_down_entropy",
                "up_entropy_down_ascii",
            ]
            .contains(&s)
        }) {
            return Err("sudoku: invalid table-type".into());
        }
        for table in self.custom_table.iter().chain(self.custom_tables.iter()) {
            if table.len() != 8
                || table.bytes().filter(|b| *b == b'x').count() != 2
                || table.bytes().filter(|b| *b == b'p').count() != 2
                || table.bytes().filter(|b| *b == b'v').count() != 4
            {
                return Err("sudoku: custom table requires 2 x, 2 p and 4 v".into());
            }
        }
        let valid_mux = |s: &str| ["", "off", "auto", "on"].contains(&s);
        if self.multiplex.as_deref().is_some_and(|s| !valid_mux(s)) {
            return Err("sudoku: invalid multiplex".into());
        }
        if let Some(mask) = &self.httpmask {
            if !["", "legacy", "stream", "poll", "auto", "ws"].contains(&mask.mode.as_str())
                || !valid_mux(&mask.multiplex)
            {
                return Err("sudoku: invalid httpmask mode or multiplex".into());
            }
        }
        Ok(())
    }
}
