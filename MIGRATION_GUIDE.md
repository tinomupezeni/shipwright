# Migration Guide: Adding Shipwright to Existing VPS Setup

This guide helps you safely integrate Shipwright into a VPS that already has deployed applications.

## Table of Contents

1. [Pre-Migration Checklist](#pre-migration-checklist)
2. [Scenario 1: Multi-Project VPS with Caddy](#scenario-1-multi-project-vps-with-caddy)
3. [Scenario 2: Single Project Migration](#scenario-2-single-project-migration)
4. [Rollback Plan](#rollback-plan)
5. [Testing Strategy](#testing-strategy)

---

## Pre-Migration Checklist

Before adding Shipwright to your existing VPS:

### 1. Document Current Setup

```bash
# On your VPS, run:
ssh your-user@your-vps << 'EOF'
  echo "=== Docker Containers ==="
  docker ps --format "table {{.Names}}\t{{.Image}}\t{{.Status}}"

  echo -e "\n=== Docker Networks ==="
  docker network ls

  echo -e "\n=== Directory Structure ==="
  ls -la ~/apps/

  echo -e "\n=== Proxy Config ==="
  docker exec caddy-proxy cat /etc/caddy/Caddyfile || echo "No Caddy"
  docker exec nginx-proxy cat /etc/nginx/nginx.conf || echo "No Nginx"
EOF
```

Save this output for reference.

### 2. Backup Everything

```bash
# Backup Caddy/Nginx config
docker cp caddy-proxy:/etc/caddy/Caddyfile ./backup-Caddyfile

# Backup database
docker exec shared-postgres pg_dumpall -U postgres > ./backup-db.sql

# Backup project directories
tar -czf ~/apps-backup.tar.gz ~/apps/
```

### 3. Install Shipwright Agent (Non-Destructive)

```bash
# On local machine
cd shipwright
shipwright setup
```

The agent installation is **read-only** - it won't modify existing containers or configurations.

---

## Scenario 1: Multi-Project VPS with Caddy

**Starting Point**: VPS like your `159.198.42.231` with:
- Multiple projects in `~/apps/`
- Caddy proxy with existing Caddyfile
- Shared resources (postgres, redis)
- Custom networks (`proxy-tier`, `core_shared-internal`)

### Step-by-Step Migration

#### Step 1: Test with One Project First

Choose a non-critical project for testing (e.g., a dev/staging app).

Create `.shipwright.yml` in the project repo:

```yaml
version: 1

project:
  name: test-project

infrastructure:
  strategy: compose
  deploy_dir: ~/apps/test-project
  auto_detect: true  # Let Shipwright learn your setup

  proxy:
    type: caddy
    container_name: caddy-proxy
    auto_update: false  # IMPORTANT: Manual control first

deploy:
  type: docker-compose
  vps:
    host: 159.198.42.231
    user: winstontino
    ssh_key: ~/.ssh/id_rsa
```

#### Step 2: Dry Run

```bash
cd test-project
shipwright register
shipwright up --dry-run
```

Review what Shipwright plans to do. It should output:
- Detected infrastructure
- Networks it will join
- Deployment strategy
- Files it will modify

#### Step 3: Manual First Deployment

```bash
# Deploy with Shipwright
shipwright up

# Check logs
shipwright logs

# Verify containers
ssh winstontino@159.198.42.231 "docker ps | grep test-project"

# Test the application
curl https://test-project.yourdomain.com
```

#### Step 4: Enable Auto-Updates (Optional)

After successful manual deployment, enable automation:

```yaml
infrastructure:
  proxy:
    auto_update: true  # Now safe to enable

# Commit and push
git add .shipwright.yml
git commit -m "feat: add Shipwright automation"
git push
```

Shipwright will now handle future deployments automatically via webhook.

#### Step 5: Migrate Other Projects

Repeat for each project, one at a time. Wait 24-48 hours between migrations to ensure stability.

---

## Scenario 2: Single Project Migration

**Starting Point**: One app deployed manually with docker-compose.

### Quick Migration

1. **Add `.shipwright.yml` to your repo**:

```yaml
version: 1

project:
  name: myapp

build:
  compose_file: docker-compose.yml  # Your existing file

infrastructure:
  strategy: compose
  auto_detect: true

deploy:
  type: docker-compose
  vps:
    host: your-vps-ip
    user: your-user
    ssh_key: ~/.ssh/id_rsa
```

2. **Register once**:

```bash
shipwright register
```

3. **Future deploys are automatic**:

```bash
git push origin main
# Shipwright takes over from here
```

---

## Rollback Plan

If something goes wrong, you can quickly rollback:

### Immediate Rollback (Container-Level)

```bash
# On VPS: Restore previous container
docker stop new-container
docker start old-container

# Or restore from backup
docker-compose -f docker-compose.yml up -d
```

### Full Rollback (Remove Shipwright)

```bash
# 1. Remove GitHub webhook
# Go to: https://github.com/your-org/your-repo/settings/hooks
# Delete the Shipwright webhook

# 2. Stop Shipwright agent
ssh your-user@your-vps "docker stop shipwright-agent && docker rm shipwright-agent"

# 3. Restore Caddyfile (if modified)
docker cp ./backup-Caddyfile caddy-proxy:/etc/caddy/Caddyfile
docker exec caddy-proxy caddy reload

# 4. Resume manual deployment
cd ~/apps/your-project
git pull
docker-compose up -d
```

Your system is back to pre-Shipwright state.

---

## Testing Strategy

### 1. Infrastructure Detection Test

```bash
# SSH into VPS and run agent manually with detection
cd ~/shipwright/agent
cargo run
# Check logs for detected infrastructure
```

Expected output:
```
🔍 Detecting existing infrastructure...
   ✓ Detected caddy proxy: caddy-proxy
   ✓ Found 5 Docker networks
   ✓ Found shared PostgreSQL
   ✓ Found shared Redis
   ✓ Multi-project setup detected (7 projects)
```

### 2. Dry-Run Test

```bash
# On local machine
cd your-project
shipwright up --dry-run
```

Verify:
- ✅ Correct deployment directory
- ✅ Right docker-compose file detected
- ✅ Proper networks listed
- ✅ No unintended changes

### 3. Isolated Test Deployment

Deploy to a test subdomain first:

```yaml
deploy:
  vps:
    domain: test.yourdomain.com
```

This isolates testing from production traffic.

### 4. Health Check Validation

```yaml
deploy:
  health:
    http:
      path: /health
      expect: 200
      timeout: 30s
```

Shipwright won't mark deployment successful unless health checks pass.

---

## Common Migration Issues

### Issue 1: Network Conflicts

**Symptom**: Containers can't communicate with existing services.

**Solution**: Explicitly list networks:

```yaml
infrastructure:
  networks:
    - proxy-tier
    - core_shared-internal
    - your-project-network
```

### Issue 2: Port Conflicts

**Symptom**: "Port already in use" errors.

**Solution**: Shipwright agent uses dynamic port discovery. Check logs:

```bash
ssh your-user@your-vps "cat /etc/shipwright/agent.env"
```

Update your `.shipwright.yml` with correct ports if needed.

### Issue 3: Caddyfile Format Issues

**Symptom**: Caddy reload fails after Shipwright update.

**Solution**:
1. Set `auto_update: false`
2. Review Shipwright's proposed changes:
   ```bash
   docker exec caddy-proxy cat /etc/caddy/Caddyfile
   ```
3. Manually adjust if needed
4. File issue at github.com/your-org/shipwright for improvements

### Issue 4: Existing Containers Not Found

**Symptom**: Shipwright creates duplicate containers.

**Solution**: Ensure container names match between docker-compose and config:

```yaml
# In docker-compose.yml
services:
  backend:
    container_name: myapp-backend  # Must match Shipwright config

# In .shipwright.yml
deploy:
  vps:
    services:
      - name: myapp-backend  # Same name
```

---

## Gradual Rollout Plan

### Week 1: Monitoring Only

```yaml
infrastructure:
  proxy:
    auto_update: false
```

Deploy manually, but let Shipwright monitor and collect metrics.

### Week 2: Automated Deployment

```yaml
infrastructure:
  proxy:
    auto_update: false  # Still manual proxy updates
```

Enable webhook deployment, but verify proxy config manually.

### Week 3+: Full Automation

```yaml
infrastructure:
  proxy:
    auto_update: true
```

Enable full automation after confidence is built.

---

## Verification Checklist

After migration, verify:

- [ ] All containers running: `docker ps`
- [ ] Networks intact: `docker network ls`
- [ ] Proxy routing works: `curl https://yourdomain.com`
- [ ] Database connections active: Check app logs
- [ ] SSL certificates valid: `curl -vI https://yourdomain.com`
- [ ] Logs accessible: `shipwright logs`
- [ ] Metrics flowing: `shipwright status`
- [ ] Webhook active: Check GitHub webhook deliveries
- [ ] Health checks passing: `shipwright status`

---

## Best Practices for Safe Migration

1. **One project at a time**: Don't migrate everything at once
2. **Test on non-production first**: Use staging/dev apps
3. **Keep backups**: Maintain restore points
4. **Monitor closely**: Watch logs for 24-48 hours after migration
5. **Document changes**: Keep notes on what you modified
6. **Have rollback ready**: Test rollback procedure before migration
7. **Use gradual rollout**: Don't enable all features immediately
8. **Communicate**: Inform team about deployment changes

---

## Getting Help

If you encounter issues:

1. **Check agent logs**:
   ```bash
   ssh your-user@your-vps "docker logs shipwright-agent"
   ```

2. **Run diagnostics**:
   ```bash
   shipwright diagnose  # Coming soon
   ```

3. **Dry-run analysis**:
   ```bash
   shipwright up --dry-run --verbose
   ```

4. **Community support**:
   - GitHub Issues: github.com/your-org/shipwright/issues
   - Discord: discord.gg/shipwright

---

## Success Stories

### Example: Multi-Project VPS Migration

**Before**: 7 manually deployed projects, 30+ manual deploy minutes per project

**After**: All automated, ~5 minutes per deploy, zero-downtime updates

**Migration time**: 2 weeks (gradual rollout)

**Issues encountered**: 2 (network config, Caddyfile format)

**Rollbacks needed**: 0

---

## Next Steps

After successful migration:

- Enable monitoring and alerts
- Set up deployment notifications (Slack/Discord)
- Configure automatic rollbacks on failure
- Explore preview environments for PRs
- Consider Shipwright Cloud for dashboard

---

**Questions? Issues?** Open a GitHub issue or join our Discord community.
