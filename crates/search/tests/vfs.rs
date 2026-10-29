use alexandria_core::{write_hashes, Entry, VerifiedFile};
use alexandria_search::{fts_query, open_verified, query};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CH: u64 = 4096;

fn tmpdir(tag: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("alex-search-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&p).unwrap();
    p
}

fn entry_for(path: &Path) -> Entry {
    let size = fs::metadata(path).unwrap().len();
    let hashes = write_hashes(path, CH).unwrap();
    let raw = fs::read(hashes).unwrap();
    use sha2::{Digest, Sha256};
    let mut h = [0u8; 32];
    h.copy_from_slice(&Sha256::digest(&raw));
    Entry {
        hashes_sha256: h,
        size,
        path: path.file_name().unwrap().to_str().unwrap().to_string(),
    }
}

fn build_db(path: &Path) {
    let c = rusqlite::Connection::open(path).unwrap();
    c.execute_batch(
        "PRAGMA journal_mode=OFF;
         CREATE VIRTUAL TABLE articles USING fts5(title, body, url UNINDEXED,
         tokenize='unicode61 remove_diacritics 2');",
    )
    .unwrap();
    let tx = c.unchecked_transaction().unwrap();
    for i in 0..3000 {
        tx.execute(
            "INSERT INTO articles(title, body, url) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                format!("Article {i}"),
                format!("filler text number {i} about rivers and mountains and cities"),
                format!("Article_{i}")
            ],
        )
        .unwrap();
    }
    tx.execute(
        "INSERT INTO articles(title, body, url) VALUES ('Water purification', 'Water purification is the process of removing contaminants from water. Boiling kills germs.', 'Water_purification')",
        [],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO articles(title, body, url) VALUES ('Dysentery', 'Dysentery is an infection of the intestines.', 'Dysentery')",
        [],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO articles(title, body, url) VALUES ('Vibrio', 'Dysentery is caused by bacteria in this genus.', 'Vibrio')",
        [],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO articles(title, body, url) VALUES ('Kinshasa', 'Kinshasa is the capital of the Congo, on the Congo River.', 'Kinshasa')",
        [],
    )
    .unwrap();
    tx.commit().unwrap();
    c.execute_batch("INSERT INTO articles(articles) VALUES('optimize'); VACUUM;")
        .unwrap();
}

#[test]
fn query_reads_through_verified_pages() {
    let dir = tmpdir("ok");
    let db = dir.join("pack.zim.search.db");
    build_db(&db);
    let e = entry_for(&db);
    assert!(e.size > 20 * CH, "fixture too small: {}", e.size);
    let vf = VerifiedFile::open_in(&dir, &e, CH).unwrap();
    let conn = open_verified(vf, CH).unwrap();
    let hits = query(conn.conn(), "congo river").unwrap();
    assert_eq!(hits[0].title, "Kinshasa");
    assert_eq!(hits[0].url, "Kinshasa");
    assert_eq!(query(conn.conn(), "Kinshasa").unwrap()[0].title, "Kinshasa");
    assert_eq!(query(conn.conn(), "What is Kinshasa?").unwrap()[0].title, "Kinshasa");
    assert_eq!(query(conn.conn(), "kinshasa zanzibar").unwrap()[0].title, "Kinshasa");
    let h = alexandria_search::ask_candidates(conn.conn(), "wikipedia_en", "How do I purify water without a filter?").unwrap();
    assert_eq!(h[0].title, "Water purification", "{h:?}");
    let d = alexandria_search::ask_candidates(conn.conn(), "wikipedia_en", "What causes dysentery?").unwrap();
    assert_eq!(d[0].title, "Dysentery", "{d:?}");
    let picked = alexandria_search::pick_sources(d.into_iter().map(|p| ("w".to_string(), p)).collect(), 3, "What causes dysentery?");
    assert!(picked.iter().all(|(_, p)| p.title != "Vibrio"), "body-only mention kept: {picked:?}");
    assert_eq!(query(conn.conn(), "article 2999").unwrap()[0].url, "Article_2999");
}

#[test]
fn tampered_page_fails_closed() {
    let dir = tmpdir("bad");
    let db = dir.join("pack.zim.search.db");
    build_db(&db);
    let e = entry_for(&db);
    let mut bytes = fs::read(&db).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xFF;
    fs::write(&db, &bytes).unwrap();
    let vf = VerifiedFile::open_in(&dir, &e, CH).unwrap();
    let result = open_verified(vf, CH).and_then(|c| {
        c.conn()
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())
    });
    match result {
        Err(_) => {}
        Ok(report) => assert!(
            report != "ok" && report.contains("error code=266"),
            "tampered index read as valid: {report}"
        ),
    }
    let vf = VerifiedFile::open_in(&dir, &e, CH).unwrap();
    if let Ok(c) = open_verified(vf, CH) {
        let scan: Result<i64, _> = c.conn().query_row(
            "SELECT sum(length(block)) FROM articles_data",
            [],
            |r| r.get(0),
        );
        let scan2: Result<i64, _> = c.conn().query_row(
            "SELECT sum(length(c1)) FROM articles_content",
            [],
            |r| r.get(0),
        );
        assert!(scan.is_err() || scan2.is_err(), "both full scans succeeded");
    }
}

