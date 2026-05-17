# Shipwright Port Configuration

## Fixed Ports

Shipwright Agent uses **fixed ports** to ensure reliable webhook delivery and avoid configuration drift.

### Port Assignments

```
HTTP (Webhooks):  17670
WebSocket (Logs): 17671
```

### Why These Ports?

**17670/17671 chosen because:**
- ✅ **Outside common web dev range** (3000-9999) - no conflicts with typical dev servers
- ✅ **Not IANA reserved** - won't conflict with standard services
- ✅ **Easy to remember** - consecutive numbers in quiet range
- ✅ **Scalable** - same ports work for all users

**Avoided ranges:**
- ❌ 8000-8999: Django, FastAPI, dev servers
- ❌ 3000-3999: Node.js, React, Next.js
- ❌ 5000-5999: Flask, Rails
- ❌ 9000-9999: Monitoring tools

## Firewall Configuration

The agent **automatically opens firewall ports** on startup:

```bash
# Agent runs these commands automatically
sudo ufw allow 17670/tcp  # HTTP webhooks
sudo ufw allow 17671/tcp  # WebSocket logs
```

### Manual Firewall Setup (if needed)

If automatic firewall configuration fails:

```bash
# UFW (Ubuntu/Debian)
sudo ufw allow 17670/tcp
sudo ufw allow 17671/tcp
sudo ufw reload

# iptables (alternative)
sudo iptables -I INPUT -p tcp --dport 17670 -j ACCEPT
sudo iptables -I INPUT -p tcp --dport 17671 -j ACCEPT
sudo iptables-save

# firewalld (RHEL/CentOS)
sudo firewall-cmd --permanent --add-port=17670/tcp
sudo firewall-cmd --permanent --add-port=17671/tcp
sudo firewall-cmd --reload
```

## GitHub Webhook URL

When registering your project, the webhook URL will be:

```
http://YOUR_VPS_IP:17670/webhooks/github
```

Example:
```
http://159.198.42.231:17670/webhooks/github
```

## Custom Ports (Advanced)

If you need to use different ports (e.g., port conflict), set environment variables:

```bash
# On VPS, before starting agent
export SHIPWRIGHT_HTTP_PORT=27500
export SHIPWRIGHT_WS_PORT=27501

# Then start agent
/usr/local/bin/shipwright-agent
```

Or in systemd service file:

```ini
[Service]
Environment="SHIPWRIGHT_HTTP_PORT=27500"
Environment="SHIPWRIGHT_WS_PORT=27501"
ExecStart=/usr/local/bin/shipwright-agent
```

**Note:** If using custom ports, you must:
1. Update GitHub webhooks manually
2. Use `shipwright watch` won't auto-detect custom ports

## Port Conflicts

If you see this error:

```
❌ Port 17670 is already in use!
   Shipwright Agent requires port 17670 for HTTP webhook connections.
   Please free this port or set SHIPWRIGHT_HTTP_PORT environment variable.
```

**Solutions:**

1. **Find what's using the port:**
   ```bash
   sudo lsof -i :17670
   sudo netstat -tulpn | grep 17670
   ```

2. **Stop the conflicting service:**
   ```bash
   sudo systemctl stop service-name
   ```

3. **Or use custom port** (see above)

## Architecture

```
┌─────────────────────────────────────────┐
│  GitHub                                  │
│  └─> Webhook: http://VPS:17670/...     │
└─────────────────┬───────────────────────┘
                  │
                  ▼ HTTP POST
┌─────────────────────────────────────────┐
│  VPS (159.198.42.231)                    │
│                                          │
│  Port 17670 (HTTP)                       │
│  ├─ /webhooks/github  ← Webhook receiver│
│  ├─ /projects         ← Registration    │
│  └─ /health           ← Health check    │
│                                          │
│  Port 17671 (WebSocket)                  │
│  └─ /ws/:project      ← Live build logs │
│                                          │
└─────────────────┬───────────────────────┘
                  │
                  ▼ WebSocket
┌─────────────────────────────────────────┐
│  Developer Machine                       │
│  └─> shipwright watch (ws://VPS:17671) │
└─────────────────────────────────────────┘
```

## Troubleshooting

### Webhook not triggering

```bash
# 1. Check agent is listening
ssh user@vps 'sudo netstat -tulpn | grep 17670'

# 2. Check firewall allows port
ssh user@vps 'sudo ufw status | grep 17670'

# 3. Test connection from local
curl http://YOUR_VPS_IP:17670/health

# 4. Check GitHub webhook delivery
# Go to: https://github.com/owner/repo/settings/hooks
# Click webhook → Recent Deliveries
```

### Watch command not connecting

```bash
# 1. Check agent WebSocket is running
ssh user@vps 'sudo netstat -tulpn | grep 17671'

# 2. Test WebSocket connection
wscat -c ws://YOUR_VPS_IP:17671
# (install: npm install -g wscat)

# 3. Check firewall
ssh user@vps 'sudo ufw status | grep 17671'
```

## Security Considerations

### Webhook Signature Verification

All webhooks are verified using HMAC-SHA256 signatures:

```
X-Hub-Signature-256: sha256=...
```

The agent **rejects** webhooks without valid signatures, even on port 17670.

### TLS/HTTPS

Currently, webhooks use HTTP (not HTTPS). For production:

**Option 1: Reverse Proxy (Recommended)**
```nginx
# Use Nginx/Caddy for TLS termination
server {
    listen 443 ssl;
    server_name yourdomain.com;

    location /shipwright/webhook {
        proxy_pass http://localhost:17670;
    }
}
```

Then GitHub webhook URL becomes:
```
https://yourdomain.com/shipwright/webhook
```

**Option 2: VPN/Private Network**
- Put VPS in private network
- Only allow GitHub IPs

### Port Exposure

The fixed ports **must be publicly accessible** for GitHub webhooks to work:

```bash
# These ports MUST allow external connections
17670/tcp  # GitHub → Agent
17671/tcp  # Developer → Agent (optional, can restrict to your IP)
```

To restrict WebSocket access to your IP only:

```bash
sudo ufw delete allow 17671/tcp
sudo ufw allow from YOUR_IP to any port 17671
```

## Migration from Old Ports

If upgrading from agent using ports 8083/8084/8081:

1. **Update agent** (new binary uses 17670/17671)
2. **Update GitHub webhook:**
   - Go to: https://github.com/owner/repo/settings/hooks
   - Edit webhook URL: `8083` → `17670`
   - Save
3. **Update firewall:**
   ```bash
   sudo ufw allow 17670/tcp
   sudo ufw allow 17671/tcp
   sudo ufw reload
   ```
4. **Remove old ports (optional):**
   ```bash
   sudo ufw delete allow 8081/tcp
   sudo ufw delete allow 8083/tcp
   sudo ufw delete allow 8084/tcp
   ```

## Related Documentation

- [Updating Agent](./UPDATING_AGENT.md) - How to deploy updates
- [Environment Variables](./ENVIRONMENT_VARIABLES.md) - Configuration
- [Secret Management](./SECRET_MANAGEMENT_DESIGN.md) - Security architecture
