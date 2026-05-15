# Shipwright: Complete Implementation Plan

## Executive Summary

**Goal**: Build a deployment tool that makes CI/CD trustworthy through built-in observability and verification.

**Timeline**: 6-month MVP → 12-month v1.0  
**Team Size**: 1-2 developers initially (you + optional contributor)  
**License**: Open source (Apache 2.0 or MIT)

---

## Tech Stack Selection

### **Core CLI: Rust** ✅

**Why Rust over Go/Python**:

| Criteria | Rust | Go | Python |
|----------|------|----|----|---|
| Single binary | ✅ | ✅ | ❌ (needs runtime) |
| Speed | ✅✅✅ | ✅✅ | ✅ |
| Memory safety | ✅✅✅ | ✅ | ✅ |
| Cross-compile | ✅✅ | ✅✅ | ❌ |
| Docker SDK | ✅ Good | ✅✅ Excellent | ✅✅ Excellent |
| CLI ecosystem | ✅✅ (clap, indicatif) | ✅ (cobra) | ✅ (click, rich) |
| Learning curve | ⚠️ Steep | ✅ Easy | ✅✅ Easiest |
| Async/streaming | ✅✅ (tokio) | ✅✅ (goroutines) | ✅ (asyncio) |

**Decision: Rust**

**Rationale**:
- **Single binary** with no runtime dependencies (critical for CLI distribution)
- **Fast** (metrics processing, log parsing, concurrent deploys)
- **Memory safe** (prevents crashes during deploys)
- **Excellent async** (Tokio for WebSocket streams, parallel Docker ops)
- **Cross-platform** (Linux, macOS, Windows from one codebase)

**Trade-off**: Steeper learning curve, but worth it for a tool that needs to be rock-solid.

---

### **VPS Agent: Rust** ✅

**Why same language as CLI**:
- Share code (health checks, metrics collection, config parsing)
- One build pipeline
- Consistent performance characteristics
- Easy to vendor both in monorepo

**Agent responsibilities**:
- Collect system metrics (CPU, memory, disk, network)
- Stream Docker container stats
- Forward logs to CLI
- Execute remote commands securely
- Report health check results

**Dependencies**:
```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
bollard = "0.16"  # Docker API client
sysinfo = "0.30"  # System metrics
tokio-tungstenite = "0.21"  # WebSocket
serde = { version = "1", features = ["derive"] }
tracing = "0.1"  # Structured logging
```

---

### **Configuration: YAML** ✅

**Why YAML over TOML/JSON/HCL**:

| Format | Pros | Cons | Verdict |
|--------|------|------|---------|
| YAML | Familiar (k8s, docker-compose), concise | Indentation-sensitive | ✅ **Choose** |
| TOML | Rust-native, explicit | Verbose for nested config | ❌ |
| JSON | Universal, parseable | No comments, verbose | ❌ |
| HCL | Powerful (Terraform) | Unfamiliar to most | ❌ |

**Example `.shipwright.yml`**:
```yaml
version: 1

project:
  name: myapp
  framework: nextjs  # auto-detected

build:
  image: node:20-alpine
  cache:
    - node_modules
    - .next/cache
  steps:
    - npm ci
    - npm run build
    - npm test

deploy:
  type: docker
  registry: ghcr.io/myorg
  replicas: 2
  
  health:
    http:
      path: /health
      expect: 200
      timeout: 30s
    
    dependencies:
      - postgres://db:5432
      - redis://cache:6379
  
  resources:
    memory: 1024
    cpu: 1.0
  
  smoke_tests:
    - GET /api/health expect=200
    - GET /api/users expect=200

notifications:
  slack:
    webhook: https://hooks.slack.com/...
    on: [failure, rollback]
```

**Parser**: `serde_yaml` crate with strict validation.

---

### **Docker Orchestration: Bollard** ✅

**Why Bollard over Docker CLI**:

| Approach | Pros | Cons |
|----------|------|------|
| Bollard (Rust API) | Native async, type-safe, no shelling out | Rust-specific |
| Docker CLI | Universal, well-documented | Parsing stdout, slower |
| Docker SDK (Python) | Easy to use | Requires Python runtime |

**Decision: Bollard**

