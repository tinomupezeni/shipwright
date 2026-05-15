use anyhow::Result;
use rusqlite::Connection;
use refinery::embed_migrations;

embed_migrations!("src/db/migrations");

pub fn init_db() -> Result<Connection> {
    let mut conn = Connection::open(".shipwright/shipwright.db")?;
    
    migrations::runner().run(&mut conn)?;
    
    Ok(conn)
}
