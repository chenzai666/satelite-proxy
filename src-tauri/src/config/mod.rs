mod builder;
mod dns_build;
mod rule_files;
mod write;

pub use builder::{
    build_singbox_config, generate_api_secret, outbound_tag, smart_pool_nodes, BuildOptions,
};
pub use rule_files::{
    dump_all_rule_sets, dump_rule_set_files, remove_rule_set_files, rules_export_dir,
};
pub use write::{active_config_path, write_active_config};
