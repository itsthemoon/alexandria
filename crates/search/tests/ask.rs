use alexandria_search::{ask_candidates, best_passage, pick_sources, Pick};
use rusqlite::Connection;

fn db(rows: &[(&str, &str)]) -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(
        "CREATE VIRTUAL TABLE articles USING fts5(title, body, url UNINDEXED, tokenize='unicode61 remove_diacritics 2');",
    )
    .unwrap();
    for i in 0..300 {
        c.execute(
            "INSERT INTO articles(title, body, url) VALUES (?1, ?2, ?3)",
            rusqlite::params![format!("Topic {i}"), format!("how to treat and make and use things, number {i}"), format!("Topic_{i}")],
        )
        .unwrap();
    }
    for (t, b) in rows {
        c.execute("INSERT INTO articles(title, body, url) VALUES (?1, ?2, ?3)", rusqlite::params![t, b, t.replace(' ', "_")]).unwrap();
    }
    c
}

fn fever() -> Connection {
    db(&[
        ("Fever (song)", "Fever is a song. Fever fever fever, the singer treats the crowd to fever."),
        ("Fever", "A fever is a body temperature above normal. To treat a fever, rest and drink fluids."),
        ("Yellow fever", "Yellow fever is a viral disease spread by mosquitoes."),
    ])
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
fn candidates_are_found_on_a_contentless_index() {
    let c = contentless_db(&[
        ("Water purification", "Water purification removes contaminants from water. Boiling kills germs and parasites."),
        ("Dysentery", "Dysentery is an infection of the intestines that causes bloody diarrhoea."),
        ("Vibrio", "Dysentery is caused by bacteria in this genus."),
    ]);
    let picks = ask_candidates(&c, "wikipedia_en", "How do I purify water?", &|_| None).unwrap();
    assert_eq!(picks[0].title, "Water purification");
    let d = ask_candidates(&c, "wikipedia_en", "What causes dysentery?", &|_| None).unwrap();
    assert_eq!(d[0].title, "Dysentery");
}

#[test]
fn contentless_body_coverage_is_scored_from_the_fetcher() {
    let c = contentless_db(&[("Vibrio", "Dysentery is caused by bacteria in this genus.")]);
    let fetch = |_url: &str| Some("Dysentery is caused by bacteria in this genus.".to_string());
    let with = ask_candidates(&c, "wikipedia_en", "What causes dysentery?", &fetch).unwrap();
    let without = ask_candidates(&c, "wikipedia_en", "What causes dysentery?", &|_| None).unwrap();
    assert!(
        with[0].score > without[0].score,
        "body coverage must raise the score: {} vs {}",
        with[0].score,
        without[0].score
    );
}

#[test]
fn parallel_body_fetch_is_deduped_and_orders_correctly() {
    let c = contentless_db(&[
        ("Water purification", "Water purification removes contaminants from water. Boiling kills germs."),
        ("Dysentery", "Dysentery is an infection of the intestines that causes bloody diarrhoea."),
        ("Vibrio", "Dysentery is caused by bacteria in this genus."),
    ]);
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let fetch = |url: &str| {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Some(format!("text about {url}"))
    };
    let picks = ask_candidates(&c, "wikipedia_en", "How do I purify water?", &fetch).unwrap();
    assert_eq!(picks[0].title, "Water purification");
    let n = calls.load(std::sync::atomic::Ordering::SeqCst);
    assert!((1..=3).contains(&n), "fetched {n} bodies for 3 unique candidates");
}

#[test]
fn the_article_beats_a_song_with_the_same_name() {
    let c = fever();
    let picks = ask_candidates(&c, "wikipedia_en-simple", "How do I treat a fever?", &|_| None).unwrap();
    let rank = |t: &str| picks.iter().position(|p| p.title == t).unwrap();
    assert_eq!(picks[0].title, "Fever");
    assert!(rank("Fever (song)") > rank("Yellow fever") || picks[rank("Fever (song)")].score < picks[0].score * 0.7);
}

#[test]
fn how_to_questions_prefer_practical_guides() {
    let c = fever();
    let q = "How do I treat a fever?";
    let top = |pack: &str| ask_candidates(&c, pack, q, &|_| None).unwrap()[0].score;
    assert!(top("zimgit-medicine_en") > top("wikipedia_en-simple"));
    let fact = "What is a fever?";
    let top = |pack: &str| ask_candidates(&c, pack, fact, &|_| None).unwrap()[0].score;
    assert_eq!(top("zimgit-medicine_en"), top("wikipedia_en-simple"));
}

#[test]
fn repair_guides_count_only_for_repair_questions() {
    let c = db(&[("Radio", "A two-way radio sends and receives. Replace the battery if it is broken.")]);
    let score = |q: &str, pack: &str| ask_candidates(&c, pack, q, &|_| None).unwrap()[0].score;
    assert!(score("How do I use a radio?", "ifixit_en") < score("How do I use a radio?", "wikipedia_en-simple"));
    assert!(score("How do I fix a broken radio?", "ifixit_en") > score("How do I fix a broken radio?", "wikipedia_en-simple"));
}

#[test]
fn no_question_words_means_no_candidates() {
    assert!(ask_candidates(&fever(), "wikipedia_en-simple", "how do I", &|_| None).unwrap().is_empty());
}

fn pick(t: &str, s: f64) -> Pick {
    Pick { title: t.into(), url: t.into(), score: s }
}

#[test]
fn weak_extra_sources_and_duplicates_are_dropped() {
    let all = vec![
        ("a".to_string(), pick("Fever", 5.0)),
        ("b".to_string(), pick("fever", 4.9)),
        ("a".to_string(), pick("Hyperthermia", 3.8)),
        ("b".to_string(), pick("Fever (song)", 3.0)),
        ("a".to_string(), pick("Stroke 9", 1.0)),
    ];
    let got: Vec<String> = pick_sources(all, 3, "How do I treat a fever?").into_iter().map(|(_, p)| p.title).collect();
    assert_eq!(got, vec!["Fever", "Hyperthermia"]);
}

#[test]
fn a_named_topic_keeps_look_alike_titles_out() {
    let all = vec![
        ("med".to_string(), pick("Heat stroke", 5.0)),
        ("simple".to_string(), pick("Heat treating", 4.8)),
        ("simple".to_string(), pick("Heat stroke in dogs", 4.0)),
    ];
    let got: Vec<String> = pick_sources(all, 3, "How do I treat heat stroke?").into_iter().map(|(_, p)| p.title).collect();
    assert_eq!(got, vec!["Heat stroke", "Heat stroke in dogs"]);
}

#[test]
fn the_passage_that_answers_is_chosen_not_the_opening() {
    let text = format!(
        "{}\nTo make water safe, boil it for one full minute and let it cool.\n{}",
        "Water covers most of the planet and has many uses in industry.\n".repeat(20),
        "Rivers flow into the sea.\n".repeat(20)
    );
    let p = best_passage(&text, "How do I make water safe to drink?", 600);
    assert!(p.contains("boil it for one full minute"), "{p}");
    assert!(p.chars().count() <= 600);
}

#[test]
fn a_short_text_is_its_own_passage() {
    assert_eq!(best_passage("Boil water.", "boil", 600), "Boil water.");
}

fn sparse_db(ids: &[i64], meta_articles: Option<i64>) -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(
        "CREATE VIRTUAL TABLE articles USING fts5(title, body, content='', tokenize='unicode61 remove_diacritics 2');
         CREATE TABLE docs(id INTEGER PRIMARY KEY, title TEXT NOT NULL, url TEXT NOT NULL);
         CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )
    .unwrap();
    let rows = [
        ("Fever (song)", "Fever is a song. Fever fever fever, the singer treats the crowd to fever."),
        ("Fever", "A fever is a body temperature above normal. To treat a fever, rest and drink fluids."),
        ("Yellow fever", "Yellow fever is a viral disease spread by mosquitoes."),
    ];
    for (i, (t, b)) in rows.iter().enumerate() {
        let id = ids[i];
        c.execute("INSERT INTO docs(id, title, url) VALUES (?1, ?2, ?3)", rusqlite::params![id, t, t.replace(' ', "_")]).unwrap();
        c.execute("INSERT INTO articles(rowid, title, body) VALUES (?1, ?2, ?3)", rusqlite::params![id, t, b]).unwrap();
    }
    if let Some(n) = meta_articles {
        c.execute("INSERT INTO meta(key, value) VALUES ('articles', ?1)", rusqlite::params![n.to_string()]).unwrap();
    }
    c
}

#[test]
fn ask_uses_the_meta_article_count_not_the_max_rowid() {
    let sparse = sparse_db(&[1000, 1001, 1002], Some(3));
    let dense = sparse_db(&[1, 2, 3], None);
    let q = "How do I treat a fever?";
    let a: Vec<Pick> = ask_candidates(&sparse, "wikipedia_en-simple", q, &|_| None).unwrap();
    let b: Vec<Pick> = ask_candidates(&dense, "wikipedia_en-simple", q, &|_| None).unwrap();
    let scores = |v: &[Pick]| v.iter().map(|p| p.score).collect::<Vec<_>>();
    assert_eq!(
        scores(&a),
        scores(&b),
        "meta article count must make sparse rowids score like a dense index"
    );
    assert!(!a.is_empty());
}
