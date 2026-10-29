use alexandria_core::VerifiedFile;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

mod ask;
mod vfs;

pub use ask::{ask_candidates, best_passage, pick_sources, Pick};

const CACHE_CHUNKS: usize = 32;

struct Reader {
    vf: VerifiedFile,
    chunk: u64,
    size: u64,
    cache: Vec<(u64, Vec<u8>)>,
}

impl Reader {
    fn chunk_bytes(&mut self, ci: u64) -> Option<&[u8]> {
        if let Some(pos) = self.cache.iter().position(|(i, _)| *i == ci) {
            let hit = self.cache.remove(pos);
            self.cache.push(hit);
        } else {
            let off = ci * self.chunk;
            let len = std::cmp::min(self.chunk, self.size - off) as usize;
            let buf = self.vf.read_at(off, len).ok()?;
            if self.cache.len() >= CACHE_CHUNKS {
                self.cache.remove(0);
            }
            self.cache.push((ci, buf));
        }
        self.cache.last().map(|(_, b)| b.as_slice())
    }

    fn read(&mut self, offset: u64, out: &mut [u8]) -> Option<usize> {
        let mut done = 0usize;
        while done < out.len() {
            let pos = offset + done as u64;
            if pos >= self.size {
                break;
            }
            let ci = pos / self.chunk;
            let from = (pos - ci * self.chunk) as usize;
            let buf = self.chunk_bytes(ci)?;
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
        cache: Vec::new(),
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

pub fn open_local(path: &std::path::Path) -> Result<VerifiedDb, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err("index is not a regular file".into());
    }
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
        | rusqlite::OpenFlags::SQLITE_OPEN_URI
        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let p = path.to_string_lossy().replace('?', "%3f").replace('#', "%23");
    let conn = rusqlite::Connection::open_with_flags(format!("file:{p}?immutable=1"), flags)
        .map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA query_only=1;")
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

pub fn pdf_text(bytes: &[u8]) -> Option<(Option<String>, String)> {
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

pub fn query(conn: &rusqlite::Connection, q: &str) -> Result<Vec<Hit>, String> {
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

fn run_query(conn: &rusqlite::Connection, fts: &str, q: &str, all: bool) -> Result<Vec<Hit>, String> {
    let t: Vec<String> = terms(q).into_iter().take(8).collect();
    let title_hits = if t.is_empty() {
        "0".to_string()
    } else {
        (0..t.len())
            .map(|i| format!("(instr(lower(title), ?{}) > 0)", i + 3))
            .collect::<Vec<_>>()
            .join(" + ")
    };
    let singular_at = t.len() + 3;
    let sql = format!(
        "SELECT title, url FROM articles
         WHERE articles MATCH ?1
         ORDER BY (title = ?2 COLLATE NOCASE OR title = ?{singular_at} COLLATE NOCASE) DESC,
                  ({title_hits}) DESC, bm25(articles, 10.0, 1.0)
         LIMIT 40"
    );
    let mut stmt = conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
    let exact = exact_title(q);
    let singular = singular(&exact);
    let mut params: Vec<String> = vec![fts.to_string(), exact];
    params.extend(t);
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
