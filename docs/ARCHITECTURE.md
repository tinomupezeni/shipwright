# Shipwright Architecture

This document describes the technical architecture of Shipwright, a VPS deployment automation tool.

## Table of Contents

- [Overview](#overview)
- [System Architecture](#system-architecture)
- [Components](#components)
- [Data Flow](#data-flow)
- [Infrastructure Detection](#infrastructure-detection)
- [Deployment Strategies](#deployment-strategies)
- [Proxy Integration](#proxy-integration)
- [Environment Management](#environment-management)
- [Security Considerations](#security-considerations)
- [Performance](#performance)

## Overview

Shipwright is designed as a **client-server architecture** where:
- **Agent** runs on the VPS as a systemd service (managed as root for orchestration)
- **CLI** runs on developer's local machine
- Communication happens via REST API and WebSocket

### Design Principles

1. **Zero-Conf "Push-to-Deploy"**: Achieving parity with PaaS platforms like Render.
2. **Infrastructure-Aware**: Auto-detect and adapt to existing setups.
3. **Integrated Lifecycle**: Build and Deploy are one atomic unit.
4. **Self-Healing**: Automated smoke tests trigger instant rollbacks.
5. **Modern Native**: Built on Docker Compose V2 for maximum reliability.

## System Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        LOCAL MACHINE                             │
│                                                                   │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │                    Shipwright CLI                           │ │
│  │                                                              │ │
│  │  - Register projects (Zero-Conf)                            │ │
│  │  - Trigger deployments                                      │ │
│  │  - Stream logs (WebSocket)                                  │ │
│  │  - Manage configurations & secrets                          │ │
│  └────────────────────────────────────────────────────────────┘ │
│                              │                                    │
└──────────────────────────────│────────────────────────────────────┘
                               │
                               │ HTTPS/WSS
                               │
┌──────────────────────────────│────────────────────────────────────┐
│                              ▼         VPS                         │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │              Shipwright Agent (systemd)                     │ │
│  │                                                              │ │
│  │  ┌────────────┐  ┌────────────┐  ┌────────────┐           │ │
│  │  │   HTTP     │  │  WebSocket │  │  GitHub    │           │ │
│  │  │   API      │  │   Server   │  │  Webhooks  │           │ │
│  │  └────────────┘  └────────────┘  └────────────┘           │ │
│  │         │               │                │                  │ │
│  │         └───────────────┴────────────────┘                  │ │
│  │                        │                                     │ │
│  │                        ▼                                     │ │
│  │  ┌────────────────────────────────────────────────────────┐│ │
│  │  │            Unified Pipeline Orchestrator               ││ │
│  │  │                                                          ││ │
│  │  │  1. Infrastructure Detection                            ││ │
│  │  │  2. Git Trust & Repository Cloning                      ││ │
│  │  │  3. Environment Validation                              ││ │
│  │  │  4. Docker Compose V2 Build                             ││ │
│  │  │  5. Smoke Tests (Pre/Post)                              ││ │
│  │  │  6. Atomic Deployment & Proxy Join                      ││ │
│  │  │  7. Auto-Rollback Engine                                ││ │
│  │  └────────────────────────────────────────────────────────┘│ │
│  └────────────────────────────────────────────────────────────┘ │
│                              │                                    │
│         ┌────────────────────┼────────────────────┐              │
│         ▼                    ▼                    ▼              │
│  ┌───────────┐        ┌───────────┐       ┌───────────┐         │
│  │  Docker   │        │  Reverse  │       │  Shared   │         │
│  │  Engine   │        │  Proxy    │       │ Resources │         │
│  │           │        │           │       │           │         │
│  │ - Build   │        │ - Caddy   │       │ - Postgres│         │
│  │ - Deploy  │        │ - Nginx   │       │ - Redis   │         │
│  │ - Monitor │        │ - Traefik │       │ - Networks│         │
│  └───────────┘        └───────────┘       └───────────┘         │
│                                                                   │
└───────────────────────────────────────────────────────────────────┘
```

## Components

### 1. CLI (`shipwright-cli`)

**Purpose**: User interface for deployment management

**Responsibilities**:
- Parse command-line arguments
- Read and validate `.shipwright.yml`
- Communicate with agent API
- Display real-time deployment progress
- Handle SSH key management

**Key Files**:
- `cli/src/main.rs`: Entry point
- `cli/src/commands/`: Command implementations

**Technologies**:
- `clap`: CLI argument parsing
- `tokio`: Async runtime
- `reqwest`: HTTP client
- `tokio-tungstenite`: WebSocket client

### 2. Agent (`shipwright-agent`)

**Purpose**: Server-side deployment orchestrator

**Responsibilities**:
- Expose REST API for deployments
- Handle GitHub webhooks
- Execute deployment pipelines
- Stream real-time logs via WebSocket
- Manage deployment state

**Key Files**:
- `agent/src/main.rs`: Entry point, HTTP server setup
- `agent/src/pipeline/build.rs`: Build orchestration
- `agent/src/pipeline/deploy.rs`: Deployment strategies
- `agent/src/infrastructure/detector.rs`: Infrastructure auto-detection
- `agent/src/infrastructure/adapters.rs`: Proxy adapters
- `agent/src/webhooks/server.rs`: GitHub webhook handling

**Technologies**:
- `axum`: Web framework
- `bollard`: Docker API client
- `tokio`: Async runtime
- `tracing`: Logging
- `serde`: Serialization

#### API Endpoints

```
POST   /deploy              # Trigger deployment
POST   /register            # Register project
GET    /ws/:project         # WebSocket log streaming
POST   /webhook/:project    # GitHub webhook endpoint
GET    /health              # Health check
```

### 3. Common (`shipwright-common`)

**Purpose**: Shared types and utilities

**Contents**:
- Configuration schema (`Config`, `BuildConfig`, `DeployConfig`)
- Shared types (`InfrastructureInfo`, `DeploymentContext`)
- Common utilities

**Key Files**:
- `common/src/config.rs`: Configuration structs
- `common/src/types.rs`: Shared types

## Data Flow

### Deployment Flow

```
1. User triggers deployment:
   CLI -> Agent (POST /deploy)

2. Agent validates request:
   - Check project exists
   - Validate configuration
   - Check prerequisites

3. Infrastructure Detection:
   - Scan for existing proxies
   - Detect Docker networks
   - Find shared resources
   - Determine deployment directory

4. Repository Cloning:
   - Convert HTTPS to SSH if private
   - Clone to deployment directory
   - Fix file ownership

5. Environment Setup:
   - Check for existing .env
   - Try .env.example/.env.template
   - Use config environment section
   - Generate minimal defaults
   - Fix file ownership

6. Docker Build:
   - Detect docker-compose file
   - Build images
   - Tag appropriately

7. Deployment:
   - Choose strategy (standalone/compose)
   - Stop old containers
   - Start new containers
   - Connect to networks

8. Smoke Tests (planned):
   - Validate environment variables
   - Test database connectivity
   - Check service health
   - Verify network routing

9. Proxy Configuration:
   - Detect proxy type
   - Generate configuration
   - Apply changes
   - Reload proxy

10. Notify completion:
    Agent -> CLI (via WebSocket)
```

### GitHub Webhook Flow

```
1. Developer pushes to GitHub:
   git push origin main

2. GitHub triggers webhook:
   GitHub -> Agent (POST /webhook/:project)

3. Agent verifies signature:
   - Validate GitHub signature
   - Check project exists

4. Start deployment:
   - Same as manual deployment flow
   - Skip steps already done recently

5. Log to journalctl:
   - All output visible via systemd
   - Also streamed via WebSocket if client connected
```

## Infrastructure Detection

### Detection Process

The agent automatically detects existing infrastructure to avoid conflicts:

```rust
pub async fn detect_infrastructure() -> Result<InfrastructureInfo> {
    // 1. Detect reverse proxy
    let proxy = detect_proxy(&docker).await?;

    // 2. Scan Docker networks
    let networks = detect_networks(&docker).await?;

    // 3. Find shared resources
    let shared_resources = detect_shared_resources(&docker).await?;

    // 4. Discover deployment directories
    let deploy_directories = detect_deploy_directories().await?;

    // 5. Determine if multi-project setup
    let is_multi_project = deploy_directories.len() > 1;

    Ok(InfrastructureInfo {
        proxy,
        networks,
        shared_resources,
        deploy_directories,
        is_multi_project,
    })
}
```

### Proxy Detection

Scans running containers for known proxy images:

```rust
async fn detect_proxy(docker: &Docker) -> Result<Option<(String, String)>> {
    for container in running_containers {
        if image.contains("caddy") || name.contains("caddy") {
            return Ok(Some(("caddy", container_name)));
        }
        if image.contains("nginx") && (name.contains("proxy") || name.contains("nginx")) {
            return Ok(Some(("nginx", container_name)));
        }
        if image.contains("traefik") {
            return Ok(Some(("traefik", container_name)));
        }
    }
    Ok(None)
}
```

### Network Detection

Lists all Docker networks for potential connections:

```rust
async fn detect_networks(docker: &Docker) -> Result<Vec<NetworkInfo>> {
    let networks = docker.list_networks(None).await?;
    // Filter and map to NetworkInfo
}
```

### Deployment Directory Detection

Searches common locations in priority order:

```rust
async fn detect_deploy_directories() -> Result<Vec<String>> {
    // Priority 1: User home directories
    // /home/*/apps
    // /home/*/projects

    // Priority 2: Current user's home
    // ~/apps
    // ~/projects

    // Priority 3: System directories
    // /opt/apps

    // NOTE: /var/www is intentionally excluded
}
```

## Deployment Strategies

Shipwright supports three deployment strategies:

### 1. Standalone

**When Used**:
- Single service projects
- No docker-compose file
- Explicit `strategy: standalone` config

**How It Works**:
```
1. Build Docker image
2. Stop existing container
3. Create new container
4. Connect to detected networks
5. Start container
```

**Implementation**: `agent/src/pipeline/deploy.rs::deploy_standalone()`

### 2. Docker Compose

**When Used**:
- docker-compose.yml exists
- Multi-service projects
- Explicit `strategy: compose` config

**How It Works**:
```
1. Find compose file (root or subdirectories)
2. Ensure .env file exists
3. Run docker-compose build
4. Run docker-compose up -d
```

**Implementation**: `agent/src/pipeline/deploy.rs::deploy_compose()`

### 3. Hybrid (Planned)

**When Used**:
- Complex multi-service projects
- Need different deployment strategies per service

**How It Works**:
```
1. Build with docker-compose
2. Deploy each service individually
3. Allows fine-grained control
```

## Proxy Integration

### Adapter Pattern

Each proxy type implements the `ProxyAdapter` trait:

```rust
#[async_trait]
pub trait ProxyAdapter: Send + Sync {
    async fn add_route(&self, config: RouteConfig) -> Result<()>;
    async fn remove_route(&self, domain: &str) -> Result<()>;
    async fn reload(&self) -> Result<()>;
    async fn test_config(&self) -> Result<bool>;
    async fn health_check(&self) -> Result<bool>;
}
```

### Caddy Adapter

```rust
pub struct CaddyAdapter {
    container_name: String,
}

impl CaddyAdapter {
    async fn add_route(&self, config: RouteConfig) -> Result<()> {
        // 1. Read current Caddyfile
        // 2. Parse and modify
        // 3. Write back
        // 4. Reload Caddy
    }
}
```

### Nginx Adapter

```rust
pub struct NginxAdapter {
    container_name: String,
    config_path: String,
}

impl NginxAdapter {
    async fn add_route(&self, config: RouteConfig) -> Result<()> {
        // 1. Generate server block
        // 2. Write to sites-available/
        // 3. Symlink to sites-enabled/
        // 4. Test configuration
        // 5. Reload Nginx
    }
}
```

### Traefik Adapter

```rust
pub struct TraefikAdapter {
    container_name: String,
}

impl TraefikAdapter {
    async fn add_route(&self, config: RouteConfig) -> Result<()> {
        // 1. Update dynamic configuration
        // 2. Traefik auto-reloads
    }
}
```

## Environment Management

### Priority Order

1. **Existing `.env`**: If found, use as-is
2. **`.env.example` or `.env.template`**: Copy to `.env`
3. **Config `environment` section**: Create from `.shipwright.yml`
4. **Auto-generated defaults**: Minimal `.env` with common variables

### Implementation

```rust
async fn ensure_env_file(
    build_dir: &Path,
    compose_file: &str,
    config: Option<&Config>
) -> Result<()> {
    let env_file = get_env_file_path(build_dir, compose_file);

    // 1. Check existing
    if env_file.exists() {
        return Ok(());
    }

    // 2. Check .env.example
    if env_example.exists() {
        fs::copy(&env_example, &env_file).await?;
        return Ok(());
    }

    // 3. Use config environment
    if let Some(env_vars) = config.build.environment {
        write_env_file(&env_file, &env_vars).await?;
        return Ok(());
    }

    // 4. Generate minimal defaults
    write_minimal_env(&env_file).await?;
}
```

### File Ownership

When agent runs as root, files are owned by root. We fix this:

```rust
async fn fix_ownership(build_dir: &Path) -> Result<()> {
    // 1. Check if running as root
    let current_user = whoami();
    if current_user != "root" {
        return Ok(());
    }

    // 2. Get directory owner
    let owner = get_directory_owner(build_dir)?;

    // 3. Change ownership recursively
    chown_recursive(build_dir, &owner)?;
}
```

## Security Considerations

### 1. SSH Key Management

- Private keys never leave local machine
- Agent uses keys copied to root (for private repos)
- Keys should be properly protected (600 permissions)

### 2. GitHub Webhooks

- Webhook signatures verified using HMAC
- Shared secret stored securely
- Invalid signatures rejected

### 3. Environment Variables

- `.env` files excluded from git
- Sensitive values should use secret management
- Auto-generated defaults are insecure (must be updated)

### 4. Docker Socket Access

- Agent needs Docker socket access
- Runs as systemd service (controlled permissions)
- Be cautious of container escape vulnerabilities

### 5. Network Isolation

- Projects deployed to specific networks
- Shared resources on separate networks
- Proper network segmentation

## Performance

### Build Optimization

1. **Local Builds**: Builds happen on VPS (no image pushing/pulling)
2. **Layer Caching**: Docker build cache reused
3. **Parallel Builds**: Services built in parallel when possible

### Deployment Speed

- **Standalone**: ~5-10 seconds (stop/start containers)
- **Compose**: ~30-60 seconds (depends on services)
- **With Build**: +2-5 minutes (depends on project size)

### Resource Usage

Agent is lightweight:
- **Memory**: ~20-50 MB at rest
- **CPU**: Minimal when idle, spikes during builds
- **Disk**: Only deployment directories and Docker images

### Scaling

Current limitations:
- Single-server deployments only
- No horizontal scaling (yet)
- No load balancing (handled by proxy)

Future improvements:
- Multi-server deployments
- Container orchestration (Kubernetes)
- Distributed builds

## Future Architecture

### Planned Improvements

1. **State Management**
   - SQLite database for deployment history
   - Rollback capabilities
   - Deployment audit log

2. **Health Monitoring**
   - Continuous health checks
   - Automatic remediation
   - Alert integration

3. **Blue-Green Deployments**
   - Zero-downtime deployments
   - Automatic traffic switching
   - Quick rollback

4. **Multi-Server Support**
   - Deploy to multiple VPS
   - Load balancing
   - Geographic distribution

5. **Plugin System**
   - Custom deployment strategies
   - Third-party integrations
   - Hook system for custom logic

---

For implementation details, see the source code in `agent/src/` and `cli/src/`.
