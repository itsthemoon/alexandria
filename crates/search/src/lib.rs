use alexandria_core::VerifiedFile;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

mod ask;
pub mod ocr;
pub mod streets;
mod vfs;

pub use ask::{ask_candidates, best_passage, pick_sources, Pick};

struct Reader {
    vf: VerifiedFile,
    chunk: u64,
    size: u64,
}

impl Reader {
    fn read(&mut self, offset: u64, out: &mut [u8]) -> Option<usize> {
        let mut done = 0usize;
        while done < out.len() {
            let pos = offset + done as u64;
            if pos >= self.size {
                break;
            }
            let ci = pos / self.chunk;
            let from = (pos - ci * self.chunk) as usize;
            let off = ci * self.chunk;
            let len = std::cmp::min(self.chunk, self.size - off) as usize;
            let buf = self.vf.read_at(off, len).ok()?;
            let take = std::cmp::min(buf.len() - from, out.len() - done);
            out[done..done + take].copy_from_slice(&buf[from..from + take]);
            done += take;
        }
        Some(done)
    }
}

type Shared = Arc<Mutex<Reader>>;

static REGISTRY: Mutex<Option<HashMap<String, Shared>>> = Mutex::new(None);
static NEXT: Mutex<u64> = Mutex::new(0);

pub struct VerifiedDb {
    conn: Option<rusqlite::Connection>,
    token: String,
}

impl VerifiedDb {
    pub fn conn(&self) -> &rusqlite::Connection {
        self.conn.as_ref().expect("connection is open until drop")
    }
}

impl Drop for VerifiedDb {
    fn drop(&mut self) {
        self.conn.take();
        if let Ok(mut g) = REGISTRY.lock() {
            if let Some(map) = g.as_mut() {
                map.remove(&self.token);
            }
        }
    }
}

pub fn open_verified(vf: VerifiedFile, chunk: u64) -> Result<VerifiedDb, String> {
    vfs::register()?;
    let size = vf.size();
    let reader = Arc::new(Mutex::new(Reader {
        vf,
        chunk,
        size,
    }));
    let token = {
        let mut n = NEXT.lock().map_err(|_| "search lock")?;
        *n += 1;
        format!("alexv-{}", *n)
    };
    REGISTRY
        .lock()
        .map_err(|_| "search lock")?
        .get_or_insert_with(HashMap::new)
        .insert(token.clone(), reader);
    let mut db = VerifiedDb { conn: None, token };
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
        | rusqlite::OpenFlags::SQLITE_OPEN_URI
        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let uri = format!("file:{}?immutable=1", db.token);
    let conn = rusqlite::Connection::open_with_flags_and_vfs(&uri, flags, vfs::NAME)
        .map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA query_only=1;")
        .map_err(|e| e.to_string())?;
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))
        .map_err(|e| e.to_string())?;
    db.conn = Some(conn);
    Ok(db)
}

fn file_uri(path: &std::path::Path) -> String {
    let lossy = path.to_string_lossy();
    let stripped = alexandria_core::strip_verbatim(&lossy);
    let mut p = String::with_capacity(stripped.len() + 8);
    for b in stripped.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => p.push(b as char),
            b'\\' => p.push('/'),
            _ => p.push_str(&format!("%{b:02X}")),
        }
    }
    if p.starts_with('/') {
        format!("file://{p}")
    } else {
        format!("file:///{p}")
    }
}

pub fn open_local(path: &std::path::Path) -> Result<VerifiedDb, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err("index is not a regular file".into());
    }
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
        | rusqlite::OpenFlags::SQLITE_OPEN_URI
        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = rusqlite::Connection::open_with_flags(format!("{}?immutable=1", file_uri(path)), flags)
        .map_err(|e| e.to_string())?;
    conn.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)
        .map_err(|e| e.to_string())?;
    conn.execute_batch(
        "PRAGMA temp_store=MEMORY; PRAGMA query_only=1; PRAGMA trusted_schema=OFF; PRAGMA cell_size_check=ON;",
    )
    .map_err(|e| e.to_string())?;
    conn.query_row("SELECT max(rowid) FROM articles", [], |r| r.get::<_, Option<i64>>(0))
        .map_err(|e| e.to_string())?;
    Ok(VerifiedDb { conn: Some(conn), token: String::new() })
}

