# Secret Management

Shipwright includes a built-in encrypted secret storage system that keeps your sensitive configuration secure.

## Quick Start

```bash
# Set a secret
shipwright secrets set DATABASE_PASSWORD

# List all secrets
shipwright secrets list

# Export secrets as .env format
shipwright secrets export > secrets.env

# Get a specific secret value
shipwright secrets get DATABASE_PASSWORD --show
```

## Features

- **AES-256-GCM Encryption**: Industry-standard encryption for all secret values
- **Audit Logging**: Track when secrets are created, updated, accessed, or deleted
- **No Secrets in Git**: Secrets are stored encrypted on your VPS, never in your repository
- **Easy Migration**: Export/import secrets between environments
- **CLI Integration**: Manage secrets without SSH access to your VPS

## How It Works

1. **Encrypted Storage**: Secrets are encrypted using AES-256-GCM before being stored in the agent's database on your VPS
2. **Key Derivation**: Encryption keys are derived from your project name and agent installation ID using PBKDF2-SHA256
3. **Secure API**: The agent exposes a REST API (Secret Management Protocol v1) for managing secrets
4. **CLI Commands**: The Shipwright CLI communicates with the agent to set/get secrets

## Architecture

```
┌─────────────────┐
│   Your Machine  │
│                 │
│  shipwright CLI │
│  secrets set    │
└────────┬────────┘
         │ HTTPS
         ▼
┌─────────────────────┐
│   VPS              │
│                    │
│  shipwright-agent  │
│  ├─ secrets.db     │  ← Encrypted SQLite database
│  │  (AES-256-GCM)  │
│  └─ API :17670     │  ← REST API for secret management
└─────────────────────┘
```

## Commands

### Set a Secret

```bash
# Interactive prompt for value
shipwright secrets set MY_SECRET

# Provide value directly
shipwright secrets set MY_SECRET --value "secret-value"

# Add tags for organization
shipwright secrets set API_KEY --value "key123" --tags production,api
```

### List Secrets

```bash
# List secret names only
shipwright secrets list

# Show metadata (created, updated times)
shipwright secrets list --with-metadata
```

### Get a Secret

```bash
# Show secret details (value hidden)
shipwright secrets get MY_SECRET

# Show the actual value
shipwright secrets get MY_SECRET --show
```

### Export Secrets

```bash
# Export as .env format (writes to stdout)
shipwright secrets export

# Save to file
shipwright secrets export > production.env
```

### Delete a Secret

```bash
# Delete with confirmation
shipwright secrets delete MY_SECRET

# Delete without confirmation
shipwright secrets delete MY_SECRET --force
```

## Integration with Deployment

Secrets are automatically available to your deployed applications through environment variables.

### Example Workflow

1. **Set your secrets:**
   ```bash
   shipwright secrets set DATABASE_PASSWORD
   shipwright secrets set JWT_SECRET
   shipwright secrets set API_KEY
   ```

2. **Create .env file on VPS** (one time):
   The secrets are stored in the agent, but you need a `.env` file in your deployment directory that references them:

   ```bash
   # SSH to VPS and create .env
   ssh user@your-vps.com
   cd ~/apps/your-project

   # Create .env file with the secrets
   cat > .env << EOF
   DATABASE_PASSWORD=the-password-you-set
   JWT_SECRET=the-secret-you-set
   API_KEY=the-key-you-set
   EOF
   ```

3. **Deploy your project:**
   ```bash
   shipwright up
   ```

Docker Compose will read the `.env` file and inject the environment variables into your containers.

## Security Best Practices

1. **Never commit secrets to Git**: Always use the secret storage for sensitive values
2. **Use strong passwords**: The encryption is only as strong as your secrets
3. **Rotate secrets regularly**: Use `shipwright secrets set` to update values
4. **Audit access**: Check audit logs with `shipwright secrets audit` (coming soon)
5. **Backup secrets**: Periodically export secrets and store them securely offline

## Troubleshooting

### "Failed to set secret"

Make sure:
- The Shipwright agent is running on your VPS
- Port 17670 is open for HTTPS traffic
- Your `.shipwright.yml` has the correct VPS host configured

### "Secret not found"

Secrets are project-specific. Make sure you're in the correct project directory with a `.shipwright.yml` file.

## Technical Details

- **Encryption**: AES-256-GCM (Galois/Counter Mode)
- **Key Derivation**: PBKDF2-SHA256 with 100,000 iterations
- **Storage**: SQLite database on VPS at `/var/lib/shipwright/shipwright-agent.db`
- **API Protocol**: Secret Management Protocol v1 (SMP/v1)
- **Transport**: HTTPS (ports 17670 for HTTP API, 17671 for WebSocket)

## Future Features

- [ ] Automatic secret rotation
- [ ] Git-based secret backup (encrypted)
- [ ] Multi-environment support (dev, staging, prod)
- [ ] Secret templates for common frameworks
- [ ] Integration with external secret managers (AWS Secrets Manager, Vault)
