use std::path::PathBuf;
use std::process::Command;

const MAGIC: u32 = 0x044D_495A;

fn u16b(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}
fn u32b(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}
fn u64b(v: u64) -> [u8; 8] {
    v.to_le_bytes()
}

fn dirent(mime: u16, cluster: u32, blob: u32, url: &str, title: &str) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&u16b(mime));
    v.push(0);
    v.push(b'C');
    v.extend_from_slice(&u32b(1));
    v.extend_from_slice(&u32b(cluster));
    if mime != 0xFFFF {
        v.extend_from_slice(&u32b(blob));
    }
    v.extend_from_slice(url.as_bytes());
    v.push(0);
    v.extend_from_slice(title.as_bytes());
    v.push(0);
    v
}

fn cluster(blobs: &[Vec<u8>]) -> Vec<u8> {
    let n = blobs.len();
    let mut plain = Vec::new();
    let mut at = (4 * (n + 1)) as u32;
    for b in blobs {
        plain.extend_from_slice(&u32b(at));
        at += b.len() as u32;
    }
    plain.extend_from_slice(&u32b(at));
    for b in blobs {
        plain.extend_from_slice(b);
    }
    let mut out = vec![0u8];
    out.extend_from_slice(&plain);
    out
}

fn zim_with(articles: usize) -> Vec<u8> {
    let body = |i: usize| {
        format!(
            "<p>Article {i} covers a subject in enough words that it is kept by the indexer, \
             with detail about the same subject repeated for length.</p>"
        )
        .into_bytes()
    };
    let mut dirents = Vec::new();
    let mut blobs = Vec::new();
    for i in 0..articles {
        dirents.push(dirent(
            0,
            0,
            i as u32,
            &format!("a{i:05}"),
            &format!("Article {i:05}"),
        ));
        blobs.push(body(i));
    }

    let mimes = b"text/html\0\0".to_vec();
    let mut out = vec![0u8; 80];
    let mime_pos = out.len() as u64;
    out.extend_from_slice(&mimes);
    let mut dirent_pos = Vec::new();
    for d in &dirents {
        dirent_pos.push(out.len() as u64);
        out.extend_from_slice(d);
    }
    let cl_pos = out.len() as u64;
    out.extend_from_slice(&cluster(&blobs));
    let url_pos = out.len() as u64;
    for p in &dirent_pos {
        out.extend_from_slice(&u64b(*p));
    }
    let title_pos = out.len() as u64;
    for i in 0..articles {
        out.extend_from_slice(&u32b(i as u32));
    }
    let cluster_pos = out.len() as u64;
    out.extend_from_slice(&u64b(cl_pos));
    let checksum_pos = out.len() as u64;
    out.extend_from_slice(&[0u8; 16]);

    out[0..4].copy_from_slice(&u32b(MAGIC));
    out[4..6].copy_from_slice(&u16b(6));
    out[6..8].copy_from_slice(&u16b(1));
    out[24..28].copy_from_slice(&u32b(articles as u32));
    out[28..32].copy_from_slice(&u32b(1));
    out[32..40].copy_from_slice(&u64b(url_pos));
    out[40..48].copy_from_slice(&u64b(title_pos));
    out[48..56].copy_from_slice(&u64b(cluster_pos));
    out[56..64].copy_from_slice(&u64b(mime_pos));
    out[64..68].copy_from_slice(&u32b(0));
    out[72..80].copy_from_slice(&u64b(checksum_pos));
    out
}

