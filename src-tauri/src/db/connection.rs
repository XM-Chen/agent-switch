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

    // busy_timeout=5000：未来若增开只读连接，遇到写锁时最多等 5s 再报 SQLITE_BUSY，
    // 而不是立即失败。当前所有访问通过 Arc<Mutex<Connection>> 序列化，属于防御性设置。
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )
    .map_err(|e| format!("数据库初始化失败: {}", e))?;

    Ok(Arc::new(Mutex::new(conn)))
}
