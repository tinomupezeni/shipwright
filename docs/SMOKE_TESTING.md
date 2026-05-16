# Smoke Testing Framework

This document outlines Shipwright's comprehensive smoke testing system designed to catch deployment issues before they cause downtime.

## Table of Contents

- [Overview](#overview)
- [Problem Statement](#problem-statement)
- [Test Categories](#test-categories)
- [Implementation Design](#implementation-design)
- [Configuration](#configuration)
- [Test Execution Flow](#test-execution-flow)
- [Failure Handling](#failure-handling)
- [Extending Tests](#extending-tests)

## Overview

Smoke tests are automated validation checks that run immediately after deployment to ensure the system is functioning correctly. They catch common deployment issues early, preventing silent failures.

### Design Goals

1. **Fast**: Complete in <2 minutes
2. **Comprehensive**: Cover common failure modes
3. **Actionable**: Clear failure messages with remediation steps
4. **Configurable**: Enable/disable tests per project
5. **Non-destructive**: Read-only validation (no data mutation)

## Problem Statement

Based on analysis of real-world deployment failures (see dev-logs repository), we've identified recurring issues:

### Top Deployment Failures (From Dev-Logs Analysis)

1. **Environment Variable Issues (40%)**
   - Vite build-time vs runtime confusion
   - Missing `EXPO_PUBLIC_` prefixes
   - Hardcoded localhost URLs
   - `.env` file not loaded

2. **Docker/Container Issues (35%)**
   - Volume permission mismatches
   - Line ending incompatibilities (CRLF vs LF)
   - DNS resolution problems
   - Image tag/registry prefix mismatches
   - Stale proxy configuration

3. **Database Connection Issues (15%)**
   - Special characters in passwords breaking parsers
   - Missing database creation
   - Migration conflicts
   - Wrong database credentials

4. **Network/Routing Problems (15%)**
   - Domain routing swapped
   - Containers not on correct networks
   - IPv6 localhost resolution issues
   - Proxy configuration not reloaded

5. **Static Files & Build Artifacts (10%)**
   - Permission errors on static file volumes
   - Missing build artifacts
   - Build arguments not passed correctly

## Test Categories

### 1. Pre-Deployment Tests

Run before deployment starts to validate configuration:

#### 1.1 Configuration Validation

```yaml
- name: validate_shipwright_config
  description: Validate .shipwright.yml syntax and required fields
  checks:
    - Valid YAML syntax
    - Required fields present (project.name, deploy.type, etc.)
    - Valid deployment strategy
    - Valid proxy type (if specified)
```

#### 1.2 Environment File Validation

```yaml
- name: validate_env_file
  description: Ensure environment files exist and contain required variables
  checks:
    - .env file exists or can be generated
    - No placeholder values (changeme, your-key-here, etc.)
    - Required variables present for framework
    - Database connection string parseable
    - No special characters breaking parsers
```

#### 1.3 Docker Compose Validation

```yaml
- name: validate_compose_file
  description: Validate docker-compose.yml syntax
  checks:
    - Valid YAML syntax
    - No CRLF line endings (Windows → Linux issues)
    - Services reference existing images or build contexts
    - Networks are defined or external
    - Environment variables properly referenced
```

#### 1.4 Build Prerequisites

```yaml
- name: check_build_prerequisites
  description: Ensure build can proceed
  checks:
    - Docker is running
    - Sufficient disk space (>5GB free)
    - Build context accessible
    - Base images pullable
```

### 2. Post-Build Tests

Run after build completes but before deployment:

#### 2.1 Image Validation

```yaml
- name: validate_built_images
  description: Ensure Docker images built correctly
  checks:
    - All specified images exist
    - Images have correct tags
    - Images are not dangling
    - Image size reasonable (<2GB warning)
```

#### 2.2 Build Artifact Validation

```yaml
- name: validate_build_artifacts
  description: Check build produced expected artifacts
  checks:
    - Static files generated (if applicable)
    - Environment variables baked into frontend builds
    - No localhost URLs in production builds
    - Build argument values correct
```

### 3. Post-Deployment Tests

Run after containers start:

#### 3.1 Container Health Checks

```yaml
- name: check_container_health
  description: Verify all containers are running
  checks:
    - All expected containers exist
    - All containers in "running" state (not restarting)
    - No containers in crash loop
    - Containers stayed up for >30 seconds
  severity: critical
  timeout: 60s
```

#### 3.2 Environment Variable Verification

```yaml
- name: verify_environment_variables
  description: Check containers have correct environment
  checks:
    - Required variables present in container
    - No placeholder values at runtime
    - Database URLs properly formatted
    - API endpoints not localhost
    - ALLOWED_HOSTS properly formatted (Django)
  severity: critical
  timeout: 10s
```

#### 3.3 Volume Permission Checks

```yaml
- name: check_volume_permissions
  description: Ensure volume permissions are correct
  checks:
    - Static file volumes writable
    - Media volumes writable
    - No root-owned files blocking access
    - Correct user:group ownership
  severity: high
  timeout: 10s
```

#### 3.4 Network Connectivity Tests

```yaml
- name: test_network_connectivity
  description: Verify network configuration
  checks:
    - Containers on expected networks
    - DNS resolution works (shared-postgres, shared-redis)
    - Inter-service communication possible
    - External network access (if needed)
  severity: critical
  timeout: 30s
```

#### 3.5 Database Connectivity

```yaml
- name: test_database_connection
  description: Verify database connectivity
  checks:
    - Can resolve database hostname
    - Can connect to database port
    - Authentication succeeds
    - Database exists
    - User has correct permissions
  severity: critical
  timeout: 30s
  retries: 3
```

#### 3.6 Redis/Cache Connectivity

```yaml
- name: test_redis_connection
  description: Verify cache connectivity
  checks:
    - Can resolve Redis hostname
    - Can connect to Redis port
    - Can SET and GET test key
    - Correct database number
  severity: high
  timeout: 10s
```

#### 3.7 HTTP Health Endpoints

```yaml
- name: test_http_health_endpoints
  description: Verify services respond to HTTP
  checks:
    - Health endpoint returns 200
    - Response time <5 seconds
    - No 502/503/504 errors
    - Correct response format
  severity: critical
  timeout: 30s
```

#### 3.8 Migration Status

```yaml
- name: check_migration_status
  description: Ensure database migrations applied
  checks:
    - No pending migrations (Django)
    - No migration conflicts
    - Migration history clean
  severity: high
  timeout: 20s
```

#### 3.9 Static Files Serving

```yaml
- name: test_static_files
  description: Verify static files accessible
  checks:
    - Static files served correctly
    - Correct MIME types
    - No 404s for common assets
    - collectstatic completed successfully
  severity: medium
  timeout: 15s
```

#### 3.10 Proxy Routing Validation

```yaml
- name: validate_proxy_routing
  description: Ensure reverse proxy routes traffic correctly
  checks:
    - Domains resolve to correct services
    - No routing swaps (admin vs frontend)
    - Proxy config reloaded
    - TLS certificates valid (if HTTPS)
  severity: critical
  timeout: 30s
```

#### 3.11 Log Output Inspection

```yaml
- name: inspect_container_logs
  description: Check logs for errors
  checks:
    - No critical errors in logs
    - No connection refused errors
    - No authentication failures
    - No import errors
    - No missing file errors
  severity: medium
  timeout: 10s
```

### 4. Integration Tests (Optional)

More thorough tests for production deployments:

#### 4.1 API Endpoint Tests

```yaml
- name: test_api_endpoints
  description: Verify API endpoints functional
  checks:
    - Can create test resource
    - Can retrieve test resource
    - Can update test resource
    - Can delete test resource
    - Authentication works
  severity: high
  timeout: 60s
  cleanup: true  # Delete test data after
```

#### 4.2 Frontend Rendering

```yaml
- name: test_frontend_rendering
  description: Verify frontend loads
  checks:
    - Homepage returns 200
    - No console errors
    - Assets load correctly
    - API calls to correct endpoints
  severity: medium
  timeout: 30s
```

## Implementation Design

### Test Runner Architecture

```rust
pub struct SmokeTestRunner {
    deployment_context: DeploymentContext,
    config: SmokeTestConfig,
    results: Vec<TestResult>,
}

impl SmokeTestRunner {
    pub async fn run_all_tests(&mut self) -> Result<TestReport> {
        // 1. Pre-deployment tests
        self.run_category(TestCategory::PreDeployment).await?;

        // 2. Post-build tests
        self.run_category(TestCategory::PostBuild).await?;

        // 3. Post-deployment tests
        self.run_category(TestCategory::PostDeployment).await?;

        // 4. Generate report
        self.generate_report()
    }

    async fn run_category(&mut self, category: TestCategory) -> Result<()> {
        let tests = self.get_tests_for_category(category);

        for test in tests {
            if !test.should_run(&self.config) {
                continue;
            }

            let result = self.run_test(test).await;
            self.results.push(result);

            // Fail fast on critical errors
            if result.is_failure() && result.severity == Severity::Critical {
                return Err(anyhow!("Critical test failed: {}", test.name));
            }
        }

        Ok(())
    }

    async fn run_test(&self, test: &SmokeTest) -> TestResult {
        let start = Instant::now();

        let outcome = match timeout(test.timeout, test.execute(&self.deployment_context)).await {
            Ok(Ok(())) => TestOutcome::Passed,
            Ok(Err(e)) => TestOutcome::Failed(e.to_string()),
            Err(_) => TestOutcome::Timeout,
        };

        TestResult {
            test_name: test.name.clone(),
            outcome,
            duration: start.elapsed(),
            severity: test.severity,
        }
    }
}
```

### Test Definition

```rust
pub struct SmokeTest {
    pub name: String,
    pub description: String,
    pub category: TestCategory,
    pub severity: Severity,
    pub timeout: Duration,
    pub execute: Box<dyn Fn(&DeploymentContext) -> BoxFuture<'static, Result<()>>>,
}

pub enum TestCategory {
    PreDeployment,
    PostBuild,
    PostDeployment,
    Integration,
}

pub enum Severity {
    Critical,  // Must pass
    High,      // Should pass, warn on failure
    Medium,    // Nice to have
    Low,       // Informational
}

pub enum TestOutcome {
    Passed,
    Failed(String),
    Skipped(String),
    Timeout,
}
```

### Example Test Implementation

```rust
// Test: Database connectivity
async fn test_database_connectivity(ctx: &DeploymentContext) -> Result<()> {
    // 1. Get database config from environment
    let db_url = get_database_url(ctx)?;
    let (user, password, host, port, database) = parse_database_url(&db_url)?;

    // 2. Check DNS resolution
    let resolved_ip = resolve_hostname(&host).await
        .context("Failed to resolve database hostname")?;
    info!("✓ Database hostname resolved to {}", resolved_ip);

    // 3. Check port connectivity
    check_tcp_connection(&host, port).await
        .context("Failed to connect to database port")?;
    info!("✓ Database port {} is accessible", port);

    // 4. Attempt authentication
    let mut conn = PostgresConnection::connect(&db_url).await
        .context("Failed to authenticate with database")?;
    info!("✓ Database authentication successful");

    // 5. Check database exists
    let exists: bool = conn.query_one(
        "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)",
        &[&database]
    ).await?.get(0);

    if !exists {
        bail!("Database '{}' does not exist", database);
    }
    info!("✓ Database '{}' exists", database);

    // 6. Check permissions
    conn.execute("SELECT 1", &[]).await
        .context("User lacks SELECT permission")?;
    info!("✓ Database permissions OK");

    Ok(())
}
```

## Configuration

### In `.shipwright.yml`

```yaml
smoke_tests:
  # Enable/disable smoke tests
  enabled: true

  # Fail deployment on test failures
  fail_on_error: true

  # Which categories to run
  categories:
    - pre_deployment
    - post_build
    - post_deployment
    # - integration  # Optional, slower tests

  # Test-specific configuration
  tests:
    database_connectivity:
      enabled: true
      timeout: 30s
      retries: 3

    http_health_endpoints:
      enabled: true
      endpoints:
        - path: /health/
          expect: 200
        - path: /api/health/
          expect: 200

    proxy_routing:
      enabled: true
      validate_https: true

  # Framework-specific tests
  framework_tests:
    django:
      - check_migrations
      - check_static_files
      - check_allowed_hosts

    nodejs:
      - check_node_modules
      - check_build_artifacts

    react:
      - check_vite_env_vars
      - check_api_endpoints_not_localhost
```

## Test Execution Flow

```
Deployment Started
      │
      ▼
┌─────────────────────┐
│ Pre-Deployment Tests│
│                     │
│ - Config validation │
│ - Env file check    │
│ - Compose syntax    │
│ - Prerequisites     │
└─────────────────────┘
      │
      ▼ (All passed)
┌─────────────────────┐
│   Build Phase       │
└─────────────────────┘
      │
      ▼
┌─────────────────────┐
│  Post-Build Tests   │
│                     │
│ - Image validation  │
│ - Artifact check    │
└─────────────────────┘
      │
      ▼ (All passed)
┌─────────────────────┐
│  Deployment Phase   │
└─────────────────────┘
      │
      ▼
┌─────────────────────┐
│Post-Deployment Tests│
│                     │
│ - Container health  │
│ - Network tests     │
│ - Database tests    │
│ - HTTP health       │
│ - Proxy routing     │
└─────────────────────┘
      │
      ├──► (All passed)
      │         │
      │         ▼
      │    Deployment Complete ✓
      │
      └──► (Some failed)
                │
                ▼
         ┌──────────────────┐
         │ Failure Handling │
         │                  │
         │ - Log failures   │
         │ - Notify user    │
         │ - Rollback? (optional) │
         └──────────────────┘
```

## Failure Handling

### Severity-Based Actions

```rust
match test_result.severity {
    Severity::Critical => {
        // Critical failures block deployment
        error!("Critical test failed: {}", test_result.test_name);
        error!("Error: {}", test_result.failure_message);
        error!("Remediation: {}", test_result.remediation_steps);

        if config.rollback_on_critical_failure {
            rollback_deployment(ctx).await?;
        }

        return Err(anyhow!("Deployment failed smoke tests"));
    }

    Severity::High => {
        // High severity: warn but continue
        warn!("Important test failed: {}", test_result.test_name);
        warn!("Consider fixing: {}", test_result.failure_message);
        // Continue deployment
    }

    Severity::Medium | Severity::Low => {
        // Log for awareness
        info!("Test failed (non-blocking): {}", test_result.test_name);
    }
}
```

### Remediation Suggestions

Tests include remediation steps:

```rust
pub struct TestResult {
    test_name: String,
    outcome: TestOutcome,
    failure_message: Option<String>,
    remediation_steps: Option<Vec<String>>,
    // ...
}

// Example
TestResult {
    test_name: "database_connectivity".to_string(),
    outcome: TestOutcome::Failed("Authentication failed".to_string()),
    remediation_steps: Some(vec![
        "Check POSTGRES_PASSWORD in .env file".to_string(),
        "Verify database user exists: docker exec shared-postgres psql -U admin -c \"\\du\"".to_string(),
        "Reset password: docker exec shared-postgres psql -U admin -c \"ALTER USER myapp WITH PASSWORD 'new_password';\"".to_string(),
    ]),
    // ...
}
```

## Extending Tests

### Adding Custom Tests

```yaml
# .shipwright.yml
smoke_tests:
  custom_tests:
    - name: check_stripe_api_key
      description: Verify Stripe API key is set
      script: |
        #!/bin/bash
        if [ -z "$STRIPE_API_KEY" ]; then
          echo "STRIPE_API_KEY not set"
          exit 1
        fi
        echo "✓ Stripe API key configured"

    - name: test_redis_cache_backend
      description: Ensure Redis cache backend working
      language: python
      script: |
        import redis
        r = redis.Redis(host='shared-redis', port=6379, db=0)
        r.set('test_key', 'test_value')
        assert r.get('test_key') == b'test_value'
        print("✓ Redis cache working")
```

### Framework-Specific Test Plugins

```rust
pub trait FrameworkTestPlugin {
    fn name(&self) -> &str;
    fn tests(&self) -> Vec<SmokeTest>;
}

pub struct DjangoTestPlugin;

impl FrameworkTestPlugin for DjangoTestPlugin {
    fn name(&self) -> &str {
        "django"
    }

    fn tests(&self) -> Vec<SmokeTest> {
        vec![
            SmokeTest {
                name: "check_django_migrations".to_string(),
                description: "Ensure Django migrations are up to date".to_string(),
                execute: Box::new(|ctx| Box::pin(check_django_migrations(ctx))),
                // ...
            },
            // More Django-specific tests
        ]
    }
}
```

## Test Report Format

```
=====================================================
       Shipwright Smoke Test Report
=====================================================
Project: myapp
Strategy: docker-compose
Timestamp: 2026-05-16 10:30:45 UTC
Duration: 1m 23s

-----------------------------------------------------
Pre-Deployment Tests (4/4 passed)
-----------------------------------------------------
✓ validate_shipwright_config      [0.1s]
✓ validate_env_file                [0.2s]
✓ validate_compose_file            [0.1s]
✓ check_build_prerequisites        [0.3s]

-----------------------------------------------------
Post-Build Tests (2/2 passed)
-----------------------------------------------------
✓ validate_built_images            [0.5s]
✓ validate_build_artifacts         [0.3s]

-----------------------------------------------------
Post-Deployment Tests (9/10 passed, 1 warning)
-----------------------------------------------------
✓ check_container_health           [30.2s]
✓ verify_environment_variables     [1.3s]
✓ check_volume_permissions         [0.8s]
✓ test_network_connectivity        [2.1s]
✓ test_database_connection         [1.7s]
✓ test_redis_connection            [0.4s]
✓ test_http_health_endpoints       [3.2s]
✓ check_migration_status           [2.8s]
⚠ test_static_files                [1.1s] (Warning)
  Some static files not found, but not critical
✓ validate_proxy_routing           [5.3s]

-----------------------------------------------------
Summary
-----------------------------------------------------
Total Tests: 16
Passed: 15
Warnings: 1
Failed: 0

Status: ✅ DEPLOYMENT SUCCESSFUL

Next Steps:
- Review warning for test_static_files
- Verify application functionality manually
- Monitor logs for first 10 minutes

=====================================================
```

## Integration with Deployment Pipeline

```rust
// In agent/src/pipeline/deploy.rs

impl DeploymentContext {
    pub async fn deploy(&self, config: Option<&Config>) -> Result<()> {
        // Pre-deployment tests
        info!("Running pre-deployment smoke tests...");
        let mut test_runner = SmokeTestRunner::new(self, config)?;
        test_runner.run_category(TestCategory::PreDeployment).await?;

        // Build phase
        info!("Building application...");
        self.build().await?;

        // Post-build tests
        info!("Running post-build smoke tests...");
        test_runner.run_category(TestCategory::PostBuild).await?;

        // Deploy phase
        info!("Deploying application...");
        match &self.strategy {
            DeployStrategy::Standalone => self.deploy_standalone().await?,
            DeployStrategy::Compose { file } => self.deploy_compose(file).await?,
            DeployStrategy::Hybrid => self.deploy_hybrid().await?,
        }

        // Post-deployment tests
        info!("Running post-deployment smoke tests...");
        test_runner.run_category(TestCategory::PostDeployment).await?;

        // Generate and log report
        let report = test_runner.generate_report();
        info!("\n{}", report);

        // Fail deployment if critical tests failed
        if report.has_critical_failures() {
            return Err(anyhow!("Deployment failed smoke tests"));
        }

        Ok(())
    }
}
```

---

## Implementation Checklist

- [ ] Create `agent/src/smoke_tests/` module
- [ ] Implement `SmokeTestRunner`
- [ ] Implement core smoke tests
- [ ] Add framework-specific test plugins
- [ ] Integrate with deployment pipeline
- [ ] Add configuration parsing
- [ ] Implement test report generation
- [ ] Add custom test script support
- [ ] Document all available tests
- [ ] Create example test configurations

---

For questions or contributions, see [CONTRIBUTING.md](../CONTRIBUTING.md).
