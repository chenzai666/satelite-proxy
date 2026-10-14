use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MihomoTemplate {
    pub url: String,
    pub ini: String,
    pub override_yaml: String,
}

impl Default for MihomoTemplate {
    fn default() -> Self {
        Self {
            url: "https://raw.githubusercontent.com/chenzai666/sub-ini/refs/heads/main/new_config.ini".into(),
            ini: include_str!("../../resources/mihomo/default.ini").into(),
            override_yaml: include_str!("../../resources/mihomo/override.yaml").into(),
        }
    }
}

impl MihomoTemplate {
    pub fn group_labels(&self) -> std::collections::BTreeMap<String, String> {
        let groups: Vec<Vec<&str>> = self
            .ini
            .lines()
            .filter_map(|line| line.trim().strip_prefix("custom_proxy_group="))
            .map(|line| line.split('`').collect())
            .filter(|fields: &Vec<&str>| fields.len() >= 2)
            .collect();
        let mut labels = std::collections::BTreeMap::new();
        if let Some(main) = groups
            .iter()
            .find(|g| {
                g[1] == "select"
                    && (g[0].eq_ignore_ascii_case("proxy") || g[0].contains("节点选择"))
            })
            .or_else(|| groups.iter().find(|g| g[1] == "select"))
        {
            labels.insert("proxy".into(), main[0].into());
        }
        if let Some(auto) = groups
            .iter()
            .find(|g| g[1] == "url-test" || g[0].contains("自动"))
        {
            labels.insert("auto".into(), auto[0].into());
        }
        labels
    }
}
