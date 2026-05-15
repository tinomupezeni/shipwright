use anyhow::Result;
use shipwright_common::config::VpsConfig;
use std::collections::HashMap;

/// Container status information
#[derive(Debug)]
pub struct ContainerStatus {
    pub name: String,
    pub status: String,
    pub health: String,
    pub exit_code: Option<i32>,
}

/// Diagnostic result with suggestions
#[derive(Debug)]
pub struct DiagnosticResult {
    pub container: String,
    pub issue: String,
    pub logs: String,
    pub suggestions: Vec<String>,
}

/// Get status of all containers for a project
pub fn get_container_statuses(vps: &VpsConfig, remote_dir: &str) -> Result<Vec<ContainerStatus>> {
    let cmd = format!(
        "cd {} && docker compose ps -a --format '{{{{.Name}}}}|{{{{.Status}}}}|{{{{.Health}}}}'",
        remote_dir
    );

    let output = super::deploy::execute_remote_command(vps, &cmd)?;
    let mut statuses = Vec::new();

    for line in output.lines() {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() >= 2 {
            statuses.push(ContainerStatus {
                name: parts[0].to_string(),
                status: parts.get(1).unwrap_or(&"unknown").to_string(),
                health: parts.get(2).unwrap_or(&"").to_string(),
                exit_code: extract_exit_code(parts.get(1).unwrap_or(&"")),
            });
        }
    }

    Ok(statuses)
}

/// Extract exit code from status string like "Exited (1)"
fn extract_exit_code(status: &str) -> Option<i32> {
    if status.contains("Exited") {
        let re = regex::Regex::new(r"Exited \((\d+)\)").ok()?;
        if let Some(cap) = re.captures(status) {
            return cap[1].parse().ok();
        }
    }
    None
}

/// Get logs from a specific container
pub fn get_container_logs(vps: &VpsConfig, container_name: &str, lines: u32) -> Result<String> {
    let cmd = format!("docker logs --tail {} {} 2>&1", lines, container_name);
    super::deploy::execute_remote_command(vps, &cmd)
}

/// Diagnose unhealthy or failed containers
pub fn diagnose_failures(vps: &VpsConfig, remote_dir: &str) -> Result<Vec<DiagnosticResult>> {
    let statuses = get_container_statuses(vps, remote_dir)?;
    let mut results = Vec::new();

    for status in statuses {
        let is_unhealthy = status.health.to_lowercase().contains("unhealthy");
        let is_exited = status.status.to_lowercase().contains("exited");
        let is_restarting = status.status.to_lowercase().contains("restarting");

        if is_unhealthy || is_exited || is_restarting {
            let logs = get_container_logs(vps, &status.name, 50).unwrap_or_default();
            let issue = if is_unhealthy {
                "Container is unhealthy (health check failing)".to_string()
            } else if is_restarting {
                "Container is in restart loop".to_string()
            } else {
                format!("Container exited with code {:?}", status.exit_code)
            };

            let suggestions = analyze_logs_for_suggestions(&status.name, &logs, &status);

            results.push(DiagnosticResult {
                container: status.name,
                issue,
                logs,
                suggestions,
            });
        }
    }

    Ok(results)
}

