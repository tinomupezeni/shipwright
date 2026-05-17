use anyhow::{Result, Context};
use std::path::Path;
use std::collections::{HashMap, HashSet};
use regex::Regex;
use tracing::{info, warn};

/// Represents a missing or invalid environment variable
#[derive(Debug, Clone)]
pub struct EnvVarIssue {
    pub var_name: String,
    pub issue_type: IssueType,
    pub found_in: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IssueType {
    Missing,
    Empty,
}

/// Result of environment variable validation
#[derive(Debug)]
pub struct ValidationReport {
    pub issues: Vec<EnvVarIssue>,
    pub total_vars_checked: usize,
    pub env_file_path: String,
}

impl ValidationReport {
    /// Check if validation passed (no issues)
    pub fn is_valid(&self) -> bool {
        self.issues.is_empty()
    }

    /// Generate a human-readable error message
    pub fn error_message(&self) -> String {
        if self.is_valid() {
            return String::new();
        }

        let mut msg = String::from("❌ Environment variable validation failed:\n\n");

        for issue in &self.issues {
            let issue_desc = match issue.issue_type {
                IssueType::Missing => "is not set",
                IssueType::Empty => "is empty",
            };

            msg.push_str(&format!("  • {} {}\n", issue.var_name, issue_desc));
            if !issue.found_in.is_empty() {
                msg.push_str(&format!("    Required by: {}\n", issue.found_in.join(", ")));
            }
        }

        msg.push_str(&format!("\n📝 Please update your .env file at: {}\n", self.env_file_path));
        msg.push_str("\n🔧 To fix this issue:\n");
        msg.push_str("  1. SSH to your VPS or update .env file remotely\n");
        msg.push_str("  2. Add the missing variables to your .env file\n");
        msg.push_str("  3. For sensitive values, use: shipwright secrets set <VAR_NAME>\n");
        msg.push_str("  4. Check .env.example for reference values\n");
        msg.push_str("\n🔄 After fixing:\n");
        msg.push_str("  • Run 'shipwright retry' to retry this deployment\n");
        msg.push_str("  • No need to push new code - retry uses existing code\n");

        msg
    }
}

/// Parse docker-compose.yml file and extract environment variable references
pub async fn extract_env_vars_from_compose(compose_file: &Path) -> Result<HashMap<String, Vec<String>>> {
    let content = tokio::fs::read_to_string(compose_file)
        .await
        .context("Failed to read docker-compose file")?;

    let mut var_usage: HashMap<String, Vec<String>> = HashMap::new();

    // Regex to match ${VAR_NAME} or $VAR_NAME patterns
    let var_pattern = Regex::new(r"\$\{([A-Z_][A-Z0-9_]*)\}|\$([A-Z_][A-Z0-9_]*)").unwrap();

    // Parse YAML to understand structure (basic parsing for service names)
    let yaml: serde_yaml::Value = serde_yaml::from_str(&content)
        .context("Failed to parse docker-compose YAML")?;

    // Extract service names
    let services = yaml.get("services")
        .and_then(|s| s.as_mapping())
        .context("No services found in docker-compose file")?;

    for (service_name, service_config) in services.iter() {
        let service_name = service_name.as_str().unwrap_or("unknown");
        let service_str = serde_yaml::to_string(service_config)
            .unwrap_or_default();

        // Find all variable references in this service
        for cap in var_pattern.captures_iter(&service_str) {
            let var_name = cap.get(1)
                .or_else(|| cap.get(2))
                .map(|m| m.as_str())
                .unwrap_or("");

            if !var_name.is_empty() {
                var_usage.entry(var_name.to_string())
                    .or_insert_with(Vec::new)
                    .push(service_name.to_string());
            }
        }
    }

    // Also check top-level environment and image references
    let compose_str = content.to_string();
    for cap in var_pattern.captures_iter(&compose_str) {
        let var_name = cap.get(1)
            .or_else(|| cap.get(2))
            .map(|m| m.as_str())
            .unwrap_or("");

        if !var_name.is_empty() && !var_usage.contains_key(var_name) {
            var_usage.insert(var_name.to_string(), vec!["docker-compose.yml".to_string()]);
        }
    }

    Ok(var_usage)
}

/// Read and parse .env file into a HashMap
pub async fn read_env_file(env_file: &Path) -> Result<HashMap<String, String>> {
    if !env_file.exists() {
        return Ok(HashMap::new());
    }

    let content = tokio::fs::read_to_string(env_file)
        .await
        .context("Failed to read .env file")?;

    let mut env_vars = HashMap::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Parse KEY=VALUE format
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim().to_string();
            let value = line[eq_pos + 1..].trim().to_string();

            // Remove quotes if present
            let value = value.trim_matches('"').trim_matches('\'').to_string();

            env_vars.insert(key, value);
        }
    }

    Ok(env_vars)
}