fn zim_titled(items: &[(&str, &str)]) -> Vec<u8> {
    let n = items.len();
    let body = |i: usize| {
        format!(
            "<p>Article {i} covers a subject in enough words that it is kept by the indexer, \
             with detail about the same subject repeated for length.</p>"
        )
        .into_bytes()
    };
    let mut dirents = Vec::new();
    let mut blobs = Vec::new();
    for (i, (url, title)) in items.iter().enumerate() {
        dirents.push(dirent(0, 0, i as u32, url, title));
        blobs.push(body(i));
    }

    let mimes = b"text/html\0\0".to_vec();
    let mut out = vec![0u8; 80];
    let mime_pos = out.len() as u64;
    out.extend_from_slice(&mimes);
    let mut dirent_pos = Vec::new();
    for d in &dirents {
        dirent_pos.push(out.len() as u64);
        out.extend_from_slice(d);
    }
    let cl_pos = out.len() as u64;
    out.extend_from_slice(&cluster(&blobs));
    let url_pos = out.len() as u64;
    for p in &dirent_pos {
        out.extend_from_slice(&u64b(*p));
    }
    let cluster_pos = out.len() as u64;
    out.extend_from_slice(&u64b(cl_pos));
    let checksum_pos = out.len() as u64;
    out.extend_from_slice(&[0u8; 16]);

    out[0..4].copy_from_slice(&u32b(MAGIC));
    out[4..6].copy_from_slice(&u16b(6));
    out[6..8].copy_from_slice(&u16b(1));
    out[24..28].copy_from_slice(&u32b(n as u32));
    out[28..32].copy_from_slice(&u32b(1));
    out[32..40].copy_from_slice(&u64b(url_pos));
    out[40..48].copy_from_slice(&u64b(u64::MAX));
    out[48..56].copy_from_slice(&u64b(cluster_pos));
    out[56..64].copy_from_slice(&u64b(mime_pos));
    out[64..68].copy_from_slice(&u32b(0));
    out[72..80].copy_from_slice(&u64b(checksum_pos));
    out
}

fn zim_clustered(cluster_of: &[usize]) -> Vec<u8> {
    let n = cluster_of.len();
    let num_clusters = cluster_of.iter().copied().max().map(|m| m + 1).unwrap_or(0);
    let body = |i: usize| {
        format!(
            "<p>Article {i} covers a subject in enough words that it is kept by the indexer, \
             with detail about the same subject repeated for length.</p>"
        )
        .into_bytes()
    };
    let mut blobs: Vec<Vec<Vec<u8>>> = (0..num_clusters).map(|_| Vec::new()).collect();
    let mut dirents: Vec<Vec<u8>> = Vec::new();
    for i in 0..n {
        let c = cluster_of[i];
        let blob = blobs[c].len();
        blobs[c].push(body(i));
        dirents.push(dirent(
            0,
            c as u32,
            blob as u32,
            &format!("a{i:05}"),
            &format!("Article {i:05}"),
        ));
    }

    let mimes = b"text/html\0\0".to_vec();
    let mut out = vec![0u8; 80];
    let mime_pos = out.len() as u64;
    out.extend_from_slice(&mimes);
    let mut dirent_pos = Vec::new();
    for d in &dirents {
        dirent_pos.push(out.len() as u64);
        out.extend_from_slice(d);
    }
    let mut cluster_pos = Vec::new();
    for b in &blobs {
        cluster_pos.push(out.len() as u64);
        out.extend_from_slice(&cluster(b));
    }
    let url_pos = out.len() as u64;
    for p in &dirent_pos {
        out.extend_from_slice(&u64b(*p));
    }
    let title_pos = out.len() as u64;
    for i in 0..n {
        out.extend_from_slice(&u32b(i as u32));
    }
    let cluster_ptr_pos = out.len() as u64;
    for p in &cluster_pos {
        out.extend_from_slice(&u64b(*p));
    }
    let checksum_pos = out.len() as u64;
    out.extend_from_slice(&[0u8; 16]);

    out[0..4].copy_from_slice(&u32b(MAGIC));
    out[4..6].copy_from_slice(&u16b(6));
    out[6..8].copy_from_slice(&u16b(1));
    out[24..28].copy_from_slice(&u32b(n as u32));
    out[28..32].copy_from_slice(&u32b(num_clusters as u32));
    out[32..40].copy_from_slice(&u64b(url_pos));
    out[40..48].copy_from_slice(&u64b(title_pos));
    out[48..56].copy_from_slice(&u64b(cluster_ptr_pos));
    out[56..64].copy_from_slice(&u64b(mime_pos));
    out[64..68].copy_from_slice(&u32b(0));
    out[72..80].copy_from_slice(&u64b(checksum_pos));
    out
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("alex-index-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn indexed(stdout: &str) -> usize {
    let line = stdout
        .lines()
        .find(|l| l.starts_with("indexed "))
        .expect("no summary line");
    line.trim_start_matches("indexed ")
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

fn run(zim: &PathBuf, out: &PathBuf, stop: Option<usize>) -> (bool, String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_alex"));
    c.arg("index").arg(zim).arg(out);
    c.env_remove("ALEX_INDEX_STOP_AFTER");
    if let Some(n) = stop {
        c.env("ALEX_INDEX_STOP_AFTER", n.to_string());
    }
    let o = c.output().unwrap();
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).to_string(),
        String::from_utf8_lossy(&o.stderr).to_string(),
    )
}