pub fn local_index_path(library: &std::path::Path, pack: &alexandria_core::Entry) -> std::path::PathBuf {
    let parent = library.parent().unwrap_or(library);
    let name = library.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tag = alexandria_core::hex(&pack.hashes_sha256);
    parent
        .join(format!("{name}-Index"))
        .join(format!("{}.{}.search.db", pack.path, &tag[..16]))
}

pub fn snippet(text: &str, q: &str) -> Option<String> {
    let words = raw_words(q);
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let hit = tokens.iter().position(|t| {
        let w = t.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
        words.iter().any(|q| same_word(&w, q))
    })?;
    let start = hit.saturating_sub(8);
    let mut out = String::new();
    let mut used = 0;
    for t in &tokens[start..] {
        if out.chars().count() + t.chars().count() + 1 > 190 {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(t);
        used += 1;
    }
    let before = if start > 0 { "…" } else { "" };
    let after = if start + used < tokens.len() { "…" } else { "" };
    Some(format!("{before}{out}{after}"))
}

static PDF_HOOK_LOCK: Mutex<()> = Mutex::new(());

pub fn pdf_text(bytes: &[u8]) -> Option<(Option<String>, String)> {
    let _guard = PDF_HOOK_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let out = pdf_extract_quiet(bytes);
    std::panic::set_hook(hook);
    out
}

fn pdf_extract_quiet(bytes: &[u8]) -> Option<(Option<String>, String)> {
    let bytes = bytes.to_vec();
    let out = std::panic::catch_unwind(move || {
        let text = pdf_extract::extract_text_from_mem(&bytes).ok()?;
        let meta = pdf_extract::Document::load_mem(&bytes).ok().and_then(|d| {
            let info = d.trailer.get(b"Info").ok()?.as_reference().ok()?;
            let dict = d.get_object(info).ok()?.as_dict().ok()?;
            let t = dict.get(b"Title").ok()?.as_str().ok()?;
            Some(String::from_utf8_lossy(t).to_string())
        });
        Some((meta, text))
    });
    out.ok().flatten()
}

pub fn pdf_text_only(bytes: &[u8]) -> Option<String> {
    let bytes = bytes.to_vec();
    std::panic::catch_unwind(move || pdf_extract::extract_text_from_mem(&bytes).ok()).ok().flatten()
}

pub fn pdf_titles(items: Vec<(String, Vec<u8>)>) -> Vec<String> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).max(1);
    let per = items.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let jobs: Vec<_> = items
            .chunks(per)
            .map(|chunk| {
                let job = s.spawn(move || {
                    chunk
                        .iter()
                        .map(|(name, bytes)| match pdf_extract_quiet(bytes) {
                            Some((meta, text)) => pdf_title(meta.as_deref(), &text, name),
                            None => pdf_name(name),
                        })
                        .collect::<Vec<_>>()
                });
                (chunk, job)
            })
            .collect();
        jobs.into_iter()
            .flat_map(|(chunk, job)| job.join().unwrap_or_else(|_| chunk.iter().map(|(n, _)| pdf_name(n)).collect()))
            .collect()
    })
}

pub fn pdf_title(meta: Option<&str>, text: &str, name: &str) -> String {
    let clean = |t: &str| {
        let t = t.trim();
        t.split_whitespace().count() >= 2
            && t.len() <= 120
            && !t.contains("://")
            && !t.to_lowercase().ends_with(".pdf")
            && !t.to_lowercase().ends_with(".doc")
            && !t.to_lowercase().ends_with(".docx")
            && !t.starts_with("Microsoft Word")
            && t.chars().all(|c| !c.is_control() && c != '\u{fffd}')
    };
    if let Some(m) = meta.filter(|m| clean(m)) {
        return m.trim().to_string();
    }
    if let Some(line) = text.lines().map(str::trim).find(|l| l.split_whitespace().count() >= 3) {
        let mut out = String::new();
        for w in line.split_whitespace() {
            if out.len() + w.len() + 1 > 80 {
                break;
            }
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(w);
        }
        return out;
    }
    pdf_name(name)
}

pub fn pdf_name(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    base.strip_suffix(".pdf").unwrap_or(base).to_string()
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub all: bool,
}