/// Analyze logs and provide contextual suggestions
fn analyze_logs_for_suggestions(container: &str, logs: &str, status: &ContainerStatus) -> Vec<String> {
    let mut suggestions = Vec::new();
    let logs_lower = logs.to_lowercase();

    // Password authentication failure - VERY COMMON issue
    if logs_lower.contains("password authentication failed") {
        suggestions.push("🔑 PASSWORD MISMATCH DETECTED!".to_string());
        suggestions.push("The database password in .env doesn't match the existing database.".to_string());
        suggestions.push("".to_string());
        suggestions.push("Fix Option 1 - Update password in database (keeps data):".to_string());
        if logs_lower.contains("user \"hbec\"") || container.contains("student") || container.contains("admin") {
            suggestions.push("  docker exec -it hbec-postgres psql -U postgres -c \"ALTER USER hbec WITH PASSWORD 'YOUR_POSTGRES_PASSWORD';\"".to_string());
        }
        if logs_lower.contains("user \"harness\"") || container.contains("harness") {
            suggestions.push("  docker exec -it hbec-harness-db psql -U postgres -c \"ALTER USER harness WITH PASSWORD 'YOUR_HARNESS_DB_PASSWORD';\"".to_string());
        }
        suggestions.push("  Then restart: docker compose restart".to_string());
        suggestions.push("".to_string());
        suggestions.push("Fix Option 2 - Reset database (LOSES ALL DATA):".to_string());
        suggestions.push("  docker compose down -v && docker compose up -d".to_string());
        return suggestions; // This is the most important issue, return early
    }

    // Database connection issues
    if logs_lower.contains("connection refused") || logs_lower.contains("could not connect") {
        if logs_lower.contains("postgres") || logs_lower.contains("5432") {
            suggestions.push("Database connection failed. Check that postgres container is healthy.".to_string());
            suggestions.push("Verify POSTGRES_PASSWORD matches between services.".to_string());
            suggestions.push("Run: docker logs hbec-postgres".to_string());
        }
        if logs_lower.contains("redis") || logs_lower.contains("6379") {
            suggestions.push("Redis connection failed. Check that redis container is running.".to_string());
            suggestions.push("Run: docker logs hbec-redis".to_string());
        }
    }

    // Migration issues
    if logs_lower.contains("migration") || logs_lower.contains("migrate") {
        if logs_lower.contains("error") || logs_lower.contains("failed") {
            suggestions.push("Database migration failed. Try running migrations manually:".to_string());
            suggestions.push(format!("  docker exec -it {} python manage.py migrate", container));
        }
    }

    // Permission issues
    if logs_lower.contains("permission denied") || logs_lower.contains("eacces") {
        suggestions.push("Permission error detected. Check file/directory permissions.".to_string());
        suggestions.push("For volume mounts, ensure the container user has access.".to_string());
    }

    // Module/import issues
    if logs_lower.contains("modulenotfounderror") || logs_lower.contains("importerror") {
        suggestions.push("Python module not found. The image may need rebuilding.".to_string());
        suggestions.push("Try: docker compose build --no-cache <service>".to_string());
    }

    // Memory issues
    if logs_lower.contains("out of memory") || logs_lower.contains("oom") || logs_lower.contains("killed") {
        suggestions.push("Container was killed, possibly due to memory limits.".to_string());
        suggestions.push("Increase memory limits in docker-compose.yml or VPS resources.".to_string());
    }

    // Health check specific
    if status.health.to_lowercase().contains("unhealthy") {
        if container.contains("backend") {
            suggestions.push("Backend health check failing. Common causes:".to_string());
            suggestions.push("  1. Application not started yet (check start_period in healthcheck)".to_string());
            suggestions.push("  2. Database not ready when app started".to_string());
            suggestions.push("  3. Missing environment variables".to_string());
            suggestions.push(format!("Check detailed logs: docker logs {} --tail 100", container));
        }
        if container.contains("frontend") {
            suggestions.push("Frontend health check failing. Nginx may not be serving content.".to_string());
            suggestions.push("Check if the build produced files in /usr/share/nginx/html".to_string());
        }
    }

    // Environment variable issues
    if logs_lower.contains("keyerror") || logs_lower.contains("environment variable") ||
       logs_lower.contains("not set") || logs_lower.contains("missing") {
        suggestions.push("Missing environment variable. Check .env file on VPS:".to_string());
        suggestions.push("  cat /home/administrator/hbec/.env".to_string());
    }

    // JWT/Auth issues
    if logs_lower.contains("jwt") || logs_lower.contains("token") || logs_lower.contains("secret") {
        suggestions.push("JWT/Authentication error. Verify JWT keys and secrets:".to_string());
        suggestions.push("  - Check JWT_SECRET in .env matches across services".to_string());
        suggestions.push("  - Verify jwt_private.pem and jwt_public.pem exist in docker/keys/".to_string());
    }

    // Network issues
    if logs_lower.contains("network") || logs_lower.contains("dns") || logs_lower.contains("resolve") {
        suggestions.push("Network/DNS issue. Check Docker network configuration:".to_string());
        suggestions.push("  docker network ls".to_string());
        suggestions.push("  docker network inspect hbec_hbec-network".to_string());
    }

    // Default suggestions if nothing specific found
    if suggestions.is_empty() {
        suggestions.push(format!("Check full logs: docker logs {} --tail 200", container));
        suggestions.push(format!("Try restarting: docker compose restart {}",
            container.replace("hbec-", "")));
        suggestions.push("Check container shell: docker exec -it <container> sh".to_string());
    }

    suggestions
}

