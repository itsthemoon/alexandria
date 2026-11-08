use alexandria_search::browse;
use rusqlite::Connection;

fn contentless_db(rows: &[(&str, &str)]) -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(
        "CREATE VIRTUAL TABLE articles USING fts5(title, body, content='', tokenize='unicode61 remove_diacritics 2');
         CREATE TABLE docs(id INTEGER PRIMARY KEY, title TEXT NOT NULL, url TEXT NOT NULL);",
    )
    .unwrap();
    for (t, u) in rows {
        c.execute("INSERT INTO docs(title, url) VALUES (?1, ?2)", rusqlite::params![t, u]).unwrap();
        let id = c.last_insert_rowid();
        c.execute(
            "INSERT INTO articles(rowid, title, body) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, t, "body text"],
        )
        .unwrap();
    }
    c
}

fn old_db(rows: &[(&str, &str)]) -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(
        "CREATE VIRTUAL TABLE articles USING fts5(title, body, url UNINDEXED, tokenize='unicode61 remove_diacritics 2');",
    )
    .unwrap();
    for (t, u) in rows {
        c.execute(
            "INSERT INTO articles(title, body, url) VALUES (?1, ?2, ?3)",
            rusqlite::params![t, "body text", u],
        )
        .unwrap();
    }
    c
}

#[test]
fn contentless_browse_filters_by_title_prefix_and_counts_total() {
    let rows = [
        ("Alpha", "a"),
        ("Alphabet", "al"),
        ("Alpine", "alp"),
        ("Beta", "b"),
        ("Gamma", "g"),
    ];
    let c = contentless_db(&rows);
    let (total, more, hits) = browse(&c, "al", false, 0, 10).unwrap();
    assert_eq!(total, 3);
    assert!(!more);
    let titles: Vec<&str> = hits.iter().map(|h| h.title.as_str()).collect();
    assert_eq!(titles, vec!["Alpha", "Alphabet", "Alpine"]);
    let urls: Vec<&str> = hits.iter().map(|h| h.url.as_str()).collect();
    assert_eq!(urls, vec!["a", "al", "alp"]);
}

#[test]
fn contentless_browse_reverses_and_paginates() {
    let rows = [
        ("Alpha", "a"),
        ("Alphabet", "al"),
        ("Alpine", "alp"),
        ("Beta", "b"),
    ];
    let c = contentless_db(&rows);
    let (total, _, hits) = browse(&c, "al", true, 0, 10).unwrap();
    assert_eq!(total, 3);
    let titles: Vec<&str> = hits.iter().map(|h| h.title.as_str()).collect();
    assert_eq!(titles, vec!["Alpine", "Alphabet", "Alpha"]);

    let (total, _, hits) = browse(&c, "al", false, 1, 1).unwrap();
    assert_eq!(total, 3);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "Alphabet");
}

#[test]
fn old_layout_browse_uses_the_articles_table() {
    let rows = [("Alpha", "a"), ("Alphabet", "al"), ("Beta", "b")];
    let c = old_db(&rows);
    let (total, _, hits) = browse(&c, "al", false, 0, 10).unwrap();
    assert_eq!(total, 2);
    let titles: Vec<&str> = hits.iter().map(|h| h.title.as_str()).collect();
    assert_eq!(titles, vec!["Alpha", "Alphabet"]);
}

#[test]
fn browse_finds_a_match_deep_in_the_index() {
    let mut rows: Vec<(String, String)> = Vec::new();
    for i in 0..5000 {
        rows.push((format!("Topic {i:04}"), format!("url{i:04}")));
    }
    let target = ("Zanzibar", "zanzibar");
    rows.push((target.0.to_string(), target.1.to_string()));
    let refs: Vec<(&str, &str)> = rows.iter().map(|(t, u)| (t.as_str(), u.as_str())).collect();
    let c = contentless_db(&refs);
    let (total, _, hits) = browse(&c, "zanz", false, 0, 10).unwrap();
    assert_eq!(total, 1);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "Zanzibar");
    assert_eq!(hits[0].url, "zanzibar");
}

#[test]
fn browse_returns_nothing_for_a_tokenless_filter() {
    let c = contentless_db(&[("Alpha", "a")]);
    let (total, more, hits) = browse(&c, "!!!", false, 0, 10).unwrap();
    assert_eq!(total, 0);
    assert!(!more);
    assert!(hits.is_empty());
}

fn title_ordered_db(rows: &[(&str, &str)]) -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(
        "CREATE VIRTUAL TABLE articles USING fts5(title, body, content='', tokenize='unicode61 remove_diacritics 2');
         CREATE TABLE docs(id INTEGER PRIMARY KEY, title TEXT NOT NULL, url TEXT NOT NULL);
         CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO meta(key, value) VALUES ('order', 'title');",
    )
    .unwrap();
    for (i, (t, u)) in rows.iter().enumerate() {
        let id = i as i64 * 7 + 3;
        c.execute(
            "INSERT INTO docs(id, title, url) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, t, u],
        )
        .unwrap();
        c.execute(
            "INSERT INTO articles(rowid, title, body) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, t, "body text"],
        )
        .unwrap();
    }
    c
}

#[test]
fn title_ordered_browse_pages_in_rowid_order_without_sorting_titles() {
    let rows = [
        ("Beta", "b"),
        ("Alphabet", "al"),
        ("Alpha", "a"),
        ("Alpine", "alp"),
    ];
    let c = title_ordered_db(&rows);
    let (total, _, hits) = browse(&c, "al", false, 0, 10).unwrap();
    assert_eq!(total, 3);
    let titles: Vec<&str> = hits.iter().map(|h| h.title.as_str()).collect();
    assert_eq!(titles, vec!["Alphabet", "Alpha", "Alpine"], "page order must follow rowid (title) order, not alphabetical");

    let (total, _, hits) = browse(&c, "al", true, 0, 10).unwrap();
    assert_eq!(total, 3);
    let titles: Vec<&str> = hits.iter().map(|h| h.title.as_str()).collect();
    assert_eq!(titles, vec!["Alpine", "Alpha", "Alphabet"]);
}

#[test]
fn the_count_is_capped_and_flagged_when_matches_exceed_the_cap() {
    let mut rows: Vec<(String, String)> = Vec::new();
    for i in 0..10050 {
        rows.push((format!("Aaa {i:05}"), format!("u{i:05}")));
    }
    let refs: Vec<(&str, &str)> = rows.iter().map(|(t, u)| (t.as_str(), u.as_str())).collect();
    let c = contentless_db(&refs);
    let (total, more, hits) = browse(&c, "a", false, 0, 10).unwrap();
    assert_eq!(total, 10000, "the count must cap at 10000");
    assert!(more, "a count over the cap must set the more flag");
    assert_eq!(hits.len(), 10);
}
