use anyhow::Result;
use rusqlite::Connection;
use refinery::embed_migrations;

embed_migrations!("src/db/migrations");

pub fn init_db() -> Result<Connection> {
    // The agent database will be stored in a global location eventually,
    // but for now we'll put it in the same directory.
    let mut conn = Connection::open("shipwright-agent.db")?;
    
    migrations::runner().run(&mut conn)?;
    
    Ok(conn)
}