**Capabilities**:
- Build images with BuildKit
- Push to registries
- Manage containers (start, stop, inspect)
- Stream logs
- Collect stats (CPU, memory)
- Execute commands in containers

**Example usage**:
```rust
use bollard::Docker;
use bollard::container::CreateContainerOptions;

async fn deploy_container(image: &str) -> Result<String, Error> {
    let docker = Docker::connect_with_socket_defaults()?;
    
    let config = CreateContainerOptions {
        name: "myapp",
        ..Default::default()
    };
    
    let container = docker.create_container(Some(config), image).await?;
    docker.start_container(&container.id, None).await?;
    
    Ok(container.id)
}
```

---

### **Metrics & Observability: Prometheus Protocol + SQLite** ✅

**Why this combo**:

**Prometheus format** for collection:
- Industry standard
- Time-series data model
- Efficient encoding

**SQLite for storage**:
- No separate database needed
- Single file (`.shipwright/metrics.db`)
- Fast queries for dashboards
- Easy backups

**Flow**:
```
VPS Agent → [Prometheus metrics] → CLI → SQLite → TUI Dashboard
                                              ↓
                                      (optional) Prometheus exporter
```

**Schema**:
```sql
CREATE TABLE metrics (
    timestamp INTEGER NOT NULL,
    deploy_id TEXT NOT NULL,
    metric_name TEXT NOT NULL,
    metric_value REAL NOT NULL,
    labels TEXT  -- JSON: {"container": "myapp", "host": "prod-1"}
);

CREATE INDEX idx_metrics_time ON metrics(timestamp);
CREATE INDEX idx_metrics_deploy ON metrics(deploy_id);
```

**Collected metrics**:
- `cpu_usage_percent`
- `memory_usage_bytes`
- `network_rx_bytes`
- `network_tx_bytes`
- `http_requests_total`
- `http_request_duration_seconds`
- `http_errors_total`

---

### **TUI (Terminal UI): Ratatui** ✅

**Why Ratatui over alternatives**:

| Library | Language | Pros | Cons |
|---------|----------|------|------|
| Ratatui | Rust | Modern, active, great docs | Rust-only |
| tui-rs | Rust | Predecessor to Ratatui | Unmaintained |
| bubbletea | Go | Excellent Elm-style arch | Go-only |
| Rich | Python | Beautiful, easy | Python runtime |

**Decision: Ratatui**

**Features we'll use**:
- Live dashboards during deploy
- Log streaming with colors
- Interactive menus (deploy target selection)
- Progress bars and spinners
- Charts (CPU, memory over time)

**Example TUI layout**:
```
┌─ Shipwright ─────────────────────────────────────────┐
│ myapp@production (v1.2.3)                            │
│ Status: ● HEALTHY  Uptime: 2h 34m                    │
└──────────────────────────────────────────────────────┘

┌─ Metrics ────────────────────────────────────────────┐
│ CPU:  ▂▃▅▃▂▁▂ 12%    Memory: 420MB/1GB  (42%)       │
│ Req/s: 23            Errors: 0/1247     (0%)        │
└──────────────────────────────────────────────────────┘

┌─ Logs ───────────────────────────────────────────────┐
│ 14:23:45  Server started on :3000                    │
│ 14:23:47  Connected to postgres://db:5432            │
│ 14:23:48  Redis connection established               │
│ 14:23:50  GET /api/users → 200 (23ms)               │
└──────────────────────────────────────────────────────┘

[L]ogs [S]hell [M]etrics [R]ollback [Q]uit
```

**Dependencies**:
```toml
ratatui = "0.26"
crossterm = "0.27"  # Terminal control
```

---

### **Communication: WebSocket (tokio-tungstenite)** ✅

**Why WebSocket over alternatives**:

| Protocol | Use Case | Pros | Cons |
|----------|----------|------|------|
| WebSocket | CLI ↔ Agent | Bidirectional, low latency | Needs keep-alive |
| gRPC | CLI ↔ Agent | Efficient, typed | Overkill, complexity |
| HTTP/SSE | Agent → CLI | Simple, unidirectional | One-way only |
| SSH | CLI ↔ VPS | Universal | High latency, harder to program |

