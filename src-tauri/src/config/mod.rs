mod builder;
mod dns_build;
mod write;

pub use builder::{
    build_singbox_config, generate_api_secret, outbound_tag, smart_pool_nodes, BuildOptions,
};
pub use write::{active_config_path, write_active_config};
