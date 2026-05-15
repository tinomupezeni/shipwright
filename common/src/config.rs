use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub version: u32,
    pub project: ProjectConfig,
    pub build: BuildConfig,
    pub deploy: DeployConfig,
    pub notifications: Option<NotificationsConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectConfig {
    pub name: String,
    pub framework: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BuildConfig {
    pub image: String,
    pub cache: Option<Vec<String>>,
    pub steps: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeployConfig {
    #[serde(rename = "type")]
    pub deploy_type: String,
    pub registry: Option<RegistryConfig>,
    pub vps: Option<VpsConfig>,
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