/// Validate that all required environment variables are set
pub async fn validate_env_vars(
    build_dir: &Path,
    compose_file: &str,
) -> Result<ValidationReport> {
    let compose_path = build_dir.join(compose_file);
    let env_path = compose_path.parent().unwrap_or(build_dir).join(".env");

    info!("🔍 Validating environment variables...");
    info!("  Compose file: {}", compose_path.display());
    info!("  Env file: {}", env_path.display());

    // Extract required variables from docker-compose
    let required_vars = extract_env_vars_from_compose(&compose_path).await?;

    if required_vars.is_empty() {
        info!("✓ No environment variables referenced in docker-compose file");
        return Ok(ValidationReport {
            issues: vec![],
            total_vars_checked: 0,
            env_file_path: env_path.display().to_string(),
        });
    }

    // Read current .env file
    let current_env = read_env_file(&env_path).await?;

    // Check for missing or empty variables
    let mut issues = Vec::new();
    let mut checked_vars = HashSet::new();

    for (var_name, found_in) in required_vars.iter() {
        checked_vars.insert(var_name.clone());

        match current_env.get(var_name) {
            None => {
                // Variable not in .env file
                issues.push(EnvVarIssue {
                    var_name: var_name.clone(),
                    issue_type: IssueType::Missing,
                    found_in: found_in.clone(),
                });
            }
            Some(value) if value.is_empty() => {
                // Variable exists but is empty
                issues.push(EnvVarIssue {
                    var_name: var_name.clone(),
                    issue_type: IssueType::Empty,
                    found_in: found_in.clone(),
                });
            }
            Some(_) => {
                // Variable is set and non-empty
            }
        }
    }

    let report = ValidationReport {
        issues,
        total_vars_checked: checked_vars.len(),
        env_file_path: env_path.display().to_string(),
    };

    if report.is_valid() {
        info!("✓ All {} environment variables are properly set", report.total_vars_checked);
    } else {
        warn!("⚠️  Found {} environment variable issue(s)", report.issues.len());
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_extract_env_vars() {
        let compose_content = r#"
version: '3.8'
services:
  app:
    image: ghcr.io/${GITHUB_REPO}/app:latest
    environment:
      - DATABASE_URL=${DATABASE_URL}
      - SECRET_KEY=$SECRET_KEY
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(compose_content.as_bytes()).unwrap();

        let vars = extract_env_vars_from_compose(temp_file.path()).await.unwrap();

        assert!(vars.contains_key("GITHUB_REPO"));
        assert!(vars.contains_key("DATABASE_URL"));
        assert!(vars.contains_key("SECRET_KEY"));
    }

    #[tokio::test]
    async fn test_read_env_file() {
        let env_content = r#"
# Comment
DATABASE_URL=postgres://localhost/db
SECRET_KEY="my-secret"
EMPTY_VAR=
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(env_content.as_bytes()).unwrap();

        let env_vars = read_env_file(temp_file.path()).await.unwrap();

        assert_eq!(env_vars.get("DATABASE_URL"), Some(&"postgres://localhost/db".to_string()));
        assert_eq!(env_vars.get("SECRET_KEY"), Some(&"my-secret".to_string()));
        assert_eq!(env_vars.get("EMPTY_VAR"), Some(&"".to_string()));
    }
}
