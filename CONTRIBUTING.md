# Contributing to Shipwright

Thank you for your interest in contributing to Shipwright! This document provides guidelines and instructions for contributing.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [How to Contribute](#how-to-contribute)
- [Coding Standards](#coding-standards)
- [Testing](#testing)
- [Pull Request Process](#pull-request-process)
- [Project Structure](#project-structure)
- [Areas for Contribution](#areas-for-contribution)

## Code of Conduct

By participating in this project, you agree to:
- Be respectful and inclusive
- Accept constructive criticism gracefully
- Focus on what's best for the community
- Show empathy towards other community members

## Getting Started

### Prerequisites

- Rust 1.70 or higher
- Docker and Docker Compose
- Git
- A Linux machine or VM for testing (Shipwright is Linux-focused)

### Development Setup

1. **Fork and clone the repository**

```bash
git clone https://github.com/YOUR_USERNAME/shipwright.git
cd shipwright
```

2. **Build the project**

```bash
cargo build
```

3. **Run tests**

```bash
cargo test
```

4. **Run the agent locally**

```bash
RUST_LOG=debug cargo run --package shipwright-agent
```

5. **Run the CLI**

```bash
cargo run --package shipwright-cli -- --help
```

## How to Contribute

### Reporting Bugs

Before creating a bug report:
1. Check existing issues to avoid duplicates
2. Collect the following information:
   - OS and version
   - Rust version (`rustc --version`)
   - Docker version (`docker --version`)
   - Agent logs (`journalctl -u shipwright-agent -n 100`)
   - Relevant configuration files

Create an issue with:
- Clear, descriptive title
- Steps to reproduce
- Expected vs actual behavior
- Logs and error messages
- Configuration (sanitized)

### Suggesting Features

Feature requests should include:
- Clear use case description
- Why existing features don't address this
- Proposed solution or API
- Alternative solutions considered
- Impact on existing functionality

### Contributing Code

1. **Find an issue to work on**
   - Check issues labeled `good first issue` or `help wanted`
   - Comment on the issue to claim it
   - Wait for maintainer confirmation

2. **Create a feature branch**

```bash
git checkout -b feature/your-feature-name
# or
git checkout -b fix/issue-number-description
```

3. **Make your changes**
   - Follow our coding standards
   - Add tests for new functionality
   - Update documentation
   - Keep commits atomic and well-described

4. **Test your changes**

```bash
# Run unit tests
cargo test

# Run clippy
cargo clippy --all-targets --all-features

# Format code
cargo fmt

# Test locally with a real deployment
cargo build --release
```

5. **Commit your changes**

```bash
git add .
git commit -m "feat: add smoke testing for database connectivity

- Implement database connection validation
- Add timeout handling
- Include retry logic for transient failures

Closes #123"
```

### Commit Message Convention

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <subject>

<body>

<footer>
```

**Types:**
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `style`: Code style changes (formatting, etc.)
- `refactor`: Code refactoring
- `test`: Adding or updating tests
- `chore`: Maintenance tasks

**Examples:**

```
feat(agent): add support for Traefik proxy detection

fix(cli): resolve connection timeout on slow networks

docs: update deployment configuration examples

test(smoke): add network connectivity validation tests
```

## Coding Standards

### Rust Style

- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Run `cargo fmt` before committing
- Fix all `cargo clippy` warnings
- Use meaningful variable names
- Add doc comments for public APIs

### Code Organization

```rust
// Good: Clear, documented, single responsibility
/// Detects existing reverse proxy infrastructure.
///
/// Searches for running containers matching known proxy images
/// (Caddy, Nginx, Traefik) and returns the detected proxy type
/// and container name.
///
/// # Returns
///
/// * `Ok(Some((proxy_type, container_name)))` if proxy detected
/// * `Ok(None)` if no proxy found
/// * `Err(...)` on Docker API errors
pub async fn detect_proxy(docker: &Docker) -> Result<Option<(String, String)>> {
    // Implementation
}

// Bad: Undocumented, unclear purpose
pub async fn check_stuff(d: &Docker) -> Result<Option<(String, String)>> {
    // Implementation
}
```

### Error Handling

- Use `anyhow::Result` for application errors
- Provide context with `.context()`
- Log errors appropriately
- Never use `unwrap()` or `expect()` in production code

```rust
// Good
let config = load_config(&config_path)
    .context(format!("Failed to load config from {}", config_path.display()))?;

// Bad
let config = load_config(&config_path).unwrap();
```

### Logging

Use appropriate log levels:

```rust
use tracing::{debug, info, warn, error};

debug!("Loading configuration from {}", path);
info!("✓ Deployment completed successfully");
warn!("No .env file found, using defaults");
error!("Failed to connect to Docker: {}", err);
```

### Testing

- Write unit tests for business logic
- Write integration tests for features
- Test error cases, not just happy paths
- Use descriptive test names

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_proxy_finds_caddy() {
        // Arrange
        let mock_containers = vec![/* ... */];

        // Act
        let result = detect_proxy_in_containers(&mock_containers);

        // Assert
        assert_eq!(result, Some(("caddy".to_string(), "caddy-proxy".to_string())));
    }

    #[test]
    fn test_detect_proxy_returns_none_when_no_proxy() {
        let result = detect_proxy_in_containers(&[]);
        assert_eq!(result, None);
    }
}
```

## Testing

### Running Tests

```bash
# All tests
cargo test

# Specific package
cargo test --package shipwright-agent

# Specific test
cargo test test_detect_proxy

# With output
cargo test -- --nocapture

# Integration tests only
cargo test --test '*'
```

### Test Coverage

We aim for >80% coverage on critical paths:
- Infrastructure detection
- Deployment strategies
- Environment file generation
- Error handling

### Manual Testing

For changes affecting deployment:

1. Set up a test VPS (DigitalOcean, Linode, etc.)
2. Install your branch of Shipwright
3. Deploy a real application
4. Verify all functionality works
5. Check logs for errors/warnings

## Pull Request Process

1. **Update documentation**
   - Update README.md if adding features
   - Add/update doc comments
   - Update CHANGELOG.md

2. **Ensure quality**
   - All tests pass
   - No clippy warnings
   - Code is formatted
   - No merge conflicts

3. **Create the PR**
   - Use a clear, descriptive title
   - Reference related issues
   - Describe what changed and why
   - Include testing steps
   - Add screenshots/logs if relevant

4. **PR Description Template**

```markdown
## Description
Brief description of changes

## Related Issues
Closes #123
Related to #456

## Changes Made
- Added X feature
- Fixed Y bug
- Refactored Z

## Testing
- [ ] Unit tests added/updated
- [ ] Integration tests pass
- [ ] Manually tested on VPS
- [ ] Documentation updated

## Screenshots/Logs
(if applicable)

## Checklist
- [ ] Code follows project style
- [ ] Tests pass
- [ ] Documentation updated
- [ ] CHANGELOG updated
```

5. **Code Review**
   - Address reviewer feedback
   - Push updates to your branch
   - Re-request review when ready

6. **Merge**
   - Maintainers will merge approved PRs
   - Delete your branch after merge

## Project Structure

```
shipwright/
├── agent/                      # Deployment agent (runs on VPS)
│   ├── src/
│   │   ├── main.rs            # Agent entry point
│   │   ├── pipeline/          # Build and deployment pipeline
│   │   │   ├── build.rs       # Build orchestration
│   │   │   └── deploy.rs      # Deployment strategies
│   │   ├── infrastructure/    # Infrastructure detection
│   │   │   ├── detector.rs    # Auto-detection logic
│   │   │   └── adapters.rs    # Proxy adapters
│   │   └── webhooks/          # GitHub webhook handling
│   └── Cargo.toml
│
├── cli/                        # Command-line interface
│   ├── src/
│   │   ├── main.rs            # CLI entry point
│   │   └── commands/          # CLI commands
│   └── Cargo.toml
│
├── common/                     # Shared code
│   ├── src/
│   │   ├── config.rs          # Configuration structs
│   │   └── types.rs           # Shared types
│   └── Cargo.toml
│
├── docs/                       # Documentation
│   ├── ARCHITECTURE.md        # Technical architecture
│   ├── SMOKE_TESTING.md       # Smoke testing guide
│   └── ENVIRONMENT_CONFIGURATION.md
│
├── examples/                   # Example configurations
│   ├── django-app/
│   ├── nodejs-app/
│   └── multi-service/
│
├── tests/                      # Integration tests
│   └── integration/
│
├── Cargo.toml                  # Workspace config
├── README.md                   # Main documentation
└── CONTRIBUTING.md             # This file
```

### Key Modules

- **agent/pipeline/build.rs**: Handles git cloning, docker builds, environment setup
- **agent/pipeline/deploy.rs**: Deployment strategies (standalone, compose, hybrid)
- **agent/infrastructure/detector.rs**: Auto-detects proxies, databases, networks
- **agent/infrastructure/adapters.rs**: Proxy configuration (Caddy, Nginx, Traefik)
- **common/config.rs**: Configuration schema and parsing

## Areas for Contribution

### High Priority

1. **Smoke Testing Framework** ⭐
   - Environment variable validation
   - Database connectivity tests
   - Service health checks
   - Network routing validation

2. **Rollback Capabilities**
   - Save deployment state before changes
   - Quick rollback command
   - Automatic rollback on health check failure

3. **Health Check Monitoring**
   - Continuous health monitoring
   - Alert on failures
   - Integration with notification systems

### Medium Priority

4. **Blue-Green Deployments**
   - Zero-downtime deployment strategy
   - Traffic switching
   - State management

5. **Additional Proxy Support**
   - HAProxy
   - Envoy
   - Kong

6. **Database Migration Support**
   - Detect and run migrations
   - Backup before migrations
   - Migration rollback

### Good First Issues

- Add support for additional frameworks (Rails, Laravel, etc.)
- Improve error messages
- Add more example configurations
- Improve logging output formatting
- Add shell completion scripts
- Write tutorial documentation

### Documentation

- Architecture diagrams
- Video tutorials
- Blog posts about use cases
- Translation to other languages

## Getting Help

- **Questions**: Open a [Discussion](https://github.com/tinomupezeni/shipwright/discussions)
- **Bugs**: Create an [Issue](https://github.com/tinomupezeni/shipwright/issues)
- **Chat**: Join our community (link TBD)

## Recognition

Contributors will be:
- Listed in CONTRIBUTORS.md
- Mentioned in release notes
- Credited in documentation

Thank you for contributing to Shipwright! 🚢
