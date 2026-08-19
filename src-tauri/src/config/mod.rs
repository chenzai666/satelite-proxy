mod builder;
mod custom;
mod dns_build;
mod dns_files;
mod punycode;
mod rule_files;
mod write;

pub use builder::{
    build_singbox_config, generate_api_secret, outbound_tag, rule_set_is_empty_for_config,
    smart_pool_nodes, BuildOptions,
};
pub use custom::inspect_singbox_config;
pub use dns_build::lookup_hosts;
pub use dns_files::dump_dns_rules_file;
pub use rule_files::{dump_rule_set_files, remove_rule_set_files};
pub use write::{
    active_config_path, remove_custom_config, write_active_config, write_custom_config,
};
