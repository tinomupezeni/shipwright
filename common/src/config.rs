use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub version: u32,
    pub project: ProjectConfig,
    pub build: BuildConfig,
    pub deploy: DeployConfig,
    pub infrastructure: Option<InfrastructureConfig>,
    pub notifications: Option<NotificationsConfig>,
    pub smoke_tests: Option<SmokeTestsConfig>,
}

/// Infrastructure configuration for existing VPS setups
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InfrastructureConfig {
    /// Deployment strategy: "standalone", "compose", or "auto"
    #[serde(default = "default_deploy_strategy")]
    pub strategy: String,

    /// Directory where projects are deployed (e.g., ~/apps or /opt/apps)
    pub deploy_dir: Option<String>,

    /// Proxy configuration (Caddy, Nginx, Traefik, or none)
    pub proxy: Option<ProxyConfig>,

    /// Shared resources (databases, redis, etc.)
    pub shared_resources: Option<SharedResourcesConfig>,

    /// Docker networks to join
    #[serde(default)]
    pub networks: Vec<String>,

    /// Whether to auto-detect existing infrastructure
    #[serde(default = "default_true")]
    pub auto_detect: bool,
}

fn default_deploy_strategy() -> String {
    "auto".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProxyConfig {
    /// Proxy type: "caddy", "nginx", "traefik", "none"
    #[serde(rename = "type")]
    pub proxy_type: String,

    /// Container name of the proxy (e.g., "caddy-proxy")
    pub container_name: Option<String>,

    /// Path to config file (for manual updates)
    pub config_path: Option<String>,

    /// Whether to auto-update proxy config
    #[serde(default = "default_true")]
    pub auto_update: bool,

    /// Reload command (e.g., "caddy reload")
    pub reload_command: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SharedResourcesConfig {
    /// Shared PostgreSQL configuration
    pub postgres: Option<SharedPostgresConfig>,

    /// Shared Redis configuration
    pub redis: Option<SharedRedisConfig>,

    /// Other shared services
    #[serde(default)]
    pub services: Vec<SharedServiceConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SharedPostgresConfig {
    /// Container name or host
    pub host: String,

    /// Port (default: 5432)
    #[serde(default = "default_postgres_port")]
    pub port: u16,

    /// Database name for this project
    pub database: String,

    /// Username
    pub user: String,

    /// Docker network where postgres is accessible
    pub network: Option<String>,
}

fn default_postgres_port() -> u16 {
    5432
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SharedRedisConfig {
    /// Container name or host
    pub host: String,

    /// Port (default: 6379)
    #[serde(default = "default_redis_port")]
    pub port: u16,

    /// Redis database number
    #[serde(default)]
    pub db: u8,

    /// Docker network where redis is accessible
    pub network: Option<String>,
}

fn default_redis_port() -> u16 {
    6379
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SharedServiceConfig {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub network: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectConfig {
    pub name: String,
    pub framework: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BuildConfig {
    /// Image name (optional - not needed for docker-compose deployments)
    pub image: Option<String>,

    pub cache: Option<Vec<String>>,

    #[serde(default)]
    pub steps: Vec<String>,

    /// Docker Compose file to use for building (if using compose strategy)
    pub compose_file: Option<String>,

    /// Services to build (if using compose with selective builds)
    pub services: Option<Vec<String>>,

    /// Environment variables for build/deployment
    /// These will be written to .env file if it doesn't exist
    pub environment: Option<std::collections::HashMap<String, String>>,
}

fn default_replicas() -> u32 {
    1
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeployConfig {
    #[serde(rename = "type")]
    pub deploy_type: String,
    pub registry: Option<RegistryConfig>,
    pub vps: Option<VpsConfig>,
    #[serde(default = "default_replicas")]
    pub replicas: u32,
    pub health: Option<HealthConfig>,
    pub resources: Option<ResourceConfig>,
    pub smoke_tests: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VpsConfig {
    pub host: String,
    pub user: String,
    pub ssh_key: String,
    pub domain: Option<String>,
    /// Service-specific domain and routing configuration
    #[serde(default)]
    pub services: Vec<ServiceConfig>,
    /// Email for ACME/Let's Encrypt certificates
    pub acme_email: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServiceConfig {
    /// Service name (must match docker-compose service name)
    pub name: String,
    /// Domain for this service (e.g., "app.example.com")
    pub domain: Option<String>,
    /// Port the service listens on inside the container
    #[serde(default = "default_port")]
    pub port: u16,
    /// Optional path prefix for path-based routing (e.g., "/api")
    pub path: Option<String>,
    /// Whether this service should be exposed via Caddy
    #[serde(default = "default_true")]
    pub expose: bool,
}

fn default_port() -> u16 {
    80
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RegistryConfig {
    pub url: String,
    pub auth: Option<RegistryAuthConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RegistryAuthConfig {
    pub username: String,
    pub token_file: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HealthConfig {
    pub http: Option<HttpHealthConfig>,
    pub dependencies: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HttpHealthConfig {
    pub path: String,
    pub expect: u16,
    pub timeout: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResourceConfig {
    pub memory: u32,
    pub cpu: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NotificationsConfig {
    pub slack: Option<SlackConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SlackConfig {
    pub webhook: String,
    pub on: Vec<String>,
}

/// Smoke tests configuration
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SmokeTestsConfig {
    /// Enable or disable smoke tests
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Fail deployment on critical test failures
    #[serde(default = "default_true")]
    pub fail_on_error: bool,

    /// Test categories to run
    #[serde(default = "default_test_categories")]
    pub categories: Vec<String>,

    /// Tests to disable
    #[serde(default)]
    pub disabled_tests: Vec<String>,

    /// Test-specific configuration
    #[serde(default)]
    pub test_config: std::collections::HashMap<String, TestConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TestConfig {
    pub enabled: Option<bool>,
    pub timeout: Option<String>,
    pub retries: Option<u32>,
}

fn default_test_categories() -> Vec<String> {
    vec![
        "pre_deployment".to_string(),
        "post_build".to_string(),
        "post_deployment".to_string(),
    ]
}
