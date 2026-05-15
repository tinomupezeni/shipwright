pub mod detector;
pub mod adapters;

pub use detector::{InfrastructureInfo, detect_infrastructure};
pub use adapters::{ProxyAdapter, CaddyAdapter, NginxAdapter, TraefikAdapter};
