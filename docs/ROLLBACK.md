# Rollback System

Shipwright includes a hybrid rollback system that automatically recovers from deployment failures by rolling back to the last successful deployment.

## Features

- **Automatic Rollback**: Automatically rolls back deployments when smoke tests fail
- **Hybrid Strategy**: Intelligently selects the best rollback approach based on service type
- **Fast Recovery**: Minimizes downtime with optimized rollback strategies
- **Deployment History**: Tracks all deployments with detailed snapshots
- **Real-time Notifications**: WebSocket updates for rollback progress

## Rollback Strategies

Shipwright uses three rollback strategies, automatically selected based on service characteristics:

### 1. Image Tagging (5-10 seconds)

**Best for**: Stateless services, microservices, APIs

**How it works**:
1. Before deployment: Tag current Docker images as `rollback-<timestamp>`
2. Deploy new version with `latest` tag
3. On failure: Re-tag rollback image as `latest` and restart containers

**Advantages**:
- Fastest rollback method (5-10 seconds)
- No data loss risk
- Works with existing images

**Example services**: REST APIs, stateless workers, caching layers

### 2. Git Commit (2-5 minutes)

**Best for**: Frontend applications, static sites

**How it works**:
1. Before deployment: Record current git commit SHA and branch
2. Deploy new version
3. On failure: Checkout previous commit, rebuild, and redeploy

**Advantages**:
- Full source code restoration
- Rebuild ensures consistency
- Works well with frontend build pipelines

**Example services**: React apps, Vue frontends, Next.js applications

### 3. Snapshot (30-60 seconds)

**Best for**: Stateful services with databases, services with migrations

**How it works**:
1. Before deployment: Create volume snapshots and database backups
2. Deploy new version
3. On failure: Restore volumes and database, restart containers

**Advantages**:
- Complete state preservation
- Database restoration included
- Handles schema migrations

**Example services**: Django/Rails apps with PostgreSQL, services with MongoDB

### 4. Hybrid (Auto-detect)

**Best for**: Multi-service applications (default)

Shipwright automatically selects the best strategy for each service:
- **Has database or migrations** → Snapshot strategy
- **Frontend service** (name contains "frontend", "web", "ui") → Git commit strategy
- **Stateless service** → Image tagging strategy

## Configuration

Add rollback configuration to your `.shipwright.yml`:

```yaml
version: 1

project:
  name: myapp

# ... build and deploy config ...

# Rollback configuration
rollback:
  # Enable automatic rollback (default: true)
  enabled: true

  # Rollback strategy: image-tagging, git-commit, snapshot, or hybrid (default: hybrid)
  strategy: hybrid

  # Maximum number of deployment snapshots to retain (default: 10)
  max_snapshots: 10

  # Automatically rollback on smoke test failure (default: true)
  auto_rollback_on_test_failure: true

  # Optional: Override strategy for specific services
  service_strategies:
    frontend: git-commit
    api: image-tagging
    database-service: snapshot
```

## How It Works

### Deployment Flow with Rollback

1. **Pre-Deployment**:
   - Create deployment snapshot using configured strategy
   - Store snapshot metadata in database
   - Notify via WebSocket: "Snapshot created"

2. **Deployment**:
   - Run pre-deployment smoke tests
   - Execute deployment (docker-compose up)
   - Update proxy configuration

3. **Post-Deployment**:
   - Run comprehensive smoke tests
   - Validate container health
   - Check environment variables
   - Test database connectivity

4. **On Failure**:
   - Detect smoke test failures
   - Notify via WebSocket: "Rollback started"
   - Execute rollback using appropriate strategy
   - Verify rolled-back deployment
   - Notify via WebSocket: "Rollback complete"

### Snapshot Storage

Snapshots are stored in:
- **Database**: `/var/lib/shipwright/shipwright-agent.db`
  - deployment_snapshots table
  - service_deployments table
  - rollback_events table

- **File System** (for snapshot strategy):
  - Volume snapshots: `/var/lib/shipwright/snapshots/{snapshot-id}/volumes/`
  - Database backups: `/var/lib/shipwright/snapshots/{snapshot-id}/`

## Manual Rollback

While rollback is automatic on smoke test failures, you can also trigger manual rollbacks:

```bash
# View rollback history
shipwright rollback list myapp

# Rollback to previous deployment
shipwright rollback previous myapp

# Rollback to specific snapshot
shipwright rollback to <snapshot-id> myapp

# View deployment snapshots
shipwright snapshots list myapp
```

## Rollback Events

All rollback operations are tracked in the database with:
- From/to snapshot IDs
- Rollback reason (smoke_test_failure, manual, health_check_failure)
- Failure details
- Rollback duration
- Success/failure status
- Performer (auto, cli, username)