#[test]
fn index_is_read_only() {
    let dir = tmpdir("ro");
    let db = dir.join("pack.zim.search.db");
    build_db(&db);
    let e = entry_for(&db);
    let before = fs::read(&db).unwrap();
    let vf = VerifiedFile::open_in(&dir, &e, CH).unwrap();
    let conn = open_verified(vf, CH).unwrap();
    let w = conn
        .conn()
        .execute("INSERT INTO articles(title, body, url) VALUES ('x','y','z')", []);
    assert!(w.is_err());
    drop(conn);
    assert_eq!(before, fs::read(&db).unwrap());
    let names: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|d| d.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        !names.iter().any(|n| n.ends_with("-journal") || n.ends_with("-wal")),
        "{names:?}"
    );
}

#[test]
fn fts_query_quotes_terms() {
    assert_eq!(fts_query("Congo river"), "\"congo\"* AND \"river\"*");
    assert_eq!(fts_query("a \" OR 1"), "\"or\"*");
    assert_eq!(fts_query("!!"), "\"\"");
    assert_eq!(fts_query("What is the Congo?"), "\"congo\"*");
    assert_eq!(fts_query("purify"), "\"purif\"*");
    assert_eq!(fts_query("The Who"), "\"the\"* AND \"who\"*");
}

#[test]
fn a_locally_built_index_opens_read_only() {
    let dir = tmpdir("local");
    let db = dir.join("pack.zim.0123456789abcdef.search.db");
    build_db(&db);
    let before = fs::read(&db).unwrap();
    let conn = alexandria_search::open_local(&db).unwrap();
    let hits = query(conn.conn(), "water purification").unwrap();
    assert_eq!(hits[0].title, "Water purification");
    assert!(conn
        .conn()
        .execute("INSERT INTO articles(title, body, url) VALUES ('x','y','z')", [])
        .is_err());
    drop(conn);
    assert_eq!(before, fs::read(&db).unwrap());
}

#[test]
fn a_missing_or_garbage_local_index_is_an_error_not_a_panic() {
    let dir = tmpdir("garbage");
    assert!(alexandria_search::open_local(&dir.join("nope.search.db")).is_err());
    let junk = dir.join("junk.search.db");
    fs::write(&junk, b"this is not sqlite at all, just bytes").unwrap();
    assert!(alexandria_search::open_local(&junk).is_err());
}

#[test]
fn local_index_sits_beside_the_library_and_names_the_pack_content() {
    let e = Entry { hashes_sha256: [0xab; 32], size: 1, path: "water.zim".into() };
    let p = alexandria_search::local_index_path(Path::new("/Volumes/SSD/Alexandria"), &e);
    assert_eq!(p, Path::new("/Volumes/SSD/Alexandria-Index/water.zim.abababababababab.search.db"));
}

#[test]
fn a_plural_search_finds_the_singular_article_first() {
    let dir = tmpdir("plural");
    let db = dir.join("p.zim.0123456789abcdef.search.db");
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch(
        "CREATE VIRTUAL TABLE articles USING fts5(title, body, url UNINDEXED, tokenize='unicode61 remove_diacritics 2');
         INSERT INTO articles VALUES ('Burns, Baby Burns', 'Burns, Baby Burns is a Simpsons episode about burns.', 'a');
         INSERT INTO articles VALUES ('Who Shot Mr. Burns?', 'Mr. Burns burns Springfield. Burns burns.', 'b');
         INSERT INTO articles VALUES ('Burn', 'A burn is an injury to skin. Burns hurt. Cool burns with water.', 'c');
         INSERT INTO articles VALUES ('Knot', 'A knot ties rope. Many knots exist.', 'd');",
    )
    .unwrap();
    drop(c);
    let conn = alexandria_search::open_local(&db).unwrap();
    assert_eq!(query(conn.conn(), "burns").unwrap()[0].title, "Burn");
    assert_eq!(query(conn.conn(), "knots").unwrap()[0].title, "Knot");
    assert_eq!(query(conn.conn(), "Burns, Baby Burns").unwrap()[0].title, "Burns, Baby Burns");
}