const STOP: &[&str] = &[
    "a", "an", "and", "are", "can", "do", "does", "for", "from", "how", "i", "in", "is", "it",
    "me", "my", "of", "on", "or", "the", "to", "was", "what", "when", "where", "which", "who",
    "why", "with", "you", "tell", "about", "explain", "cause", "causes", "caused", "happens", "happen",
    "mean", "means", "work", "works", "without", "using", "use", "get", "make",
    "need", "best", "way", "ways", "should", "could", "would", "there", "this", "that", "they",
    "them", "did", "has", "have", "be", "been", "if", "as", "at", "by", "into", "than", "then",
    "so", "not", "no", "any", "some", "your", "our", "we", "much", "many", "very",
];

pub(crate) fn stem(t: &str) -> String {
    let n = t.chars().count();
    if n >= 6 {
        t.chars().take(std::cmp::max(5, n * 3 / 4)).collect()
    } else {
        t.to_string()
    }
}

pub(crate) fn same_word(a: &str, b: &str) -> bool {
    fn base(w: &str) -> String {
        for (suffix, with) in [("ying", "y"), ("ied", "y"), ("ies", "y"), ("ing", ""), ("ed", ""), ("es", ""), ("s", "")] {
            if let Some(stem) = w.strip_suffix(suffix) {
                if stem.chars().count() >= 3 {
                    return format!("{stem}{with}");
                }
            }
        }
        w.to_string()
    }
    a == b || base(a) == base(b) || base(a) == b || base(b) == a
}

pub(crate) fn raw_words(q: &str) -> Vec<String> {
    q.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() > 1)
        .map(|t| t.to_lowercase())
        .filter(|t| !STOP.contains(&t.as_str()))
        .collect()
}

fn terms(q: &str) -> Vec<String> {
    let all: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() > 1)
        .map(|t| t.to_lowercase())
        .collect();
    let content: Vec<String> = all.iter().filter(|t| !STOP.contains(&t.as_str())).cloned().collect();
    let kept = if content.is_empty() { all } else { content };
    kept.iter().map(|t| stem(t)).collect()
}

fn join(terms: &[String], op: &str) -> String {
    if terms.is_empty() {
        return "\"\"".to_string();
    }
    terms
        .iter()
        .map(|t| format!("\"{t}\"*"))
        .collect::<Vec<_>>()
        .join(op)
}

pub fn fts_query(q: &str) -> String {
    join(&terms(q), " AND ")
}

fn exact_title(q: &str) -> String {
    let t = terms(q);
    if t.len() == 1 {
        t[0].clone()
    } else {
        q.trim().trim_end_matches('?').to_string()
    }
}

pub(crate) fn has_docs(conn: &rusqlite::Connection) -> bool {
    conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'docs'",
        [],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}

pub(crate) fn meta_str(conn: &rusqlite::Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .ok()
}

pub(crate) fn meta_int(conn: &rusqlite::Connection, key: &str) -> Option<i64> {
    meta_str(conn, key).and_then(|v| v.parse().ok())
}

pub(crate) fn title_ordered(conn: &rusqlite::Connection) -> bool {
    meta_str(conn, "order").as_deref() == Some("title")
}

fn title_query(q: &str) -> String {
    let tokens: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect();
    if tokens.is_empty() {
        return String::new();
    }
    tokens
        .iter()
        .map(|t| format!("title : \"{t}\"*"))
        .collect::<Vec<_>>()
        .join(" AND ")
}

const COUNT_CAP: usize = 10_000;

pub fn browse(
    conn: &rusqlite::Connection,
    needle: &str,
    reverse: bool,
    offset: usize,
    limit: usize,
) -> Result<(usize, bool, Vec<Hit>), String> {
    let fts = title_query(needle);
    if fts.is_empty() {
        return Ok((0, false, Vec::new()));
    }
    let contentless = has_docs(conn);
    let title_col = if contentless { "docs.title" } else { "articles.title" };
    let url_col = if contentless { "docs.url" } else { "articles.url" };
    let from = if contentless {
        " FROM articles JOIN docs ON docs.id = articles.rowid "
    } else {
        " FROM articles "
    };
    let n = conn
        .query_row(
            &format!(
                "SELECT count(*) FROM (SELECT rowid FROM articles WHERE articles MATCH ?1 LIMIT {})",
                COUNT_CAP + 1
            ),
            rusqlite::params![fts],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n as usize)
        .map_err(|e| e.to_string())?;
    let more = n > COUNT_CAP;
    let total = n.min(COUNT_CAP);
    let order = if reverse { "DESC" } else { "ASC" };
    let sql = if contentless && title_ordered(conn) {
        format!(
            "SELECT {title_col}, {url_col} {from} WHERE articles MATCH ?1
             ORDER BY articles.rowid {order} LIMIT ?2 OFFSET ?3"
        )
    } else {
        format!(
            "SELECT {title_col}, {url_col} {from} WHERE articles MATCH ?1
             ORDER BY {title_col} COLLATE NOCASE {order} LIMIT ?2 OFFSET ?3"
        )
    };
    let mut stmt = conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            rusqlite::params![fts, limit as i64, offset as i64],
            |row| {
                Ok(Hit {
                    title: row.get(0)?,
                    url: row.get(1)?,
                    all: true,
                })
            },
        )
        .map_err(|e| e.to_string())?;
    let items = rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    Ok((total, more, items))
}