View rollback history:
```bash
shipwright rollback history myapp
```

## Real-time Notifications

When using `shipwright watch`, you'll receive real-time rollback notifications:

```
📸 Creating deployment snapshot... (hybrid strategy)
✅ Snapshot created: abc123 (image-tagging)

🧪 Running post-deployment smoke tests...
✗ Container health check failed: api-service not responding

🔄 Smoke tests failed, initiating rollback...
⏱  Rolling back to previous deployment (snapshot: def456)
✅ Rollback completed successfully (8.3 seconds)
```

## Best Practices

### 1. Enable Smoke Tests

Rollback is most effective when combined with comprehensive smoke tests:

```yaml
smoke_tests:
  enabled: true
  fail_on_error: true
  categories:
    - pre_deployment
    - post_build
    - post_deployment
```

### 2. Retain Sufficient Snapshots

Keep enough snapshots to roll back multiple versions:

```yaml
rollback:
  max_snapshots: 10  # Last 10 deployments
```

### 3. Test Rollback Procedures

Periodically test manual rollback to ensure it works:

```bash
# Deploy a test version
shipwright up

# Manually rollback
shipwright rollback previous myapp

# Verify services are healthy
docker ps
```

### 4. Monitor Rollback Events

Review rollback history to identify problematic deployments:

```bash
shipwright rollback history myapp
```

### 5. Service-Specific Strategies

Override the hybrid strategy for services with special requirements:

```yaml
rollback:
  strategy: hybrid
  service_strategies:
    # Force snapshot for service with critical data
    payment-service: snapshot

    # Force image tagging for ultra-fast rollback
    cache-service: image-tagging
```

## Limitations

### Image Tagging Strategy
- Only works with Docker images
- Requires images to be available locally
- Cannot rollback configuration changes outside containers

### Git Commit Strategy
- Requires rebuild (2-5 minutes)
- Git repository must be accessible
- Build dependencies must be available

### Snapshot Strategy
- Requires disk space for volume snapshots
- Database restoration time depends on size
- May not work with external databases (not in Docker)

## Troubleshooting

### Rollback Failed: "No previous successful deployment found"

This means there are no previous snapshots to roll back to. This can happen on:
- First deployment of a project
- After database cleanup
- If snapshots were manually deleted

**Solution**: Deploy a known-good version first to create a baseline snapshot.

### Rollback Failed: "Snapshot verification failed"

The rollback snapshot is corrupted or missing.

**Solution**:
```bash
# List available snapshots
shipwright snapshots list myapp

# Rollback to a specific known-good snapshot
shipwright rollback to <snapshot-id> myapp
```

### Slow Rollback Performance

If rollbacks are taking longer than expected:

1. **Check strategy selection**:
   ```bash
   shipwright snapshots list myapp --show-strategy
   ```

2. **Consider using image-tagging** for faster rollback:
   ```yaml
   rollback:
     strategy: image-tagging  # Fastest
   ```

3. **Reduce volume snapshot size** by excluding unnecessary data

### Disk Space Issues

Snapshots consume disk space. Clean up old snapshots:

```bash
# Reduce max snapshots
shipwright config set rollback.max_snapshots 5

# Manually clean old snapshots
shipwright snapshots prune myapp --keep 5
```

## Architecture

The rollback system consists of:

### Core Components

1. **RollbackManager** (`agent/src/rollback/mod.rs`)
   - Coordinates rollback operations
   - Selects appropriate strategy
   - Manages snapshot lifecycle

2. **Rollback Strategies** (`agent/src/rollback/*_strategy.rs`)
   - ImageTaggingStrategy: Fast container rollback
   - GitCommitStrategy: Source-based rollback
   - SnapshotStrategy: Full state restoration

3. **Rollback Storage** (`agent/src/rollback/storage.rs`)
   - Database operations for snapshots
   - Rollback event tracking
   - Snapshot retrieval and management

### Database Schema

See `agent/src/db/migrations/V4__rollback_system.sql` for complete schema.

**Key tables**:
- `deployment_snapshots`: Snapshot metadata
- `service_deployments`: Per-service deployment state
- `rollback_events`: Audit trail of rollback operations

### Integration Points

- **Smoke Tests**: Triggers automatic rollback on failure
- **Deployment Pipeline**: Creates snapshots before deployment
- **WebSocket**: Real-time rollback notifications
- **CLI**: Manual rollback commands

## Future Enhancements

- [ ] Blue-green deployment strategy
- [ ] Canary rollback (gradual rollback)
- [ ] Rollback dry-run mode
- [ ] Automated rollback testing
- [ ] Rollback analytics dashboard
- [ ] Integration with external backup services
- [ ] Multi-region rollback coordination
