# Updating the Shipwright Agent

When you make improvements to the Shipwright Agent (like adding environment variable validation), you can deploy the updates to your VPS without needing to SSH into it.

## Quick Update

From your local machine, in your Shipwright project directory:

```bash
shipwright update-agent
```

This command will:
1. ✅ Build the latest agent binary locally
2. ✅ Upload it to your VPS via SCP
3. ✅ Safely stop the running agent
4. ✅ Install the new binary
5. ✅ Restart the agent service
6. ✅ Verify it's running correctly

## What You Need

- A `.shipwright.yml` file with VPS configuration
- SSH access to your VPS (configured in `.shipwright.yml`)
- Cargo/Rust installed locally to build the agent

## Example Output

```
🚀 Updating Shipwright Agent on server1.example.com...
? This will rebuild and upload the agent binary. Continue? (Y/n) yes
🔨 Building Shipwright Agent binary locally...
✅ Agent binary built successfully
⏸️  Stopping agent service...
📤 Uploading new agent binary to VPS...
⚙️  Installing new agent binary...
🔄 Restarting agent service...
🔍 Checking agent status...

✅ Shipwright Agent updated successfully!
✨ The agent is now running with the latest changes.
```

## Safety Features

- **Automatic rollback**: If the upload fails, the old agent is automatically restarted
- **Service verification**: After update, the command verifies the agent is running
- **User confirmation**: You'll be prompted before making any changes

## When to Update

Update your agent when:
- 🐛 Bug fixes are released
- ✨ New features are added (like environment validation)
- 🔒 Security patches are available
- ⚡ Performance improvements are merged

## Troubleshooting

If the update fails:

1. **Check the agent status manually**:
   ```bash
   # From the command output, it will suggest this if there's an issue
   ssh user@your-vps 'sudo journalctl -u shipwright-agent -f'
   ```

2. **Verify SSH access**:
   ```bash
   ssh -i ~/.ssh/your-key user@your-vps
   ```

3. **Check build errors**: If the local build fails, fix compilation errors first

4. **Re-run the command**: The update is idempotent and safe to retry

## Manual Update (Not Recommended)

If you really need to SSH into the VPS manually:

```bash
# Don't do this - use 'shipwright update-agent' instead!
ssh user@vps
sudo systemctl stop shipwright-agent
# ... manual steps ...
```

**Remember**: Shipwright is designed so you never need to SSH into your VPS. Everything should be done from your local terminal using the CLI.

## Related Commands

- `shipwright setup` - Initial VPS setup and agent installation
- `shipwright status` - Check current deployment status
- `shipwright logs` - View application logs
- `shipwright watch` - Watch live build logs

## Technical Details

The update process:
1. Builds the agent using `cargo build --release -p shipwright-agent`
2. Uses SCP with your configured SSH key to transfer the binary
3. Moves the binary to `/usr/local/bin/shipwright-agent`
4. Restarts the systemd service `shipwright-agent.service`

The agent runs as a systemd service with automatic restart on failure, ensuring high availability during updates.
