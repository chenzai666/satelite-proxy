mod builder;
mod custom;
mod dns_build;
mod dns_files;
mod mihomo;
mod punycode;
mod rule_files;
mod write;
mod xray;

pub use builder::{
    apply_udp_node_compatibility, build_singbox_config_with_connection_policy, generate_api_secret,
    outbound_tag, rule_set_is_empty_for_config, smart_pool_nodes, subscription_proxy_port,
    BuildOptions, SUBSCRIPTION_PROXY_USERNAME,
};
pub use custom::inspect_singbox_config;
pub use dns_build::lookup_hosts;
pub use dns_files::dump_dns_rules_file;
pub use mihomo::build_mihomo_config;
pub use rule_files::{dump_rule_set_files, remove_rule_set_files};
pub use write::{
    active_config_path, active_yaml_config_path, remove_custom_config, write_active_config,
    write_active_yaml_config, write_custom_config,
};
pub use xray::build_xray_config;
