/// Version information for Shipwright
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const AGENT_VERSION: &str = "0.1.3";
pub const CLI_VERSION: &str = "0.1.3";

/// Get full version string
pub fn get_version_string() -> String {
    format!("Shipwright v{}", VERSION)
}

/// Get detailed version info
pub fn get_detailed_version() -> String {
    format!(
        "Shipwright v{}\nAgent: v{}\nCLI: v{}",
        VERSION, AGENT_VERSION, CLI_VERSION
    )
}
