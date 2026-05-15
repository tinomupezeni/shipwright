# Shipwright Deployment Strategies

Shipwright supports multiple deployment strategies to work seamlessly with different infrastructure setups.

## Table of Contents

1. [Simple Single-Project Setup](#simple-single-project-setup)
2. [Multi-Project VPS with Caddy](#multi-project-vps-with-caddy)
3. [Multi-Project with Nginx](#multi-project-with-nginx)
4. [Existing Docker Compose Projects](#existing-docker-compose-projects)
5. [Shared Resources (Database, Redis)](#shared-resources)

---

## Simple Single-Project Setup

**Use case**: You have a VPS with a single application and no existing infrastructure.

### Configuration (`.shipwright.yml`)

```yaml
version: 1

project:
  name: myapp
  framework: auto

build:
  image: node:20-alpine
  steps:
    - npm ci
    - npm run build

deploy:
  type: docker
  vps:
    host: 192.168.1.100
    user: ubuntu
    ssh_key: ~/.ssh/id_rsa
  replicas: 1
  health:
    http:
      path: /health
      expect: 200
      timeout: 30s
```

### What Shipwright Does

1. Clones to `/home/ubuntu/apps/myapp`
2. Builds Docker image
3. Deploys standalone container
4. No proxy configuration needed (access via IP:port)

---

## Multi-Project VPS with Caddy

**Use case**: Your VPS hosts multiple applications behind Caddy reverse proxy (like the example with savens-blog, tese, mlms, etc.)

### Configuration (`.shipwright.yml`)

```yaml
version: 1

project:
  name: savens-blog
  framework: auto

build:
  compose_file: docker-compose.deploy.yml
  services:  # Optional: build only specific services
    - backend
    - frontend
    - admin

deploy:
  type: docker-compose
  vps:
    host: 159.198.42.231
    user: winstontino
    ssh_key: ~/.ssh/id_rsa
    domain: savens.restksolutions.co.zw
    services:
      - name: savens-frontend
        domain: savens.restksolutions.co.zw
        port: 80
        expose: true

      - name: savens-admin
        domain: admin.savens.restksolutions.co.zw
        port: 80
        expose: true

      - name: savens-backend
        domain: api.savens.restksolutions.co.zw
        port: 8000
        expose: true

    acme_email: admin@restksolutions.co.zw

# Infrastructure configuration
infrastructure:
  strategy: compose  # Forces docker-compose deployment
  deploy_dir: ~/apps/savens-blog  # Explicit deployment directory

  proxy:
    type: caddy
    container_name: caddy-proxy
    auto_update: true  # Automatically update Caddyfile

  networks:
    - proxy-tier  # Join existing proxy network
    - core_shared-internal  # Join shared resources network

  shared_resources:
    postgres:
      host: shared-postgres
      port: 5432
      database: savens_blog
      user: savens
      network: core_shared-internal

    redis:
      host: shared-redis
      port: 6379
      db: 0
      network: core_shared-internal

deploy:
  replicas: 1
  health:
    http:
      path: /health/
      expect: 200
      timeout: 30s
```

### What Shipwright Does

1. **Auto-detects** existing infrastructure:
   - Finds Caddy proxy running
   - Discovers `proxy-tier` and `core_shared-internal` networks
   - Detects shared Postgres and Redis

2. **Clones to** `~/apps/savens-blog` (respects existing structure)

3. **Builds** using `docker-compose.deploy.yml`

4. **Deploys** with `docker-compose up -d`

5. **Updates Caddyfile** automatically:
   ```
   # --- savens-frontend (Shipwright) ---
   savens.restksolutions.co.zw {
       import security
       reverse_proxy savens-frontend:80
   }

   # --- savens-admin (Shipwright) ---
   admin.savens.restksolutions.co.zw {
       import security
       reverse_proxy savens-admin:80
   }

   # --- savens-backend (Shipwright) ---
   api.savens.restksolutions.co.zw {
       import security
       import cors_handle
       reverse_proxy savens-backend:8000
   }
   ```

6. **Reloads Caddy** gracefully

---

## Multi-Project with Nginx

**Use case**: Using Nginx as reverse proxy instead of Caddy.

### Configuration

```yaml
version: 1

project:
  name: myapp
  framework: auto

infrastructure:
  strategy: compose

  proxy:
    type: nginx
    container_name: nginx-proxy
    auto_update: true

  deploy_dir: ~/apps/myapp

  networks:
    - nginx-network

deploy:
  type: docker-compose
  vps:
    host: 192.168.1.100
    user: ubuntu
    ssh_key: ~/.ssh/id_rsa
    domain: myapp.example.com
    services:
      - name: myapp-web
        domain: myapp.example.com
        port: 3000
        expose: true
```

### What Shipwright Does

1. Creates Nginx config file: `/etc/nginx/conf.d/myapp-web.conf`
2. Connects service to `nginx-network`
3. Reloads Nginx with `nginx -s reload`

---

## Existing Docker Compose Projects

**Use case**: Migrating an existing project that's already deployed with docker-compose.

### Before Shipwright

```bash
cd ~/apps/myapp
git pull
docker-compose -f docker-compose.vps.yml build
docker-compose -f docker-compose.vps.yml up -d
```

### After Shipwright

1. Add `.shipwright.yml` to your repo:

```yaml
version: 1

project:
  name: myapp
  framework: auto

build:
  compose_file: docker-compose.vps.yml

infrastructure:
  strategy: compose  # Use existing compose file
  deploy_dir: ~/apps/myapp  # Keep same location
  auto_detect: true  # Auto-detect proxy, networks, etc.

deploy:
  type: docker-compose
  vps:
    host: your-vps-ip
    user: your-user
    ssh_key: ~/.ssh/id_rsa
```

2. Register with Shipwright:

```bash
shipwright register
```

3. Push to GitHub - automatic deployment!

### Migration Notes

- Shipwright will **not** recreate containers if they're already running
- Uses your existing `docker-compose.vps.yml` file
- Respects existing networks and volumes
- No downtime during migration

---

## Shared Resources

### Using Shared PostgreSQL

```yaml
infrastructure:
  shared_resources:
    postgres:
      host: shared-postgres
      port: 5432
      database: myapp_db
      user: myapp
      network: core_shared-internal
```

**Shipwright will**:
- Connect your services to `core_shared-internal` network
- Set environment variables in containers:
  ```
  DATABASE_URL=postgres://myapp:password@shared-postgres:5432/myapp_db
  ```

### Using Shared Redis

```yaml
infrastructure:
  shared_resources:
    redis:
      host: shared-redis
      port: 6379
      db: 1
      network: core_shared-internal
```

**Shipwright will**:
- Set `REDIS_URL=redis://shared-redis:6379/1`
- Connect to appropriate network

---

## Auto-Detection Mode

If you don't want to configure everything manually, use auto-detection:

```yaml
version: 1

project:
  name: myapp

infrastructure:
  auto_detect: true  # Let Shipwright figure it out

deploy:
  type: docker-compose
  vps:
    host: your-vps-ip
    user: your-user
    ssh_key: ~/.ssh/id_rsa
    domain: myapp.example.com
```

**Shipwright will automatically**:
1. Detect Caddy/Nginx/Traefik
2. Find existing networks
3. Locate shared databases/redis
4. Choose appropriate deployment strategy
5. Determine best deployment directory

---

## Strategy Comparison

| Strategy | Use Case | Networks | Proxy Integration |
|----------|----------|----------|-------------------|
| `standalone` | Simple single app | Creates new | Manual |
| `compose` | Multi-service app | Joins existing | Automatic |
| `auto` | Let Shipwright decide | Intelligent | Automatic |

---

## Best Practices

### 1. **Always Specify `deploy_dir` for Multi-Project Setups**

```yaml
infrastructure:
  deploy_dir: ~/apps/your-project-name
```

This prevents conflicts and maintains organized structure.

### 2. **Use Explicit Network Configuration**

```yaml
infrastructure:
  networks:
    - proxy-tier
    - your-project-network
```

This ensures services can communicate with proxy and each other.

### 3. **Enable Auto-Update for Development, Disable for Production**

```yaml
infrastructure:
  proxy:
    auto_update: true  # Dev
    # auto_update: false  # Production (manual review)
```

### 4. **Test with Dry Run First**

```bash
shipwright up --dry-run
```

This shows what Shipwright will do without making changes.

### 5. **Use Health Checks**

```yaml
deploy:
  health:
    http:
      path: /health
      expect: 200
      timeout: 30s
```

Ensures deployment succeeds before marking complete.

---

## Troubleshooting

### Issue: Containers can't reach each other

**Solution**: Add them to the same network

```yaml
infrastructure:
  networks:
    - your-project-network
```

### Issue: Proxy not updating

**Check**:
1. Is `auto_update: true`?
2. Is `container_name` correct?
3. Check Shipwright agent logs: `docker logs shipwright-agent`

### Issue: Port conflicts

**Solution**: Shipwright uses port discovery. Check agent logs for assigned ports.

---

## Next Steps

- See [EXAMPLES.md](./EXAMPLES.md) for complete project examples
- Read [MIGRATION_GUIDE.md](./MIGRATION_GUIDE.md) for migrating existing projects
- Check [ARCHITECTURE.md](./ARCHITECTURE.md) for technical details