**Decision: WebSocket**

**Why**:
- **Bidirectional**: CLI can send commands, Agent streams metrics/logs
- **Low latency**: Real-time updates (< 100ms)
- **Firewall-friendly**: Works over HTTPS
- **Reconnection**: Auto-reconnect on network hiccups

**Security**:
- TLS encryption (wss://)
- Token-based auth (JWT or API keys)
- Rate limiting

**Example flow**:
```rust
// Agent side
async fn start_agent(auth_token: String) {
    let url = "wss://shipwright.dev/agent/connect";
    let (ws_stream, _) = connect_async(url).await?;
    
    loop {
        // Stream metrics every 2 seconds
        let metrics = collect_metrics().await;
        ws_stream.send(Message::Binary(serialize(&metrics))).await?;
        
        // Handle commands from CLI
        if let Some(msg) = ws_stream.next().await {
            handle_command(msg).await?;
        }
    }
}
```

---

### **Build System: BuildKit** ✅

**Why BuildKit over Docker CLI**:

| Feature | Docker CLI | BuildKit |
|---------|-----------|----------|
| Parallel builds | ❌ | ✅ |
| Layer caching | Basic | Advanced (cross-stage) |
| Multi-platform | Limited | ✅ |
| Secrets handling | ❌ | ✅ (mount=type=secret) |
| Performance | Slow | Fast |

**Decision: BuildKit**

**Enables**:
- Cache mounts (`--mount=type=cache,target=/root/.npm`)
- Parallel stage builds
- Cross-platform images (linux/amd64, linux/arm64)
- Secure secret injection

**Dockerfile optimization**:
```dockerfile
# syntax=docker/dockerfile:1.4

FROM node:20-alpine AS deps
WORKDIR /app
# Cache mount for npm
RUN --mount=type=cache,target=/root/.npm \
    npm ci --production

FROM node:20-alpine AS builder
WORKDIR /app
COPY package*.json ./
RUN --mount=type=cache,target=/root/.npm npm ci
COPY . .
RUN npm run build

FROM node:20-alpine AS runner
WORKDIR /app
ENV NODE_ENV=production
COPY --from=deps /app/node_modules ./node_modules
COPY --from=builder /app/.next ./.next
COPY --from=builder /app/public ./public

CMD ["npm", "start"]
```

**Bollard BuildKit integration**:
```rust
use bollard::image::BuildImageOptions;

let build_opts = BuildImageOptions {
    dockerfile: "Dockerfile",
    t: "myapp:latest",
    version: BuilderVersion::BuilderBuildKit,
    ..Default::default()
};

docker.build_image(build_opts, None, None).await?;
```

---

### **Registry: Multi-Registry Support** ✅

**Support matrix**:

| Registry | Auth Method | Priority |
|----------|------------|----------|
| Docker Hub | Username/password | ✅ Phase 1 |
| GitHub Container Registry (GHCR) | PAT | ✅ Phase 1 |
| GitLab Container Registry | Deploy token | ✅ Phase 1 |
| AWS ECR | IAM | ⚠️ Phase 2 |
| GCP Artifact Registry | Service account | ⚠️ Phase 2 |
| Self-hosted | Basic auth | ✅ Phase 1 |

**Config**:
```yaml
registry:
  type: ghcr
  url: ghcr.io/myorg
  auth:
    username: myusername
    token_file: ~/.shipwright/ghcr-token
```

**Credentials storage**: 
- Encrypted with `age` (like SOPS)
- Stored in `~/.shipwright/credentials.enc`
- Never in config files

---

### **Secret Management: SOPS + Age** ✅

**Why SOPS + Age**:

| Solution | Pros | Cons |
|----------|------|------|
| SOPS + Age | Simple, git-friendly, no server | Manual key management |
| Vault | Powerful, centralized | Needs server, complex |
| AWS Secrets Manager | Integrated | Cloud lock-in, costs |
| Encrypted env files | Simple | Manual rotation, no audit |

**Decision: SOPS + Age**

**Flow**:
```bash
# Developer creates secrets
shipwright secret set DATABASE_URL postgres://...

# Encrypted in .shipwright/secrets.enc
# Committed to git (safe)

# During deploy, agent decrypts with age key
# Injects as env vars
```

**File format** (`.shipwright/secrets.enc`):
```yaml
# Encrypted with SOPS
database_url: ENC[AES256_GCM,data:...,iv:...,tag:...]
redis_url: ENC[AES256_GCM,data:...,iv:...,tag:...]
```

**Implementation**:
```rust
use age::Decryptor;

fn decrypt_secrets(key_file: &Path) -> HashMap<String, String> {
    let encrypted = fs::read(".shipwright/secrets.enc")?;
    let key = age::IdentityFile::from_file(key_file)?;
    
    let decrypted = Decryptor::new(&encrypted[..])
        .decrypt(&key)?
        .collect()?;
    
    serde_yaml::from_slice(&decrypted)?
}
```

---

### **Database: SQLite** ✅

**Why SQLite over Postgres/MySQL**:

| Database | Pros | Cons | Verdict |
|----------|------|------|---------|
| SQLite | No server, single file, portable | Single writer | ✅ **Perfect** |
| Postgres | Powerful, concurrent writes | Needs server, overkill | ❌ |
| MySQL | Widely used | Needs server | ❌ |

**Decision: SQLite**

**What we store**:
- Deploy history
- Metrics (time-series)
- Configuration snapshots
- Audit logs
- Health check results

**Schema**:
```sql
-- Deploy history
CREATE TABLE deploys (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    environment TEXT NOT NULL,
    status TEXT NOT NULL,  -- pending, running, success, failed
    started_at INTEGER NOT NULL,
    completed_at INTEGER,
    deployed_by TEXT,
    rollback_from TEXT,  -- if this is a rollback
    confidence_score INTEGER  -- 0-100
);

-- Metrics (see earlier)
CREATE TABLE metrics (...);

-- Audit log
CREATE TABLE audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp INTEGER NOT NULL,
    user TEXT NOT NULL,
    action TEXT NOT NULL,  -- deploy, rollback, secret_set
    details TEXT  -- JSON
);

-- Health checks
CREATE TABLE health_checks (
    deploy_id TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    check_type TEXT NOT NULL,  -- http, tcp, command
    check_name TEXT NOT NULL,
    success BOOLEAN NOT NULL,
    duration_ms INTEGER,
    error_message TEXT
);
```

**Library**: `rusqlite` with migrations via `refinery`.

---

### **Logging: Tracing + Structured Logs** ✅

**Why tracing over log crate**:

| Crate | Paradigm | Best For |
|-------|----------|----------|
| `tracing` | Structured, async-aware | Modern async apps |
| `log` | Printf-style | Simple sync apps |
| `slog` | Structured | Complex logging needs |

**Decision: tracing**

**Features**:
- Structured fields (`info!(deploy_id = %id, "Starting deploy")`)
- Spans for request tracing
- Async-aware (works with Tokio)
- Multiple subscribers (stdout, file, metrics)

**Setup**:
```rust
use tracing::{info, warn, error};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn init_logging() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();
}

#[tracing::instrument]
async fn deploy(config: &Config) -> Result<(), Error> {
    info!("Starting deploy");
    
    let build_span = tracing::info_span!("build");
    let _guard = build_span.enter();
    // Build steps here
    
    Ok(())
}
```

**Output**:
```
2024-04-05T14:23:45Z INFO  deploy{deploy_id=abc123}: Starting deploy
2024-04-05T14:23:46Z INFO  build: Running npm ci
2024-04-05T14:23:48Z INFO  build: Build complete in 2.3s
```

---

### **Testing Strategy**

#### **Unit Tests** (Rust built-in)
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let yaml = r#"
            version: 1
            project:
              name: test
        "#;
        
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.project.name, "test");
    }
    
    #[tokio::test]
    async fn test_health_check() {
        let result = check_http_health("http://localhost:8080/health").await;
        assert!(result.is_ok());
    }
}
```

#### **Integration Tests**
```rust
// tests/integration_test.rs
use shipwright::docker::build_image;