pub fn title_urls(q: &str) -> Vec<String> {
    let t = exact_title(q);
    let mut out: Vec<String> = Vec::new();
    for s in [t.clone(), singular(&t)] {
        let mut c = s.chars();
        let Some(f) = c.next() else { continue };
        let u = f.to_uppercase().chain(c).collect::<String>().replace(' ', "_");
        if !out.contains(&u) {
            out.push(u);
        }
    }
    out
}

pub fn query(conn: &rusqlite::Connection, q: &str) -> Result<Vec<Hit>, String> {
    if raw_words(q).is_empty() {
        return Ok(Vec::new());
    }
    let mut hits = run_query(conn, &fts_query(q), q, true)?;
    let t = terms(q);
    if hits.len() < 3 && t.len() > 1 {
        for h in run_query(conn, &join(&t, " OR "), q, false)? {
            if !hits.iter().any(|x| x.url == h.url) {
                hits.push(h);
            }
        }
    }
    Ok(hits)
}

fn singular(s: &str) -> String {
    if let Some(stem) = s.strip_suffix("ies") {
        if stem.chars().count() >= 2 {
            return format!("{stem}y");
        }
    }
    match s.strip_suffix('s') {
        Some(stem) if !stem.ends_with('s') && stem.chars().count() >= 3 => stem.to_string(),
        _ => s.to_string(),
    }
}

const EXACT_CAP_SMALL: i64 = 50_000;
const EXACT_CAP_BIG: i64 = 5_000;
const BIG_INDEX: i64 = 1_000_000;
const TITLE_CANDIDATES: usize = 200;
const BODY_CANDIDATES: usize = 2000;
const BIG_MATCHES: i64 = 100_000;

fn run_query(conn: &rusqlite::Connection, fts: &str, q: &str, all: bool) -> Result<Vec<Hit>, String> {
    let contentless = has_docs(conn);
    let t: Vec<String> = terms(q).into_iter().take(8).collect();
    if t.is_empty() {
        return Ok(Vec::new());
    }
    if contentless {
        return run_query_contentless(conn, fts, q, all, &t);
    }
    exact_query(conn, fts, q, all, &t, "articles.title", "articles.url", " FROM articles ")
}

