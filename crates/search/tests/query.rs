use alexandria_search::query;
use rusqlite::Connection;

fn db(rows: &[(&str, &str)]) -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(
        "CREATE VIRTUAL TABLE articles USING fts5(title, body, url UNINDEXED, tokenize='unicode61 remove_diacritics 2');",
    )
    .unwrap();
    for (t, b) in rows {
        c.execute("INSERT INTO articles(title, body, url) VALUES (?1, ?2, ?3)", rusqlite::params![t, b, t.replace(' ', "_")]).unwrap();
    }
    c
}

#[test]
fn hits_say_whether_they_matched_every_word() {
    let c = db(&[
        ("Dehydration", "To treat dehydration, drink fluids."),
        ("How to replace your iPhone 5 battery", "Treat the battery gently."),
    ]);
    let hits = query(&c, "How do I treat dehydration?").unwrap();
    let all = |t: &str| hits.iter().find(|h| h.title == t).map(|h| h.all);
    assert_eq!(all("Dehydration"), Some(true));
    assert_eq!(all("How to replace your iPhone 5 battery"), Some(false));
}