#[tokio::test]
async fn test_full_deploy() {
    // Start test Docker daemon
    let docker = Docker::connect_with_socket_defaults().unwrap();
    
    // Build test image
    build_image(&docker, "tests/fixtures/Dockerfile").await.unwrap();
    
    // Deploy
    // ...
    
    // Verify health
    // ...
}
```

#### **E2E Tests** (GitHub Actions)
```yaml
# .github/workflows/e2e.yml
name: E2E Tests
on: [push]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Build CLI
        run: cargo build --release
      - name: Run E2E
        run: |
          ./target/release/shipwright init
          ./target/release/shipwright up --dry-run
```

---

### **Distribution**

#### **Binary Releases**
```yaml
# .github/workflows/release.yml
name: Release
on:
  push:
    tags:
      - 'v*'

jobs:
  build:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v3
      - name: Build
        run: cargo build --release
      - name: Upload
        uses: actions/upload-artifact@v3
        with:
          name: shipwright-${{ matrix.os }}
          path: target/release/shipwright
```

#### **Package Managers**

**Homebrew** (macOS/Linux):
```ruby
# Formula/shipwright.rb
class Shipwright < Formula
  desc "CI/CD with built-in observability"
  homepage "https://shipwright.dev"
  url "https://github.com/shipwright/shipwright/archive/v0.1.0.tar.gz"
  sha256 "..."

  def install
    bin.install "shipwright"
  end
