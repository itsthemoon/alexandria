use crate::{raw_words, same_word, stem};
use rusqlite::Connection;

#[derive(Debug, Clone)]
pub struct Pick {
    pub title: String,
    pub url: String,
    pub score: f64,
}

const PRACTICAL: &[&str] = &["zimgit-", "appropedia", "wikipedia_en_medicine", "post-disaster", "trueprepper"];
const REPAIR: &[&str] = &["fix", "repair", "replace", "replacing", "broken", "battery", "screen", "disassemble", "teardown"];

fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() > 1)
        .map(str::to_string)
        .collect()
}

fn has(tokens: &[String], w: &str) -> bool {
    tokens.iter().any(|t| same_word(t, w))
}

fn passages(text: &str, size: usize) -> Vec<String> {
    let mut units = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let mut part = String::new();
        for w in line.split_whitespace() {
            if !part.is_empty() && part.chars().count() + w.chars().count() + 1 > size {
                units.push(std::mem::take(&mut part));
            }
            if !part.is_empty() {
                part.push(' ');
            }
            part.extend(w.chars().take(size));
        }
        if !part.is_empty() {
            units.push(part);
        }
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    for u in units {
        if !cur.is_empty() && cur.chars().count() + u.chars().count() + 1 > size {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(&u);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn words(q: &str) -> Vec<String> {
    let mut w = raw_words(q);
    w.sort();
    w.dedup();
    w
}

pub fn best_passage(text: &str, q: &str, size: usize) -> String {
    let ws = words(q);
    let mut best = (0, String::new());
    for p in passages(text, size) {
        let pt = tokens(&p);
        let got = ws.iter().filter(|w| has(&pt, w)).count();
        if got > best.0 || best.1.is_empty() {
            best = (got, p);
        }
    }
    best.1
}

fn rows(c: &Connection, fts: &str, limit: usize) -> Result<Vec<(String, String, String)>, String> {
    let sql = format!("SELECT title, url, body FROM articles WHERE articles MATCH ?1 ORDER BY bm25(articles, 10.0, 1.0) LIMIT {limit}");
    let mut st = c.prepare_cached(&sql).map_err(|e| e.to_string())?;
    let out = st
        .query_map([fts], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string());
    out
}

fn count(c: &Connection, fts: &str) -> f64 {
    c.query_row("SELECT count(*) FROM articles WHERE articles MATCH ?1", [fts], |r| r.get::<_, i64>(0))
        .unwrap_or(0) as f64
}

pub fn ask_candidates(c: &Connection, pack: &str, q: &str) -> Result<Vec<Pick>, String> {
    let ws = words(q);
    if ws.is_empty() {
        return Ok(Vec::new());
    }
    let term = |w: &str| format!("\"{}\"*", stem(w));
    let total = c.query_row("SELECT max(rowid) FROM articles", [], |r| r.get::<_, Option<i64>>(0)).ok().flatten().unwrap_or(1).max(1) as f64;
    let idf: Vec<f64> = ws.iter().map(|w| (1.0 + total / (1.0 + count(c, &term(w)))).ln()).collect();
    let mass: f64 = idf.iter().sum();
    let any = ws.iter().map(|w| term(w)).collect::<Vec<_>>().join(" OR ");
    let mut found = rows(c, &any, 12)?;
    found.extend(rows(c, &format!("title : ({any})"), 12)?);
    let mut rare: Vec<usize> = (0..ws.len()).collect();
    rare.sort_by(|a, b| idf[*b].total_cmp(&idf[*a]));
    for &i in rare.iter().take(3) {
        found.extend(rows(c, &format!("title : ^\"{}\"", ws[i]), 5)?);
    }
    let lower = q.to_lowercase();
    let howto = ["how", "what should", "what do i do", "steps", "ways to"].iter().any(|p| lower.trim_start().starts_with(p));
    let repair = tokens(q).iter().any(|t| REPAIR.iter().any(|r| t.starts_with(r)));
    let practical = PRACTICAL.iter().any(|p| pack.starts_with(p));
    let ifixit = pack.starts_with("ifixit");
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (title, url, body) in found {
        if !seen.insert(url.clone()) {
            continue;
        }
        let tt = tokens(&title);
        let tcov: f64 = ws.iter().zip(&idf).filter(|(w, _)| has(&tt, w)).map(|(_, i)| i).sum::<f64>() / mass;
        let focus = tt.iter().filter(|t| ws.iter().any(|w| same_word(t, w))).count() as f64 / tt.len().max(1) as f64;
        let bscore = passages(&body, 700)
            .iter()
            .map(|p| {
                let pt = tokens(p);
                ws.iter().zip(&idf).filter(|(w, _)| has(&pt, w)).map(|(_, i)| i).sum::<f64>() / mass
            })
            .fold(0.0, f64::max);
        let low = title.to_lowercase();
        let junk = title.contains('(') || title.contains('/') || low.contains("disambiguation") || low.contains("quiz");
        let mut score = 3.0 * tcov + focus + 2.0 * bscore - if junk { 1.5 } else { 0.0 } + if focus == 1.0 { 1.0 } else { 0.0 };
        if howto && practical {
            score += 0.4;
        }
        if ifixit {
            score += if repair { 0.4 } else { -1.0 };
        }
        out.push(Pick { title, url, score });
    }
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    Ok(out)
}

fn phrase_in(hay: &[String], needle: &[String]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

pub fn pick_sources(mut all: Vec<(String, Pick)>, n: usize, q: &str) -> Vec<(String, Pick)> {
    all.sort_by(|a, b| b.1.score.total_cmp(&a.1.score));
    let top = all.first().map(|p| p.1.score).unwrap_or(0.0);
    let qt = tokens(q);
    let topic = all
        .iter()
        .map(|(_, p)| tokens(&p.title))
        .filter(|t| t.len() >= 2 && phrase_in(&qt, t))
        .max_by_key(|t| t.len());
    let mut seen = std::collections::HashSet::new();
    all.into_iter()
        .filter(|(_, p)| p.score >= 0.7 * top)
        .filter(|(_, p)| topic.as_ref().map_or(true, |t| phrase_in(&tokens(&p.title), t)))
        .filter(|(_, p)| seen.insert(p.title.to_lowercase()))
        .take(n)
        .collect()
}
