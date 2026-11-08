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

fn contentless_db(rows: &[(&str, &str)]) -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(
        "CREATE VIRTUAL TABLE articles USING fts5(title, body, content='', tokenize='unicode61 remove_diacritics 2');
         CREATE TABLE docs(id INTEGER PRIMARY KEY, title TEXT NOT NULL, url TEXT NOT NULL);",
    )
    .unwrap();
    for (t, b) in rows {
        c.execute("INSERT INTO docs(title, url) VALUES (?1, ?2)", rusqlite::params![t, t.replace(' ', "_")]).unwrap();
        let id = c.last_insert_rowid();
        c.execute("INSERT INTO articles(rowid, title, body) VALUES (?1, ?2, ?3)", rusqlite::params![id, t, b]).unwrap();
    }
    c
}

#[test]
fn a_contentless_index_returns_the_same_top_results_as_the_old_layout() {
    let rows: Vec<(&str, &str)> = vec![
        ("Dehydration", "To treat dehydration, drink fluids and rest in the shade."),
        ("Water purification", "Water purification removes contaminants from water. Boiling kills germs and parasites."),
        ("Dysentery", "Dysentery is an infection of the intestines that causes bloody diarrhoea."),
        ("Heat stroke", "Heat stroke is a severe form of heat illness when the body overheats dangerously."),
        ("Tourniquet", "A tourniquet stops bleeding from a wounded limb in an emergency."),
        ("Photosynthesis", "Photosynthesis is how plants turn sunlight, water and carbon dioxide into sugar."),
        ("Insulin", "Insulin is a hormone that regulates blood sugar levels in the body."),
        ("Paris", "Paris is the capital city of France on the river Seine."),
        ("Tokyo", "Tokyo is the capital city of Japan and one of the largest cities."),
        ("Bread", "Bread is a staple food baked from flour, water and yeast."),
        ("How to start a fire", "To start a fire, gather dry tinder, kindling and fuel wood, then strike a spark."),
        ("World War II", "World War II was a global war fought from 1939 to 1945 between the Allies and the Axis."),
    ];
    let old = db(&rows);
    let new = contentless_db(&rows);
    let queries = [
        "dehydration",
        "water purification",
        "tourniquet",
        "paris",
        "photosynthesis",
        "world war ii",
        "how to start a fire",
        "insulin",
        "tokyo",
        "bread",
        "heat stroke",
        "dysentery",
        "how do I treat dehydration",
        "capital city",
    ];
    for q in queries {
        let a: Vec<String> = query(&old, q).unwrap().into_iter().map(|h| h.title).collect();
        let b: Vec<String> = query(&new, q).unwrap().into_iter().map(|h| h.title).collect();
        assert_eq!(a, b, "query {q:?} returned different top results");
    }
}

#[test]
fn title_urls_guess_the_article_url_for_an_exact_title() {
    assert_eq!(alexandria_search::title_urls("paris")[0], "Paris");
    assert_eq!(alexandria_search::title_urls("heat stroke"), vec!["Heat_stroke".to_string()]);
    assert_eq!(alexandria_search::title_urls("burns"), vec!["Burns".to_string(), "Burn".to_string()]);
}