end
```

**Cargo** (Rust):
```toml
[package]
name = "shipwright"
version = "0.1.0"
```
```bash
cargo install shipwright
```

**APT** (Debian/Ubuntu):
```bash
# Create .deb package
cargo install cargo-deb
cargo deb

# Host on Cloudsmith or GitHub Releases
```

**Scoop** (Windows):
```json
{
  "version": "0.1.0",
  "url": "https://github.com/shipwright/shipwright/releases/download/v0.1.0/shipwright-windows.zip",
  "bin": "shipwright.exe"
}
```

---

## Project Structure

```
shipwright/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── LICENSE
│
├── cli/                    # Main CLI binary
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs
│   │   ├── commands/
│   │   │   ├── mod.rs
│   │   │   ├── init.rs
│   │   │   ├── deploy.rs
│   │   │   ├── logs.rs
│   │   │   ├── rollback.rs
│   │   │   └── status.rs
│   │   ├── config/
│   │   │   ├── mod.rs
│   │   │   └── parser.rs
│   │   ├── docker/
│   │   │   ├── mod.rs
│   │   │   ├── build.rs
│   │   │   └── deploy.rs
│   │   └── tui/
│   │       ├── mod.rs
│   │       ├── dashboard.rs
│   │       └── logs.rs
│   └── tests/
│
├── agent/                  # VPS agent binary
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs
│   │   ├── metrics/
│   │   │   ├── mod.rs
│   │   │   ├── collector.rs
│   │   │   └── docker.rs
│   │   ├── health/
│   │   │   ├── mod.rs
│   │   │   └── checks.rs
│   │   └── websocket/
│   │       ├── mod.rs
│   │       └── client.rs
│   └── tests/
│
├── common/                 # Shared library
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── config.rs       # Config parsing
│       ├── models.rs       # Data models
│       ├── metrics.rs      # Metrics types
│       └── protocol.rs     # WebSocket protocol
│
├── docs/
│   ├── getting-started.md
│   ├── configuration.md
│   └── architecture.md
│
└── examples/
    ├── nextjs/
    │   └── .shipwright.yml
    ├── django/
    │   └── .shipwright.yml
    └── rails/
        └── .shipwright.yml