/// Print diagnostic results in a user-friendly format
pub fn print_diagnostics(results: &[DiagnosticResult]) {
    if results.is_empty() {
        println!("\n✅ All containers are healthy!");
        return;
    }

    println!("\n🔍 Deployment Diagnostics");
    println!("========================\n");

    for (i, result) in results.iter().enumerate() {
        println!("❌ Issue {}: {}", i + 1, result.container);
        println!("   Problem: {}", result.issue);

        println!("\n   📋 Recent Logs:");
        println!("   ───────────────");
        for line in result.logs.lines().take(20) {
            println!("   {}", line);
        }
        if result.logs.lines().count() > 20 {
            println!("   ... (truncated, {} more lines)", result.logs.lines().count() - 20);
        }

        println!("\n   💡 Suggestions:");
        for suggestion in &result.suggestions {
            println!("   • {}", suggestion);
        }
        println!();
    }

    println!("═══════════════════════════════════════════════════════════════");
    println!("🛠️  Quick Commands (run on VPS via SSH):");
    println!("───────────────────────────────────────────────────────────────");
    println!("   View all logs:     docker compose logs -f");
    println!("   Restart service:   docker compose restart <service-name>");
    println!("   Rebuild & restart: docker compose up -d --build <service>");
    println!("   Check resources:   docker stats");
    println!("   Enter container:   docker exec -it <container-name> sh");
    println!("═══════════════════════════════════════════════════════════════\n");
}

/// Quick health summary
pub fn print_health_summary(vps: &VpsConfig, remote_dir: &str) -> Result<bool> {
    let statuses = get_container_statuses(vps, remote_dir)?;

    let healthy: Vec<_> = statuses.iter()
        .filter(|s| s.status.to_lowercase().contains("up") &&
                   !s.health.to_lowercase().contains("unhealthy"))
        .collect();

    let unhealthy: Vec<_> = statuses.iter()
        .filter(|s| s.health.to_lowercase().contains("unhealthy") ||
                   s.status.to_lowercase().contains("exited") ||
                   s.status.to_lowercase().contains("restarting"))
        .collect();

    println!("\n📊 Container Health Summary:");
    println!("   ✅ Healthy: {}", healthy.len());
    println!("   ❌ Unhealthy/Failed: {}", unhealthy.len());

    if !unhealthy.is_empty() {
        println!("\n   Failed containers:");
        for s in &unhealthy {
            println!("   • {} - {} {}", s.name, s.status, s.health);
        }
    }

    Ok(unhealthy.is_empty())
}

/// Common fixes that can be attempted automatically
pub fn attempt_common_fixes(vps: &VpsConfig, remote_dir: &str, results: &[DiagnosticResult]) -> Result<Vec<String>> {
    let mut fixes_applied = Vec::new();

    // Check for password authentication issues first
    let has_password_issue = results.iter().any(|r|
        r.logs.to_lowercase().contains("password authentication failed")
    );

    if has_password_issue {
        println!("\n   🔑 Password mismatch detected!");
        println!("   This requires manual intervention to preserve your data.");
        println!("\n   Please SSH into your VPS and run:");

        // Try to get passwords from .env on remote
        if let Ok(env_content) = super::deploy::execute_remote_command(vps, &format!("cat {}/.env", remote_dir)) {
            let postgres_pw = extract_env_value(&env_content, "POSTGRES_PASSWORD");
            let harness_pw = extract_env_value(&env_content, "HARNESS_DB_PASSWORD");

            if let Some(pw) = postgres_pw {
                println!("\n   # Fix hbec user password:");
                println!("   docker exec -it hbec-postgres psql -U postgres -c \"ALTER USER hbec WITH PASSWORD '{}';\"", pw);
            }
            if let Some(pw) = harness_pw {
                println!("\n   # Fix harness user password:");
                println!("   docker exec -it hbec-harness-db psql -U postgres -c \"ALTER USER harness WITH PASSWORD '{}';\"", pw);
            }

            println!("\n   # Then restart services:");
            println!("   cd {} && docker compose restart", remote_dir);
        }

        return Ok(fixes_applied);
    }

    for result in results {
        // If it's a simple restart issue, try restarting
        if result.issue.contains("unhealthy") && !result.logs.contains("error") {
            let service_name = result.container.replace("hbec-", "");
            println!("   Attempting restart of {}...", service_name);

            let cmd = format!("cd {} && docker compose restart {}", remote_dir, service_name);
            if super::deploy::execute_remote_command(vps, &cmd).is_ok() {
                fixes_applied.push(format!("Restarted {}", service_name));
            }
        }
    }

    Ok(fixes_applied)
}

