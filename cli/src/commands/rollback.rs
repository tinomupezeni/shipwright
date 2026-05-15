use anyhow::Result;
use shipwright_common::config::Config;

pub async fn run(_config: &Config) -> Result<()> {
    // TODO: Implement rollback logic
    // 1. Get previous successful deploy image from DB
    // 2. Deploy it to VPS
    println!("Rolling back to previous stable version...");
    Ok(())
}
