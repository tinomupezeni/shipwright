# Environment Variables in Shipwright

## Quick Start: Band-aid Fix (Current)

Your agent now supports injecting environment variables from `.shipwright.yml` into your `.env` file during deployment.

### How It Works

1. **Define variables in `.shipwright.yml`** (in your project repo)
2. **Agent merges them into `.env`** on every deployment
3. **Variables persist across git pull/clone** operations

### Example Configuration

In your project's `.shipwright.yml`:

```yaml
version: 1
project:
  name: TESE-MARKET---BFF-ARCHITECTURE

build:
  compose_file: docker-compose.vps.yml

  # Add your environment variables here
  environment:
    # GitHub Container Registry
    GITHUB_REPOSITORY_OWNER: "winstontino"

    # Application Secrets
    JWT_SECRET: "your-secret-jwt-key-here"

    # Database Configuration
    DATABASE_URL: "postgresql://user:pass@postgres:5432/dbname"

    # API Keys
    STRIPE_API_KEY: "sk_test_..."
    SENDGRID_API_KEY: "SG...."

    # Other Variables
    NODE_ENV: "production"
    APP_URL: "https://app.tesemarket.com"

deploy:
  type: docker-compose
  vps:
    host: 159.198.42.231
    user: winstontino
    ssh_key: ~/.ssh/id_rsa
```

### What Happens During Deployment

```
1. Agent clones/pulls your repository
   └─> This might wipe .env file

2. Agent reads .shipwright.yml
   └─> Extracts build.environment variables

3. Agent reads existing .env (if any)
   └─> Preserves any variables already there

4. Agent merges config variables into .env
   └─> Config variables take precedence
   └─> Writes merged .env file

5. Agent validates all docker-compose vars are set
   └─> Shows clear error if any are missing

6. Deployment proceeds with complete .env
```

### Environment Variable Priority

When merging, priority is:

1. **Highest**: Variables in `.shipwright.yml` (your config)
2. **Lowest**: Variables already in `.env` (from git)

This means:
- ✅ Config always wins (you control it)
- ✅ Can override committed .env defaults
- ✅ Can add secrets not in git

### Security Considerations

**⚠️ IMPORTANT**: `.shipwright.yml` is typically committed to git.

**For public repos:**
```yaml
# ❌ DO NOT DO THIS in public repos
environment:
  JWT_SECRET: "my-secret-123"  # Exposed to everyone!
```

**For private repos:**
```yaml
# ✅ OK for private repos
environment:
  JWT_SECRET: "my-secret-123"  # Only team can see
```

**Best practice (until secret store is ready):**
```yaml
# Option 1: Use .shipwright.local.yml (gitignored)
# Add to .gitignore:
# .shipwright.local.yml

# Option 2: Use environment variable substitution
# Set on your machine: export JWT_SECRET=...
# Then in .shipwright.yml:
environment:
  JWT_SECRET: "${JWT_SECRET}"  # Reads from your shell

# Option 3: Wait for secret store (coming soon)
```

### Validation

The agent now **validates environment variables** before deployment:

**Before (old behavior):**
```
The GITHUB_REPOSITORY_OWNER variable is not set. Defaulting to a blank string.
Pulling auth-api (ghcr.io//tese-auth-api:latest)...
invalid reference format
```

**After (new behavior):**
```
❌ Environment variable validation failed:

  • GITHUB_REPOSITORY_OWNER is not set
    Required by: auth-api
  • JWT_SECRET is not set
    Required by: auth-api

📝 Please update your .env file at: /home/winstontino/apps/PROJECT/.env

You can:
  1. Add the missing variables to your .env file
  2. Check .env.example for reference values
  3. Review your docker-compose file for required variables
```

### Troubleshooting

**Problem: Variables still missing after deployment**

Check the agent logs:
```bash
ssh user@vps 'sudo journalctl -u shipwright-agent -n 50 | grep -A5 "environment"'
```

Look for:
```
✓ Merged 5 environment variables from config into .env
```

**Problem: Can't find .shipwright.yml**

The file must be in your **project root** (same level as docker-compose.yml):
```
your-project/
  ├── .shipwright.yml          ← Here
  ├── docker-compose.vps.yml
  ├── docker-compose.yml
  └── .env
```

**Problem: Variables are empty/wrong**

Check your .shipwright.yml syntax:
```yaml
# ✅ Correct
environment:
  KEY: "value"

# ❌ Wrong (missing quotes for special chars)
environment:
  KEY: value-with-dashes

# ✅ Correct (quotes for safety)
environment:
  KEY: "value-with-dashes"
```

## Future: Proper Secret Store (Coming Soon)

The current solution is a **band-aid fix**. The proper solution is in development:

### Secret Store Features

```bash
# Set secrets via CLI (never in git)
shipwright secrets set JWT_SECRET my-secret-value
shipwright secrets set GITHUB_REPOSITORY_OWNER winstontino

# List secrets (names only, not values)
shipwright secrets list

# Secrets automatically backed up to:
# 1. Agent database (encrypted)
# 2. VPS file backup
# 3. Your local machine
# 4. Optional: Git (encrypted)
```

### Migration Path

When secret store is ready:

```bash
# Automatic migration tool
shipwright migrate-secrets

# It will:
# 1. Read environment from .shipwright.yml
# 2. Upload to secret store (encrypted)
# 3. Remove from .shipwright.yml
# 4. Commit changes
```

Your `.shipwright.yml` will become:
```yaml
version: 1
project:
  name: TESE-MARKET---BFF-ARCHITECTURE

build:
  compose_file: docker-compose.vps.yml
  # No more environment section!
  # Secrets managed by agent

deploy:
  type: docker-compose
  vps:
    host: 159.198.42.231
    user: winstontino
    ssh_key: ~/.ssh/id_rsa
```

## Related Documentation

- [Secret Management Design](./SECRET_MANAGEMENT_DESIGN.md) - Full architecture
- [Updating Agent](./UPDATING_AGENT.md) - How to deploy agent updates
- [Smoke Testing](./SMOKE_TESTING.md) - Validation framework

## Questions?

- **Q: Should I commit .shipwright.yml with secrets?**
  - A: Only if using a **private repo**. For public repos, wait for secret store.

- **Q: What about .env.example?**
  - A: Keep it! The agent will use it as a template if .env doesn't exist.

- **Q: Can I mix both methods?**
  - A: Yes! Config variables will override .env file variables.

- **Q: When will secret store be ready?**
  - A: Estimated 2-4 weeks. See [SECRET_MANAGEMENT_DESIGN.md](./SECRET_MANAGEMENT_DESIGN.md) for timeline.
