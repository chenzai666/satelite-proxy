mod builder;
mod dns_build;
mod dns_files;
mod punycode;
mod rule_files;
mod write;

pub use builder::{
    build_singbox_config, generate_api_secret, outbound_tag, smart_pool_nodes, BuildOptions,
};
pub use dns_build::lookup_hosts;
pub use dns_files::dump_dns_rules_file;
pub use rule_files::{dump_rule_set_files, remove_rule_set_files};
pub use write::{active_config_path, write_active_config};