/// Extract value from .env content
fn extract_env_value(content: &str, key: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with(key) && line.contains('=') {
            if let Some((_, value)) = line.split_once('=') {
                return Some(value.trim().trim_matches('"').trim_matches('\'').to_string());
            }
        }
    }
    None
}

/// Escape a password for use in SQL single-quoted string
fn escape_sql_password(password: &str) -> String {
    // In PostgreSQL, single quotes are escaped by doubling them
    password.replace("'", "''")
}

/// Escape a string for shell command (for use in double-quoted strings)
fn escape_shell_arg(arg: &str) -> String {
    arg.replace("\\", "\\\\")
       .replace("\"", "\\\"")
       .replace("$", "\\$")
       .replace("`", "\\`")
}

/// Attempt to fix database password mismatches
pub fn fix_database_passwords(vps: &VpsConfig, remote_dir: &str) -> Result<bool> {
    println!("\n🔧 Attempting to fix database passwords...");

    // Get passwords from .env
    let env_content = super::deploy::execute_remote_command(vps, &format!("cat {}/.env", remote_dir))?;

    let postgres_pw = extract_env_value(&env_content, "POSTGRES_PASSWORD");
    let harness_pw = extract_env_value(&env_content, "HARNESS_DB_PASSWORD");

    let mut fixed = false;

    // Fix hbec user in postgres
    // Note: POSTGRES_USER=hbec means 'hbec' is the superuser, not 'postgres'
    if let Some(pw) = postgres_pw {
        println!("   Updating hbec user password in postgres...");
        // Escape password for SQL (double single quotes) and shell (escape special chars)
        let sql_escaped_pw = escape_sql_password(&pw);
        let shell_escaped_pw = escape_shell_arg(&sql_escaped_pw);
        // Use -U hbec since that's the superuser (POSTGRES_USER=hbec in docker-compose)
        let cmd = format!(
            "docker exec hbec-postgres psql -U hbec -c \"ALTER USER hbec WITH PASSWORD '{}';\"",
            shell_escaped_pw
        );
        match super::deploy::execute_remote_command(vps, &cmd) {
            Ok(_) => {
                println!("   ✓ Updated hbec password");
                fixed = true;
            }
            Err(e) => {
                println!("   ✗ Failed to update hbec password: {}", e);
            }
        }
    }

    // Fix harness user in harness-db
    // Note: POSTGRES_USER=harness means 'harness' is the superuser
    if let Some(pw) = harness_pw {
        println!("   Updating harness user password in harness-db...");
        let sql_escaped_pw = escape_sql_password(&pw);
        let shell_escaped_pw = escape_shell_arg(&sql_escaped_pw);
        // Use -U harness since that's the superuser (POSTGRES_USER=harness in docker-compose)
        let cmd = format!(
            "docker exec hbec-harness-db psql -U harness -c \"ALTER USER harness WITH PASSWORD '{}';\"",
            shell_escaped_pw
        );
        match super::deploy::execute_remote_command(vps, &cmd) {
            Ok(_) => {
                println!("   ✓ Updated harness password");
                fixed = true;
            }
            Err(e) => {
                println!("   ✗ Failed to update harness password: {}", e);
            }
        }
    }

    if fixed {
        println!("\n   Restarting affected services...");
        let _ = super::deploy::execute_remote_command(vps, &format!(
            "cd {} && docker compose restart student-backend admin-backend harness",
            remote_dir
        ));
    }

    Ok(fixed)
}