fn exact_query(
    conn: &rusqlite::Connection,
    fts: &str,
    q: &str,
    all: bool,
    t: &[String],
    title_col: &str,
    url_col: &str,
    from: &str,
) -> Result<Vec<Hit>, String> {
    let title_hits = (0..t.len())
        .map(|i| format!("(instr(lower({title_col}), ?{}) > 0)", i + 3))
        .collect::<Vec<_>>()
        .join(" + ");
    let singular_at = t.len() + 3;
    let sql = format!(
        "SELECT {title_col}, {url_col}{from}
         WHERE articles MATCH ?1
         ORDER BY ({title_col} = ?2 COLLATE NOCASE OR {title_col} = ?{singular_at} COLLATE NOCASE) DESC,
                  ({title_hits}) DESC, bm25(articles, 10.0, 1.0)
         LIMIT 40"
    );
    let mut stmt = conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
    let exact = exact_title(q);
    let singular = singular(&exact);
    let mut params: Vec<String> = vec![fts.to_string(), exact];
    params.extend(t.iter().cloned());
    params.push(singular);
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            Ok(Hit {
                title: row.get(0)?,
                url: row.get(1)?,
                all,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

fn run_query_contentless(
    conn: &rusqlite::Connection,
    fts: &str,
    q: &str,
    all: bool,
    t: &[String],
) -> Result<Vec<Hit>, String> {
    let m = conn
        .query_row(
            &format!("SELECT count(*) FROM (SELECT rowid FROM articles WHERE articles MATCH ?1 LIMIT {})", BIG_MATCHES + 1),
            [fts],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| e.to_string())?;
    let max_id = conn
        .query_row("SELECT max(id) FROM docs", [], |r| r.get::<_, Option<i64>>(0))
        .ok()
        .flatten()
        .unwrap_or(0);
    let exact_cap = if max_id > BIG_INDEX { EXACT_CAP_BIG } else { EXACT_CAP_SMALL };
    if m <= exact_cap {
        return exact_query(
            conn,
            fts,
            q,
            all,
            t,
            "docs.title",
            "docs.url",
            " FROM articles JOIN docs ON docs.id = articles.rowid ",
        );
    }
    let big = m > BIG_MATCHES;
    let mut scores: HashMap<i64, f64> = HashMap::new();
    let title_fts = format!("title : ({fts})");
    let mut st = conn
        .prepare_cached(&format!(
            "SELECT rowid, bm25(articles, 10.0, 1.0) FROM articles WHERE articles MATCH ?1 ORDER BY bm25(articles, 10.0, 1.0) LIMIT {TITLE_CANDIDATES}"
        ))
        .map_err(|e| e.to_string())?;
    let title_rows = st
        .query_map([&title_fts], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)))
        .map_err(|e| e.to_string())?;
    for r in title_rows {
        let (id, s) = r.map_err(|e| e.to_string())?;
        scores.entry(id).or_insert(s);
    }
    if !big {
        let mut st = conn
            .prepare_cached(&format!(
                "SELECT rowid, bm25(articles, 10.0, 1.0) FROM articles WHERE articles MATCH ?1 ORDER BY bm25(articles, 10.0, 1.0) LIMIT {BODY_CANDIDATES}"
            ))
            .map_err(|e| e.to_string())?;
        let body_rows = st
            .query_map([fts], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)))
            .map_err(|e| e.to_string())?;
        for r in body_rows {
            let (id, s) = r.map_err(|e| e.to_string())?;
            scores.insert(id, s);
        }
    }
    if scores.is_empty() {
        return Ok(Vec::new());
    }
    let exact = exact_title(q);
    let singular = singular(&exact);
    let mut docs = conn
        .prepare_cached("SELECT title, url FROM docs WHERE id = ?1")
        .map_err(|e| e.to_string())?;
    let mut hits = Vec::with_capacity(scores.len());
    for (&id, &score) in &scores {
        let Ok((title, url)) = docs.query_row([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) else {
            continue;
        };
        let lower = title.to_lowercase();
        let title_hits = t.iter().filter(|w| lower.contains(w.as_str())).count();
        let exact_hit = lower == exact || lower == singular;
        hits.push((exact_hit, title_hits, score, title, url));
    }
    hits.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| a.3.cmp(&b.3))
    });
    Ok(hits
        .into_iter()
        .take(40)
        .map(|(_, _, _, title, url)| Hit { title, url, all })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{file_uri, pdf_text};
    use std::path::Path;

    #[test]
    fn file_uri_percent_encodes_the_path() {
        assert_eq!(file_uri(Path::new("/a%z.db")), "file:///a%25z.db");
        assert_eq!(file_uri(Path::new("/a b.db")), "file:///a%20b.db");
        assert_eq!(file_uri(Path::new("/a#b.db")), "file:///a%23b.db");
        assert_eq!(file_uri(Path::new("/a?b.db")), "file:///a%3Fb.db");
    }

    #[test]
    fn file_uri_handles_windows_drive_paths() {
        assert_eq!(file_uri(Path::new(r"C:\Data\index.db")), "file:///C:/Data/index.db");
    }

    #[test]
    fn file_uri_strips_the_windows_verbatim_prefix() {
        assert_eq!(file_uri(Path::new(r"\\?\C:\x")), "file:///C:/x");
        assert_eq!(file_uri(Path::new(r"\\?\UNC\server\share\index.db")), "file:////server/share/index.db");
    }

    #[test]
    fn pdf_text_suppresses_and_restores_the_panic_hook() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static FLAG: AtomicBool = AtomicBool::new(false);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| FLAG.store(true, Ordering::SeqCst)));
        let _ = pdf_text(b"not a pdf");
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| assert_eq!(pdf_text(b"also not a pdf"), None));
            }
        });
        let caught = std::panic::catch_unwind(|| panic!("probe"));
        assert!(caught.is_err());
        assert!(FLAG.load(Ordering::SeqCst), "the original panic hook was not restored");
        std::panic::set_hook(previous);
    }
}
