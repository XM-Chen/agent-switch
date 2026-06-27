use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Open (or create) the SQLite database and return a thread-safe wrapped connection.
pub fn open_db(db_path: &Path) -> Result<Arc<Mutex<Connection>>, String> {
    // Ensure parent directory exists
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("无法创建数据库目录 '{}': {}", parent.display(), e))?;
    }

    let conn = Connection::open(db_path)
        .map_err(|e| format!("无法打开数据库 '{}': {}", db_path.display(), e))?;

    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("数据库初始化失败: {}", e))?;

    Ok(Arc::new(Mutex::new(conn)))
}