#[test]
fn an_interrupted_index_finishes_on_the_next_run_without_duplicates() {
    let d = dir("resume");
    let zim = d.join("pack.zim");
    std::fs::write(&zim, zim_with(40)).unwrap();

    let clean = d.join("clean.search.db");
    let (ok, out, _) = run(&zim, &clean, None);
    assert!(ok, "clean run failed");
    let all = indexed(&out);
    assert_eq!(all, 40);

    let part = d.join("part.search.db");
    let (ok, _, _) = run(&zim, &part, Some(5));
    assert!(!ok, "the interrupted run should fail");
    assert!(part.is_file(), "the partial database must survive the failure");

    let (ok, out, err) = run(&zim, &part, None);
    assert!(ok, "the resumed run failed");
    assert!(
        err.contains("resuming after 5"),
        "the second run must continue the partial index, not start over: {err}"
    );
    assert_eq!(
        indexed(&out),
        all,
        "the resumed index must hold every article exactly once"
    );

    let (ok, _, err) = run(&zim, &part, None);
    assert!(ok);
    assert_eq!(indexed(&out), all, "a finished index is rebuilt, not extended");
    assert!(!err.contains("resuming"), "a finished index starts over: {err}");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn the_new_index_is_contentless_with_a_docs_table_and_still_searchable() {
    let d = dir("contentless");
    let zim = d.join("pack.zim");
    std::fs::write(&zim, zim_with(30)).unwrap();

    let out = d.join("pack.search.db");
    let (ok, _, err) = run(&zim, &out, None);
    assert!(ok, "index failed: {err}");

    let conn = rusqlite::Connection::open(&out).unwrap();
    let docs: i64 = conn
        .query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='docs'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(docs, 1, "the new index must carry a docs table");
    let stored_body: i64 = conn
        .query_row("SELECT count(*) FROM articles WHERE body LIKE '%covers a subject%'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stored_body, 0, "article bodies must not be stored in the index");
    let n: i64 = conn
        .query_row("SELECT count(*) FROM articles", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 30, "every article must be indexed once");
    drop(conn);

    let db = alexandria_search::open_local(&out).unwrap();
    let hits = alexandria_search::query(db.conn(), "Article 00007").unwrap();
    assert_eq!(hits[0].title, "Article 00007");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn a_resumed_index_follows_the_latest_checkpoint_when_old_builds_left_many_rows() {
    let d = dir("resume-many");
    let zim = d.join("pack.zim");
    std::fs::write(&zim, zim_with(2500)).unwrap();

    let part = d.join("part.search.db");
    let (ok, _, _) = run(&zim, &part, Some(1500));
    assert!(!ok, "the interrupted run should fail");
    assert!(part.is_file());

    let conn = rusqlite::Connection::open(&part).unwrap();
    let (total, bytes): (i64, i64) = conn
        .query_row("SELECT total, bytes FROM progress LIMIT 1", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    conn.execute("DELETE FROM progress", []).unwrap();
    for (s, k) in [(500, 500), (1000, 1000), (1500, 1500)] {
        conn.execute(
            "INSERT INTO progress(seen, kept, total, bytes) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![s, k, total, bytes],
        )
        .unwrap();
    }
    drop(conn);

    let (ok, _, err) = run(&zim, &part, None);
    assert!(ok, "the resumed run failed: {err}");
    assert!(
        err.contains("resuming after 1500"),
        "must resume from the latest checkpoint, not the first: {err}"
    );

    let conn = rusqlite::Connection::open(&part).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM articles", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2500, "each article must be indexed exactly once");
    drop(conn);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn rowids_follow_title_order_when_the_title_index_is_absent() {
    let d = dir("title-order");
    let zim = d.join("pack.zim");
    std::fs::write(&zim, zim_titled(&[("zulu", "Zulu"), ("alpha", "Alpha"), ("mike", "Mike")])).unwrap();

    let out = d.join("pack.search.db");
    let (ok, _, err) = run(&zim, &out, None);
    assert!(ok, "index failed: {err}");

    let conn = rusqlite::Connection::open(&out).unwrap();
    let titles: Vec<String> = conn
        .prepare("SELECT title FROM docs ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|t| t.unwrap())
        .collect();
    assert_eq!(titles, vec!["Alpha", "Mike", "Zulu"], "rowids must follow title order, not URL order");
    let order: String = conn
        .query_row("SELECT value FROM meta WHERE key = 'order'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(order, "title", "the index must be marked title-ordered");
    let articles: i64 = conn
        .query_row("SELECT value FROM meta WHERE key = 'articles'", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(articles, 3, "the article count must be written to meta");
    drop(conn);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn a_cluster_ordered_index_resumes_exactly_once() {
    let d = dir("cluster-resume");
    let zim = d.join("pack.zim");
    let cluster_of: Vec<usize> = (0..60).map(|i| i % 3).collect();
    std::fs::write(&zim, zim_clustered(&cluster_of)).unwrap();

    let part = d.join("part.search.db");
    let (ok, _, _) = run(&zim, &part, Some(7));
    assert!(!ok, "the interrupted run should fail");
    assert!(part.is_file());

    let (ok, _, err) = run(&zim, &part, None);
    assert!(ok, "the resumed run failed: {err}");
    assert!(err.contains("resuming after 7"), "the run must continue, not restart: {err}");

    let conn = rusqlite::Connection::open(&part).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM docs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 60, "each article must be indexed exactly once");
    drop(conn);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn duplicate_titles_resume_without_primary_key_collisions() {
    let d = dir("dup-resume");
    let zim = d.join("pack.zim");
    std::fs::write(&zim, zim_titled(&[("b", "Same"), ("a", "Same"), ("c", "Other")])).unwrap();

    let clean = d.join("clean.search.db");
    let (ok, out, _) = run(&zim, &clean, None);
    assert!(ok);
    assert_eq!(indexed(&out), 3);
    let conn = rusqlite::Connection::open(&clean).unwrap();
    let urls: Vec<String> = conn
        .prepare("SELECT url FROM docs WHERE title = 'Same' ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|u| u.unwrap())
        .collect();
    assert_eq!(urls, vec!["b", "a"], "duplicate titles must order by url index, not url text");
    drop(conn);

    let part = d.join("part.search.db");
    let (ok, _, _) = run(&zim, &part, Some(1));
    assert!(!ok, "the interrupted run should fail");
    let (ok, _, err) = run(&zim, &part, None);
    assert!(ok, "the resumed run failed: {err}");
    assert!(err.contains("resuming after 1"), "must resume, not restart: {err}");
    let conn = rusqlite::Connection::open(&part).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM docs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 3, "each article must be indexed exactly once");
    drop(conn);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn an_old_schema_part_is_rebuilt_from_scratch() {
    let d = dir("schema-version");
    let zim = d.join("pack.zim");
    std::fs::write(&zim, zim_with(10)).unwrap();

    let part = d.join("part.search.db");
    {
        let conn = rusqlite::Connection::open(&part).unwrap();
        conn.execute_batch(
            "CREATE TABLE progress(seen INTEGER NOT NULL, kept INTEGER NOT NULL, total INTEGER NOT NULL, bytes INTEGER NOT NULL);
             INSERT INTO progress(seen, kept, total, bytes) VALUES (5, 5, 10, 0);",
        )
        .unwrap();
    }

    let (ok, out, err) = run(&zim, &part, None);
    assert!(ok, "the rebuild failed: {err}");
    assert!(err.contains("schema"), "must explain the rebuild: {err}");
    assert_eq!(indexed(&out), 10, "the rebuilt index must hold every article");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn search_looks_in_every_pack_and_puts_the_exact_title_first() {
    let d = dir("allpacks");
    let lib = d.join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("a.zim"), zim_titled(&[("Other", "Other"), ("Paris_community", "Paris community")])).unwrap();
    std::fs::write(lib.join("b.zim"), zim_titled(&[("Lyon", "Lyon"), ("Paris", "Paris")])).unwrap();
    let alex = |args: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_alex")).args(args).output().unwrap();
        assert!(o.status.success(), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).to_string()
    };
    let seed = d.join("seed").to_string_lossy().to_string();
    let pubf = d.join("pub").to_string_lossy().to_string();
    let l = lib.to_string_lossy().to_string();
    alex(&["keygen", &seed, &pubf]);
    let key = std::fs::read_to_string(&pubf).unwrap().trim().to_string();
    alex(&["manifest", &l, "v0.2.0", "2026-11-06"]);
    alex(&["sign", &l, &seed]);
    alex(&["index-library", &l, &key]);
    let out = alex(&["search", &l, &key, "Paris"]);
    assert_eq!(out.lines().next().unwrap().split('\t').next().unwrap(), "Paris", "{out}");
    assert!(out.contains("Paris community"), "{out}");
    std::fs::remove_dir_all(&d).ok();
}
