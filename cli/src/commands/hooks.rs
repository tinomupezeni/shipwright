use anyhow::{Result, Context};
use std::fs;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub async fn install() -> Result<()> {
    let git_dir = Path::new(".git");
    if !git_dir.exists() {
        anyhow::bail!("Not a git repository. Run this command from the root of your project.");
    }

    let hooks_dir = git_dir.join("hooks");
    if !hooks_dir.exists() {
        fs::create_dir_all(&hooks_dir)?;
    }

    let pre_push_hook = r#"#!/bin/sh
# Shipwright Pre-push Hook
# Verifies deployment configuration before pushing

echo "🚢 Shipwright: Verifying deployment configuration..."

# Check if shipwright is installed
if ! command -v shipwright >/dev/null 2>&1; then
    echo "⚠️  Shipwright CLI not found in PATH. Skipping verification."
    exit 0
fi

# Run dry-run deployment
if ! shipwright up --dry-run; then
    echo "❌ Shipwright: Deployment verification failed. Push aborted."
    exit 1
fi

echo "✅ Shipwright: Configuration verified."
exit 0
"#;

    let hook_path = hooks_dir.join("pre-push");
    fs::write(&hook_path, pre_push_hook).context("Failed to write pre-push hook")?;

    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&hook_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&hook_path, perms)?;
    }

    println!("✅ Git pre-push hook installed at {}", hook_path.display());
    println!("ℹ️  Shipwright will now verify your configuration before every 'git push'.");

    Ok(())
}
