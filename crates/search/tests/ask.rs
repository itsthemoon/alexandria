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

#[test]
fn the_article_beats_a_song_with_the_same_name() {
    let c = fever();
    let picks = ask_candidates(&c, "wikipedia_en-simple", "How do I treat a fever?").unwrap();
    let rank = |t: &str| picks.iter().position(|p| p.title == t).unwrap();
    assert_eq!(picks[0].title, "Fever");
    assert!(rank("Fever (song)") > rank("Yellow fever") || picks[rank("Fever (song)")].score < picks[0].score * 0.7);
}

#[test]
fn how_to_questions_prefer_practical_guides() {
    let c = fever();
    let q = "How do I treat a fever?";
    let top = |pack: &str| ask_candidates(&c, pack, q).unwrap()[0].score;
    assert!(top("zimgit-medicine_en") > top("wikipedia_en-simple"));
    let fact = "What is a fever?";
    let top = |pack: &str| ask_candidates(&c, pack, fact).unwrap()[0].score;
    assert_eq!(top("zimgit-medicine_en"), top("wikipedia_en-simple"));
}

#[test]
fn repair_guides_count_only_for_repair_questions() {
    let c = db(&[("Radio", "A two-way radio sends and receives. Replace the battery if it is broken.")]);
    let score = |q: &str, pack: &str| ask_candidates(&c, pack, q).unwrap()[0].score;
    assert!(score("How do I use a radio?", "ifixit_en") < score("How do I use a radio?", "wikipedia_en-simple"));
    assert!(score("How do I fix a broken radio?", "ifixit_en") > score("How do I fix a broken radio?", "wikipedia_en-simple"));
}

#[test]
fn no_question_words_means_no_candidates() {
    assert!(ask_candidates(&fever(), "wikipedia_en-simple", "how do I").unwrap().is_empty());
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
