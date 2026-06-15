#[test]
fn test_fts5_available() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute("CREATE VIRTUAL TABLE test USING fts5(content)", [])
        .expect("FTS5 should be available");
}
