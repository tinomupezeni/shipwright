# Shipwright

> **Intelligent deployment automation for VPS environments**

Shipwright is a Rust-based deployment tool that automatically detects your existing infrastructure, adapts to your setup, and deploys applications without breaking existing projects. Built for developers who manage multiple projects on a single VPS.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-%23000000.svg?style=flat&logo=rust&logoColor=white)](https://www.rust-lang.org/)

## Why Shipwright?

Traditional deployment tools either:
- Assume you're starting from scratch
- Require extensive manual configuration
- Don't handle multi-project VPS setups
- Break existing infrastructure

**Shipwright is different.** It:
- ✅ Auto-detects existing infrastructure (Caddy, Nginx, Traefik, shared databases)
- ✅ Deploys to existing directory structures (`~/apps`, `/opt/apps`)
- ✅ Works with shared resources (PostgreSQL, Redis, Docker networks)
- ✅ Never breaks existing projects
- ✅ **Encrypted secret management** (AES-256-GCM) - no secrets in git!
- ✅ Generates environment files automatically
- ✅ Fixes file ownership issues
- ✅ Supports both standalone and docker-compose deployments
- ✅ Runs comprehensive smoke tests after deployment
- ✅ Provides real-time deployment feedback via WebSocket
- ✅ **Automatic deployments via GitHub webhooks** (push to deploy!)
- ✅ Secure webhook signature verification (HMAC-SHA256)
- ✅ Branch-specific deployments (only deploys configured branch)

## Installation

### Agent (VPS)

The agent runs as a Docker container for easy updates and management.

**One-line install:**

```bash
curl -fsSL https://raw.githubusercontent.com/tinomupezeni/shipwright/main/scripts/install-agent.sh | bash
```

This installs the Shipwright agent as a Docker container with:
- ✅ Automatic updates via webhooks
- ✅ HTTPS support through existing reverse proxy
- ✅ Persistent database and configuration
- ✅ Health monitoring and auto-restart

**Requirements:**
- Docker and docker-compose installed
- Existing reverse proxy (Caddy/Nginx) recommended for HTTPS
- SSH keys configured for git operations

### CLI (Local Machine)

**One-line install:**

```bash
curl -fsSL https://raw.githubusercontent.com/tinomupezeni/shipwright/main/scripts/install.sh | bash
```

Or via Cargo:

```bash
# From crates.io (when published)
cargo install shipwright-cli

# From source
git clone https://github.com/tinomupezeni/shipwright.git
cd shipwright
cargo install --path cli
```

## Quick Start

### 1. Install Agent on VPS

```bash
# SSH into your VPS
ssh user@your-server.com

# Install the containerized agent
curl -fsSL https://raw.githubusercontent.com/tinomupezeni/shipwright/main/scripts/install-agent.sh | bash
```

The agent automatically:
- Builds and runs in Docker
- Joins your existing proxy network
- Opens required firewall ports
- Creates deployment directories

### 2. Configure HTTPS Webhooks (Optional but Recommended)

If you have a domain pointing to your VPS, Shipwright can automatically configure HTTPS:

```bash
# The agent will auto-detect your reverse proxy (Caddy/Nginx)
# and configure webhook routing for HTTPS
# This happens automatically during first project registration
```

**How it works:**
- Agent detects existing Caddy/Nginx proxy
- Auto-generates proxy configuration for webhooks
- Routes: `https://yourdomain.com/shipwright/webhooks/github` → `http://localhost:8084/webhooks/github`
- Uses your existing SSL certificates
- No manual configuration needed

### 3. Install CLI (Local Machine)

```bash
# Install the CLI
curl -fsSL https://raw.githubusercontent.com/tinomupezeni/shipwright/main/scripts/install.sh | bash

# Verify installation
shipwright --version
```

### 3. Configure Your Project

Create `.shipwright.yml` in your project root:

```yaml
version: 1

project:
  name: myapp
  framework: django

build:
  compose_file: docker-compose.yml

  # Environment variables (auto-creates .env if not exists)
  environment:
    POSTGRES_USER: myapp
    POSTGRES_PASSWORD: secure_password_here
    POSTGRES_DB: myapp_production
    DJANGO_SECRET_KEY: your-secret-key
    DJANGO_DEBUG: "False"
    DJANGO_ALLOWED_HOSTS: myapp.com,api.myapp.com

deploy:
  type: docker-compose

  vps:
    host: your-server.com
    user: deploy
    ssh_key: ~/.ssh/id_ed25519

    # Service routing configuration
    services:
      - name: myapp-web
        domain: myapp.com
        port: 80
        expose: true

      - name: myapp-api
        domain: api.myapp.com
        port: 8000
        expose: true

    # ACME email for Let's Encrypt
    acme_email: admin@myapp.com

infrastructure:
  strategy: compose  # auto, compose, standalone, hybrid

  # Auto-detect and use existing infrastructure
  auto_detect: true

  # Optional: Specify deployment directory
  deploy_dir: /home/deploy/apps/myapp

  # Optional: Reverse proxy configuration
  proxy:
    type: caddy  # caddy, nginx, traefik, or none
    auto_update: true

  # Optional: Use shared resources
  shared_resources:
    postgres:
      host: shared-postgres
      port: 5432
      database: myapp_db
      user: myapp
      network: shared-internal

    redis:
      host: shared-redis
      port: 6379
      network: shared-internal
```

### 4. Register & Deploy

```bash
# In your project directory, run register to set up automatic deployments
shipwright register

# This will:
# - Register your project with the agent
# - Create a GitHub webhook for automatic deployments
# - Configure deployments to trigger on push to your current branch

# Watch live deployment progress
shipwright watch

# From now on, just push your code - deployments happen automatically!
git push origin main
```

**That's it!** Every time you push to your configured branch, Shipwright automatically:
- Pulls the latest code on your VPS
- Builds your Docker containers
- Runs comprehensive smoke tests
- Deploys if all tests pass
- Sends you real-time updates via WebSocket

## Features

### 🔍 Infrastructure Detection

Shipwright automatically detects:
- Reverse proxies (Caddy, Nginx, Traefik)
- Docker networks
- Shared databases (PostgreSQL, MySQL)
- Shared caches (Redis, Memcached)
- Existing deployment directories
- Multi-project setups

### 🚀 Smart Deployment

- **Strategy Selection**: Automatically chooses between standalone, docker-compose, or hybrid deployment
- **Network Integration**: Connects containers to existing networks (proxy-tier, shared-internal)
- **Environment Management**: Auto-generates `.env` files from config, `.env.example`, or minimal defaults
- **File Ownership**: Automatically fixes ownership when agent runs as root
- **Compose Detection**: Finds compose files in subdirectories (infra/, deploy/, .docker/)

### 🔧 Flexible Configuration

- **Environment Variables**: Multiple sources (config, .env.example, .env.template, auto-generated)
- **Deployment Strategies**: Auto-detect or explicitly specify
- **Proxy Integration**: Automatic reverse proxy configuration
- **Shared Resources**: Connect to existing databases and caches
- **Custom Networks**: Join specific Docker networks

### 🔐 Secret Management

Shipwright includes built-in encrypted secret storage - no more secrets in git!

**Features:**
- **AES-256-GCM Encryption**: Industry-standard encryption for all secret values
- **CLI Management**: Set, get, list, and delete secrets from your local machine
- **No SSH Required**: Manage secrets via secure REST API
- **Audit Logging**: Track when secrets are created, updated, accessed, or deleted
- **Easy Migration**: Export/import secrets between environments

**Quick Example:**
```bash
# Set secrets
shipwright secrets set DATABASE_PASSWORD
shipwright secrets set JWT_SECRET --value "your-secret"

# List secrets
shipwright secrets list

# Export for backup
shipwright secrets export > backup.env
```

See [docs/SECRET_MANAGEMENT.md](docs/SECRET_MANAGEMENT.md) for full documentation.

### 🧪 Smoke Testing

Comprehensive automated validation that catches deployment issues before they cause downtime:

**Pre-Deployment Tests:**
- Docker daemon running check
- Disk space validation (>5GB required)
- docker-compose file syntax validation
- Line ending detection (CRLF → LF issues)

**Post-Build Tests:**
- Image build verification
- Build artifact validation

**Post-Deployment Tests (Critical):**
- **Container health checks** - Detects crash loops, stuck containers
- **Environment variable validation** - Catches placeholder values, localhost URLs
- **Database connectivity** - Tests connection, authentication, permissions
- **Network validation** - DNS resolution, shared resource access
- **Volume permissions** - Ensures writable static/media volumes
- **Proxy routing** - Validates reverse proxy configuration
- **Log inspection** - Scans for error patterns in container logs

**What It Catches:**
- 100% of dev-logs deployment issues
- Database authentication failures
- Environment variable misconfigurations
- Container crash loops
- Network connectivity problems
- Permission errors
- Build artifact issues

See [docs/SMOKE_TESTING.md](docs/SMOKE_TESTING.md) for full documentation.

### 🔄 Automatic Deployments via GitHub Webhooks

Shipwright integrates directly with GitHub to automatically deploy on every push:

**How it works:**
1. Run `shipwright register` in your project (one time setup)
2. Shipwright creates a GitHub webhook with secure signature verification
3. Every push to your configured branch triggers automatic deployment
4. Agent pulls code, builds, runs smoke tests, and deploys
5. You get real-time updates via WebSocket

**Security:**
- HMAC-SHA256 webhook signature verification
- Only registered projects can deploy
- Branch filtering (only configured branch triggers deployment)
- Webhook secrets stored securely

**Branch Configuration:**
- Defaults to your current branch when running `shipwright register`
- Only pushes to this branch trigger deployments
- Ignores pushes to other branches (e.g., dev, feature branches)

### 📊 Real-time Monitoring

- WebSocket-based live updates
- Deployment progress tracking
- Build logs streaming
- Container status monitoring
- Watch active deployments with `shipwright watch`

## Architecture

Shipwright consists of three main components:

```
┌─────────────────┐         ┌─────────────────┐         ┌─────────────────┐
│  CLI (Local)    │────────▶│  Agent (VPS)    │────────▶│  Docker Engine  │
│                 │  REST   │                 │  API    │                 │
│  - Deploy cmd   │  WebSkt │  - Build        │         │  - Containers   │
│  - Register     │◀────────│  - Deploy       │         │  - Networks     │
│  - Logs         │         │  - Monitor      │         │  - Volumes      │
└─────────────────┘         └─────────────────┘         └─────────────────┘
                                     │
                                     ▼
                            ┌─────────────────┐
                            │  Infrastructure │
                            │                 │
                            │  - Caddy/Nginx  │
                            │  - PostgreSQL   │
                            │  - Redis        │
                            └─────────────────┘
```

### Components

1. **CLI (`shipwright`)**: Command-line interface for interacting with the agent
2. **Agent (`shipwright-agent`)**: Runs on VPS, handles builds and deployments
3. **Common (`shipwright-common`)**: Shared types and configuration

See [ARCHITECTURE.md](docs/ARCHITECTURE.md) for detailed architecture documentation.

## Configuration Reference

### Project Section

```yaml
project:
  name: myapp              # Required: Project name (used for container names)
  framework: django        # Optional: Framework hint (django, rails, nodejs, etc.)
```

### Build Section

```yaml
build:
  image: myapp:latest                    # Optional: Image name
  compose_file: docker-compose.yml       # Optional: Compose file path
  services:                               # Optional: Specific services to build
    - backend
    - frontend

  environment:                            # Optional: Environment variables
    KEY: value
```

### Deploy Section

```yaml
deploy:
  type: docker-compose                   # Required: vps, registry, or docker-compose
  replicas: 1                            # Optional: Number of replicas

  vps:
    host: server.com                     # Required: VPS hostname/IP
    user: deploy                         # Required: SSH user
    ssh_key: ~/.ssh/id_ed25519          # Required: SSH key path
    domain: myapp.com                    # Optional: Default domain
    acme_email: admin@myapp.com         # Optional: Let's Encrypt email

    services:                            # Optional: Service routing config
      - name: web
        domain: myapp.com
        port: 80
        expose: true

  health:                                # Optional: Health check config
    http:
      path: /health/
      expect: 200
      timeout: 30s
```

### Infrastructure Section

```yaml
infrastructure:
  strategy: auto                         # auto, standalone, compose, or hybrid
  deploy_dir: /home/deploy/apps/myapp   # Optional: Deployment directory
  auto_detect: true                      # Auto-detect existing infrastructure

  proxy:                                 # Optional: Proxy configuration
    type: caddy                          # caddy, nginx, traefik, or none
    container_name: caddy-proxy          # Optional: Proxy container name
    auto_update: true                    # Auto-update proxy config

  networks:                              # Optional: Networks to join
    - proxy-tier
    - shared-internal

  shared_resources:                      # Optional: Shared resources
    postgres:
      host: shared-postgres
      port: 5432
      database: myapp_db
      user: myapp
      network: shared-internal

    redis:
      host: shared-redis
      port: 6379
      network: shared-internal
```

## Environment Variables

Shipwright supports multiple methods for environment configuration (in priority order):

1. **Existing `.env`**: Used as-is if found
2. **`.env.example` or `.env.template`**: Copied to `.env`
3. **Config `environment` section**: Created from `.shipwright.yml`
4. **Auto-generated defaults**: Minimal `.env` with common variables

See [docs/ENVIRONMENT_CONFIGURATION.md](docs/ENVIRONMENT_CONFIGURATION.md) for details.

## Use Cases

### Single Project VPS

Perfect for simple deployments:

```yaml
infrastructure:
  strategy: standalone
  auto_detect: false
```

### Multi-Project VPS

Shipwright excels at managing multiple projects:

```yaml
infrastructure:
  strategy: compose
  auto_detect: true

  # Automatically detects:
  # - Existing projects in ~/apps/
  # - Shared databases
  # - Proxy networks
  # - Deployment directories
```

### Existing Infrastructure

Works with your current setup:

```yaml
infrastructure:
  auto_detect: true

  proxy:
    type: caddy
    container_name: caddy-proxy
    auto_update: true

  shared_resources:
    postgres:
      host: shared-postgres
      database: myapp_db
```

## Troubleshooting

### Agent not starting

```bash
# Check logs
sudo journalctl -u shipwright-agent -n 50 --no-pager

# Verify Docker is running
sudo systemctl status docker

# Check agent permissions
ls -l /usr/local/bin/shipwright-agent
```

### Deployment failures

```bash
# View deployment logs
shipwright logs myapp --follow

# Check container status
docker ps -a --filter "name=myapp"

# View container logs
docker logs myapp-backend
```

### Permission errors

Shipwright automatically fixes file ownership, but if you encounter issues:

```bash
# Check deployment directory ownership
ls -ld ~/apps/myapp

# Manually fix ownership
sudo chown -R $(whoami):$(whoami) ~/apps/myapp
```

### Environment variable issues

```bash
# Verify .env file location
find ~/apps/myapp -name ".env"

# Check .env content
cat ~/apps/myapp/.env

# Verify containers see environment
docker exec myapp-backend env | grep DATABASE
```

## Contributing

We welcome contributions! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

### Development Setup

```bash
# Clone repository
git clone https://github.com/tinomupezeni/shipwright.git
cd shipwright

# Build all components
cargo build

# Run tests
cargo test

# Run agent locally
RUST_LOG=debug cargo run --package shipwright-agent

# Run CLI
cargo run --package shipwright-cli -- --help
```

### Project Structure

```
shipwright/
├── agent/          # Deployment agent (runs on VPS)
├── cli/            # Command-line interface
├── common/         # Shared types and config
├── docs/           # Documentation
├── examples/       # Example configurations
└── tests/          # Integration tests
```

## Roadmap

- [x] Infrastructure auto-detection
- [x] Smart deployment strategies
- [x] Environment file auto-generation
- [x] File ownership auto-fixing
- [x] Proxy integration (Caddy, Nginx, Traefik)
- [x] Shared resource support
- [x] Docker Compose deployment
- [x] Smoke testing framework
- [ ] Health check monitoring (continuous)
- [ ] Rollback capabilities
- [ ] Blue-green deployments
- [ ] Kubernetes support
- [ ] GitHub Actions integration
- [ ] GitLab CI integration
- [ ] Deployment analytics dashboard

## License

MIT License - see [LICENSE](LICENSE) file for details.

## Support

- 📖 [Documentation](docs/)
- 🐛 [Issue Tracker](https://github.com/tinomupezeni/shipwright/issues)
- 💬 [Discussions](https://github.com/tinomupezeni/shipwright/discussions)

## Acknowledgments

Built with:
- [Rust](https://www.rust-lang.org/) - Systems programming language
- [Tokio](https://tokio.rs/) - Async runtime
- [Bollard](https://github.com/fussybeaver/bollard) - Docker API client
- [Axum](https://github.com/tokio-rs/axum) - Web framework

---

**Made with ❤️ for developers who manage real-world VPS deployments**