```

---

## Development Phases

### **Phase 1: MVP (Months 1-3)**

**Goal**: Prove the core concept works.

**Features**:
- ✅ CLI with `init`, `deploy`, `logs`, `status`
- ✅ Docker build + push to GHCR
- ✅ Deploy to single VPS via SSH + Docker Compose
- ✅ Basic health checks (HTTP only)
- ✅ Live logs streaming
- ✅ Simple TUI dashboard
- ✅ SQLite for deploy history

**Deliverable**: `shipwright init && shipwright up` works end-to-end.

**Validation**:
- Deploy a Next.js app from scratch in < 5 minutes
- See live logs during deploy
- Verify app is healthy via dashboard

---

### **Phase 2: Observability (Months 4-6)**

**Goal**: Make deploys trustworthy.

**Features**:
- ✅ VPS agent for metrics collection
- ✅ WebSocket communication
- ✅ Advanced health checks (TCP, commands, dependencies)
- ✅ Performance comparison (current vs previous deploy)
- ✅ Smoke tests
- ✅ Confidence scoring
- ✅ Automatic rollback on failure

**Deliverable**: Deploys are self-verifying.

**Validation**:
- Deploy regression (slower response time) is auto-detected
- Failing smoke test triggers rollback
- Confidence score drops below threshold → alert

---

### **Phase 3: Polish (Months 7-9)**

**Goal**: Make it delightful to use.

**Features**:
- ✅ Beautiful TUI with charts
- ✅ Framework detection (Next.js, Django, Rails, Go)
- ✅ Auto-generated config
- ✅ Secret management (SOPS + Age)
- ✅ Multi-environment support (staging, production)
- ✅ Slack/Discord notifications
- ✅ `shipwright diagnose` for troubleshooting

**Deliverable**: Zero-config deploys for common frameworks.

**Validation**:
- Run `shipwright init` in a Next.js repo → correct config generated
- No manual YAML editing needed for 80% of projects

---

### **Phase 4: Scale (Months 10-12)**

**Goal**: Support teams and complex setups.

**Features**:
- ✅ Multi-VPS deployments (load balancing)
- ✅ Kubernetes support
- ✅ Preview environments (PR deploys)
- ✅ Web dashboard (optional, hosted)
- ✅ Team features (deploy approvals, audit log)
- ✅ Plugin system
- ✅ Cloud hosting option (Shipwright Cloud)

**Deliverable**: Production-ready for teams.

**Validation**:
- 5-person team uses Shipwright for 10 microservices
- PR previews work automatically
- Deploy approvals enforced for production

---

## Tech Stack Summary

| Component | Technology | Why |
|-----------|-----------|-----|
| **CLI** | Rust | Fast, single binary, safe |
| **Agent** | Rust | Code sharing, performance |
| **Config** | YAML | Familiar, concise |
| **Docker** | Bollard | Native Rust API, async |
| **TUI** | Ratatui | Modern, beautiful |
| **Metrics** | Prometheus + SQLite | Standard format, local storage |
| **Communication** | WebSocket (tokio-tungstenite) | Real-time, bidirectional |
| **Build** | BuildKit | Fast, parallel, modern |
| **Secrets** | SOPS + Age | Git-friendly, simple |
| **Database** | SQLite | No server, portable |
| **Logging** | Tracing | Structured, async-aware |
| **Testing** | Cargo test + GitHub Actions | Built-in, reliable |
| **Distribution** | Homebrew, Cargo, APT, Scoop | Multi-platform |

---

## Why This Stack Wins

### **1. Performance**
- Rust: Fast builds, fast metrics processing
- BuildKit: Parallel builds, aggressive caching
- WebSocket: Low-latency real-time updates

### **2. Reliability**
- Rust: Memory safety, no crashes
- SQLite: No database server to fail
- Health checks: Verify before marking success

### **3. Developer Experience**
- Single binary: Just download and run
- Zero config: Auto-detect frameworks
- Beautiful TUI: See what's happening live

### **4. Simplicity**
- No external dependencies (except Docker)
- Local-first (no cloud required)
- Git-native (config in repo)

### **5. Extensibility**
- Plugin system (Phase 4)
- Multi-registry support
- Export to standard formats (docker-compose, k8s)

---

## Next Steps

**Week 1-2**: Project setup
- Initialize Cargo workspace
- Set up CI/CD (GitHub Actions)
- Create basic CLI skeleton with `clap`

**Week 3-4**: Docker integration
- Bollard setup
- Build image from Dockerfile
- Push to GHCR

**Week 5-6**: Deploy to VPS
- SSH connection
- Docker Compose deployment
- Health checks

**Week 7-8**: TUI dashboard
- Ratatui setup
- Live logs streaming
- Basic metrics display

**Week 9-12**: Polish MVP
- Framework detection
- Error handling
- Documentation

Want me to dive deeper into any specific component? The WebSocket protocol design or the health check framework would be particularly interesting to detail out.