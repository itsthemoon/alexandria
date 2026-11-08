use alexandria_search::streets::Streets;
use alexandria_core::{parse_verifying_key, Entry, Manifest, VerifiedFile};
use alexandria_pmtiles::{Pmtiles, TileType};
use alexandria_roads::Roads;
use alexandria_zim::{article_html, article_html_with, html_to_text, search, Zim};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

mod llm;
mod prompts;

static LIBRARY: Mutex<Option<PathBuf>> = Mutex::new(None);
struct SearchCache {
    key: String,
    db: Arc<Mutex<alexandria_search::VerifiedDb>>,
}
static SEARCH_CACHE: Mutex<Option<Vec<SearchCache>>> = Mutex::new(None);
const MAX_OPEN_INDEXES: usize = 16;

const IMAGE_BUDGET: usize = 24 << 20;
const IMAGE_MAX: usize = 4 << 20;
const IMAGE_MIN_LEFT: usize = 1024;
const IMAGE_MAX_LOOKUPS: usize = 64;
const IMAGE_ATTEMPT_COST: usize = 4 << 20;

struct ImageBudget {
    left: usize,
    lookups: usize,
}

impl ImageBudget {
    fn new() -> ImageBudget {
        ImageBudget { left: IMAGE_BUDGET, lookups: 0 }
    }

    fn cap(&mut self) -> Option<usize> {
        if self.left < IMAGE_MIN_LEFT || self.lookups >= IMAGE_MAX_LOOKUPS {
            return None;
        }
        self.lookups += 1;
        Some(self.left.min(IMAGE_MAX) * 3 / 4)
    }

    fn charge(&mut self, uri: Option<String>) -> Option<String> {
        match &uri {
            Some(u) => self.left = self.left.saturating_sub(u.len()),
            None => self.left = self.left.saturating_sub(IMAGE_ATTEMPT_COST),
        }
        uri
    }
}

#[derive(Serialize)]
struct FileInfo {
    path: String,
    size: u64,
}

#[derive(Serialize)]
struct Article {
    title: String,
    url: String,
    text: String,
    html: String,
}

fn env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is not set"))
}

fn trust_key() -> Result<String, String> {
    if let Some(k) = option_env!("ALEXANDRIA_PUBKEY") {
        return Ok(k.to_string());
    }
    if cfg!(debug_assertions) {
        return env("ALEX_PUBKEY");
    }
    Err("this build has no pinned release key".into())
}

fn libraries_in(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
            if name.contains("alexandria") && p.join("MANIFEST").is_file() {
                out.push(p);
            }
        }
    }
}

fn find_libraries(
    home: Option<&std::path::Path>,
    mounts: &[PathBuf],
    drives: &[PathBuf],
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(h) = home {
        libraries_in(h, &mut out);
    }
    let mut roots: Vec<PathBuf> = drives.to_vec();
    for m in mounts {
        libraries_in(m, &mut out);
        if let Ok(entries) = fs::read_dir(m) {
            roots.extend(entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
        }
    }
    for r in &roots {
        libraries_in(r, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

fn pointer_file() -> PathBuf {
    alexandria_core::config_dir().join("library-path")
}

fn library_from_pointer(note: &std::path::Path) -> Option<PathBuf> {
    let raw = fs::read_to_string(note).ok()?;
    let p = PathBuf::from(raw.trim_start_matches('\u{feff}').trim());
    p.join("MANIFEST").is_file().then_some(p)
}

fn known_libraries() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(p) = std::env::var("ALEX_LIBRARY") {
        out.push(PathBuf::from(p));
    }
    if let Some(p) = library_from_pointer(&pointer_file()) {
        out.push(p);
    }
    out
}

fn first_library(known: Vec<PathBuf>, scan: impl FnOnce() -> Vec<PathBuf>) -> Option<PathBuf> {
    known.into_iter().next().or_else(|| scan().into_iter().next())
}

fn scanned_libraries() -> Vec<PathBuf> {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok().map(PathBuf::from);
    let mut mounts = vec![PathBuf::from("/Volumes"), PathBuf::from("/mnt")];
    if let Ok(user) = std::env::var("USER") {
        mounts.push(PathBuf::from(format!("/media/{user}")));
        mounts.push(PathBuf::from(format!("/run/media/{user}")));
    }
    let drives: Vec<PathBuf> = if cfg!(windows) {
        (b'C'..=b'Z').map(|d| PathBuf::from(format!("{}:\\", d as char))).filter(|p| p.is_dir()).collect()
    } else {
        Vec::new()
    };
    find_libraries(home.as_deref(), &mounts, &drives)
}

fn scanned_libraries_bounded() -> Vec<PathBuf> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(scanned_libraries());
    });
    rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap_or_default()
}

fn library_candidates() -> Vec<PathBuf> {
    let mut out = known_libraries();
    out.extend(scanned_libraries_bounded());
    out.dedup();
    out
}

fn library_dir() -> Result<PathBuf, String> {
    if let Ok(guard) = LIBRARY.lock() {
        if let Some(p) = guard.as_ref() {
            return Ok(p.clone());
        }
    }
    first_library(known_libraries(), scanned_libraries_bounded).ok_or_else(|| {
        "No library found. Get one with the install command at itsthemoon.github.io/alexandria"
            .to_string()
    })
}

fn select_library(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    if !p.join("MANIFEST").is_file() || !p.join("MANIFEST.sig").is_file() {
        return Err(format!("{path} is not an Alexandria library"));
    }
    let selected = p.canonicalize().unwrap_or(p);
    let clean = alexandria_core::strip_verbatim(&selected.to_string_lossy()).into_owned();
    let selected = PathBuf::from(clean);
    let previous = LIBRARY.lock().ok().and_then(|g| g.clone());
    if let Ok(mut guard) = LIBRARY.lock() {
        *guard = Some(selected.clone());
    }
    if let Err(e) = load_manifest() {
        if let Ok(mut guard) = LIBRARY.lock() {
            *guard = previous;
        }
        return Err(e);
    }
    Ok(selected)
}

fn load_manifest() -> Result<(PathBuf, Manifest), String> {
    let dir = library_dir()?;
    let root = parse_verifying_key(&trust_key()?).map_err(|e| e.to_string())?;
    let signed = alexandria_core::open_signed(&dir, &root).map_err(|e| e.to_string())?;
    let edition = edition_of(&signed.manifest.release);
    newest_release(
        &alexandria_core::config_dir().join(release_record(edition)),
        &signed.manifest.date,
    )?;
    Ok((dir, signed.manifest))
}

fn edition_of(release: &str) -> &str {
    if release.to_ascii_lowercase().ends_with("-full") {
        "full"
    } else {
        "lite"
    }
}

fn release_record(edition: &str) -> String {
    if edition == "full" {
        "newest-release-full".to_string()
    } else {
        "newest-release".to_string()
    }
}

fn newest_release(note: &Path, date: &str) -> Result<(), String> {
    let seen = fs::read_to_string(note).unwrap_or_default();
    let seen = seen.trim();
    if alexandria_core::is_date(seen) && date < seen {
        return Err(format!(
            "This library ({date}) is older than one this computer has already opened ({seen}). Run the install command to update it."
        ));
    }
    if date != seen {
        if let Some(dir) = note.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(note, date);
    }
    Ok(())
}

fn library_info_impl(path: Option<String>) -> Result<serde_json::Value, String> {
    let selected = match path {
        Some(p) => select_library(&p)?,
        None => library_dir()?,
    };
    let (_, m) = load_manifest()?;
    let libraries: Vec<serde_json::Value> = library_candidates()
        .iter()
        .map(|p| serde_json::json!({ "path": p.display().to_string(), "label": p.file_name().and_then(|s| s.to_str()).unwrap_or("Library") }))
        .collect();
    let files: Vec<FileInfo> = m
        .entries
        .iter()
        .map(|e| FileInfo {
            path: e.path.clone(),
            size: e.size,
        })
        .collect();
    let packs: Vec<serde_json::Value> = ordered_packs(&m)
        .iter()
        .map(|e| serde_json::json!({ "path": e.path, "label": pack_label(&e.path) }))
        .collect();
    let maps: Vec<serde_json::Value> = m
        .entries
        .iter()
        .filter(|e| e.path.ends_with(".pmtiles"))
        .map(|e| {
            let (label, street) = map_label(&e.path);
            serde_json::json!({ "path": e.path, "label": label, "street": street })
        })
        .collect();
    let roads = m.entries.iter().any(|e| e.path == "roads-us.roads");
    let model = llm::pick_model(&m).map(|e| llm::model_label(&e.path));
    Ok(serde_json::json!({
        "model": model,
        "release": m.release,
        "date": m.date,
        "files": files,
        "packs": packs,
        "maps": maps,
        "roads": roads,
        "path": selected.display().to_string(),
        "libraries": libraries
    }))
}

fn map_label(path: &str) -> (String, bool) {
    let stem = path.trim_end_matches(".pmtiles");
    let dated = stem.len() == 8 && stem.bytes().all(|b| b.is_ascii_digit());
    match stem {
        "world" => ("World — overview".into(), false),
        "planet" => ("World — street level".into(), true),
        _ if dated => ("World — street level".into(), true),
        _ => (stem.into(), false),
    }
}

const KNOWN_PACKS: &[(&str, &str)] = &[
    ("wikipedia_en_all", "Wikipedia"),
    ("wikipedia_en-simple", "Simple English Wikipedia"),
    ("wikipedia_en_medicine", "WikiMed (medical)"),
    ("ifixit", "iFixit repair guides"),
    ("appropedia", "Appropedia"),
    ("zimgit-post-disaster", "Post-disaster guides"),
    ("zimgit-water", "Water"),
    ("zimgit-food-preparation", "Food preparation"),
    ("zimgit-medicine", "Field medicine"),
    ("trueprepper", "TruePrepper"),
    ("wikipedia_ln", "Wikipedia (Lingala)"),
    ("wikipedia_en_100", "Wikipedia sample"),
];

fn ordered_packs(m: &Manifest) -> Vec<Entry> {
    let rank = |p: &str| {
        let p = p.to_ascii_lowercase();
        KNOWN_PACKS.iter().position(|(k, _)| p.starts_with(k)).unwrap_or(KNOWN_PACKS.len())
    };
    let mut zims: Vec<Entry> = m.entries.iter().filter(|e| e.path.ends_with(".zim")).cloned().collect();
    zims.sort_by_key(|e| rank(&e.path));
    zims
}

fn default_pack(m: &Manifest) -> Option<Entry> {
    ordered_packs(m).into_iter().next()
}

fn pack_label(path: &str) -> String {
    let p = path.to_ascii_lowercase();
    for (k, v) in KNOWN_PACKS {
        if p.starts_with(k) {
            return v.to_string();
        }
    }
    path.trim_end_matches(".zim").replace(['_', '-'], " ")
}

fn zim_entry_for(m: &Manifest, pack: Option<&str>) -> Result<Entry, String> {
    let zims: Vec<&Entry> = m
        .entries
        .iter()
        .filter(|e| e.path.ends_with(".zim"))
        .collect();
    match pack {
        Some(p) => zims
            .into_iter()
            .find(|e| e.path == p || e.path.ends_with(p))
            .cloned()
            .ok_or_else(|| "that pack is not in this library".to_string()),
        None => default_pack(m).ok_or_else(|| "this library has no .zim pack".to_string()),
    }
}

fn model_path(dir: &std::path::Path, m: &Manifest) -> Result<PathBuf, String> {
    let entry = llm::pick_model(m).ok_or("this library has no model")?;
    alexandria_core::cached_copy(dir, &entry, m.chunk, &alexandria_core::cache_dir()).map_err(|e| e.to_string())
}

#[derive(Serialize, Clone)]
struct Source {
    title: String,
    url: String,
    pack: String,
}

#[derive(Serialize)]
struct Answer {
    text: String,
    sources: Vec<Source>,
}

fn ask_impl(
    question: String,
    pack: Option<String>,
    on_text: &mut dyn FnMut(&str),
) -> Result<Answer, String> {
    let gen = llm::next_generation();
    let (dir, m) = load_manifest()?;
    let mut all = Vec::new();
    for z in m.entries.iter().filter(|e| e.path.ends_with(".zim")) {
        let found = with_db(&dir, &m, z, |db| {
            let vf = VerifiedFile::open_in(&dir, z, m.chunk).ok()?;
            let zim = Zim::open(vf).ok()?;
            let zim = std::sync::Mutex::new(zim);
            let ocr = ocr_for(&dir, &m, &z.path);
            alexandria_search::ask_candidates(db.conn(), &z.path, &question, &|url| {
                let (raw, mime, e_url) = {
                    let mut zim = zim.lock().unwrap();
                    let idx = zim.find_index('C', url).or_else(|_| zim.find_index('A', url)).ok()?;
                    let e = zim.resolve(idx).ok()?;
                    let mime = zim.mime(e.mime).map(str::to_string);
                    let raw = zim.read_entry_blob(&e).ok()?;
                    (raw, mime, e.url)
                };
                if mime.as_deref() == Some("application/pdf") {
                    match alexandria_search::pdf_text_only(&raw).filter(|t| t.split_whitespace().count() >= 20) {
                        Some(text) => Some(text),
                        None => ocr.as_ref().and_then(|o| o.text(&e_url).map(str::to_string)),
                    }
                } else {
                    Some(html_to_text(&String::from_utf8_lossy(&raw)))
                }
            })
            .ok()
        });
        all.extend(found.unwrap_or_default().into_iter().map(|p| (z.path.clone(), p)));
    }
    let picked: Vec<(String, String)> = if all.is_empty() {
        let zim = zim_entry_for(&m, pack.as_deref())?;
        let vf = VerifiedFile::open_in(&dir, &zim, m.chunk).map_err(|e| e.to_string())?;
        let mut z = Zim::open(vf).map_err(|e| e.to_string())?;
        search(&mut z, &question, 3)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|h| (zim.path.clone(), h.url))
            .collect()
    } else {
        alexandria_search::pick_sources(all, 3, &question).into_iter().map(|(pack, p)| (pack, p.url)).collect()
    };
    let mut context = String::new();
    let mut sources = Vec::new();
    for (pack, url) in picked {
        if let Ok(a) = read_article_impl(url, Some(pack.clone())) {
            let passage = alexandria_search::best_passage(&llm::compact(&a.text), &question, 1500);
            context.push_str(&format!("[{}] {}\n{}\n\n", sources.len() + 1, a.title, passage));
            sources.push(Source { title: a.title, url: a.url, pack });
        }
    }
    if sources.is_empty() {
        return Err("Nothing in the library matches that question. Try other words.".into());
    }
    let (system, user) = llm::ask_prompt(&context, question.trim());
    let model = model_path(&dir, &m)?;
    let tpl = llm::template_file(&alexandria_core::cache_dir()).map_err(|e| e.to_string())?;
    let text = llm::run(base_command(llama_bin()?), &model, &tpl, &system, &user, 600, "0.1", gen, on_text)?;
    Ok(Answer { text, sources })
}

fn text_page(title: Option<&str>, text: &str) -> String {
    let h1 = title.map(|t| format!("<h1>{}</h1>", esc(t))).unwrap_or_default();
    article_html(&format!(
        "<body>{h1}<pre style=\"white-space:pre-wrap;font:inherit;background:none;padding:0\">{}</pre></body>",
        esc(text)
    ))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn ocr_for(dir: &Path, m: &Manifest, pack: &str) -> Option<alexandria_search::ocr::Ocr> {
    let path = format!("{pack}.ocr");
    let entry = m.entries.iter().find(|e| e.path == path)?;
    let mut vf = VerifiedFile::open_in(dir, entry, m.chunk).ok()?;
    let size = vf.size();
    if size > 64 * 1024 * 1024 {
        return None;
    }
    let bytes = vf.read_at(0, size as usize).ok()?;
    Some(alexandria_search::ocr::Ocr::parse(&bytes)).filter(|o| !o.is_empty())
}

fn read_article_impl(url: String, pack: Option<String>) -> Result<Article, String> {
    let (dir, m) = load_manifest()?;
    let entry = zim_entry_for(&m, pack.as_deref())?;
    let vf = VerifiedFile::open_in(&dir, &entry, m.chunk).map_err(|e| e.to_string())?;
    let mut z = Zim::open(vf).map_err(|e| e.to_string())?;
    let idx = z
        .find_index('C', &url)
        .or_else(|_| z.find_index('A', &url))
        .map_err(|e| e.to_string())?;
    let hit = z.resolve(idx).map_err(|e| e.to_string())?;
    let mime = z.mime(hit.mime).unwrap_or("").to_string();
    if mime == "application/pdf" {
        let bytes = z.read_blob(hit.cluster, hit.blob).map_err(|e| e.to_string())?;
        let (meta, text) = match alexandria_search::pdf_text(&bytes)
            .filter(|(_, t)| t.split_whitespace().count() >= 20)
        {
            Some(found) => found,
            None => {
                let text = ocr_for(&dir, &m, &entry.path)
                    .and_then(|o| o.text(&hit.url).map(str::to_string))
                    .ok_or("This PDF has no readable text (it may be a scan).")?;
                (None, text)
            }
        };
        let title = alexandria_search::pdf_title(meta.as_deref(), &text, &hit.title);
        let html = text_page(Some(&title), &text);
        return Ok(Article { title, url: hit.url, text, html });
    }
    if mime != "text/html" && mime != "text/plain" {
        return Err(format!("not a text article ({mime})"));
    }
    let body = z
        .read_blob(hit.cluster, hit.blob)
        .map_err(|e| e.to_string())?;
    let raw = String::from_utf8_lossy(&body);
    let text = html_to_text(&raw);
    let html = if mime == "text/html" {
        let mut budget = ImageBudget::new();
        article_html_with(&raw, &mut |src| {
            let max = budget.cap()?;
            budget.charge(z.image_data_uri(src, max))
        })
    } else {
        text_page(None, &text)
    };
    Ok(Article {
        title: hit.title,
        url: hit.url,
        text,
        html,
    })
}

#[tauri::command]
async fn library_info(
    state: tauri::State<'_, MapState>,
    path: Option<String>,
) -> Result<serde_json::Value, String> {
    let info = tauri::async_runtime::spawn_blocking(move || library_info_impl(path))
        .await
        .map_err(|e| e.to_string())??;
    std::thread::spawn(|| {
        if let Ok((dir, m)) = load_manifest() {
            let _ = model_path(&dir, &m);
        }
    });
    if let Ok(mut guard) = state.0.lock() {
        *guard = None;
    }
    if let Ok(mut guard) = SEARCH_CACHE.lock() {
        *guard = None;
    }
    if let Ok(mut guard) = STREETS.lock() {
        *guard = None;
    }
    if let Ok(mut guard) = ROADS.lock() {
        *guard = None;
    }
    Ok(info)
}

#[tauri::command]
async fn ask(
    question: String,
    pack: Option<String>,
    on_text: tauri::ipc::Channel<String>,
) -> Result<Answer, String> {
    tauri::async_runtime::spawn_blocking(move || {
        ask_impl(question, pack, &mut |t| {
            let _ = on_text.send(t.to_string());
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
struct BrowseItem {
    title: String,
    url: String,
}

#[derive(Serialize)]
struct BrowsePage {
    total: usize,
    partial: bool,
    more: bool,
    items: Vec<BrowseItem>,
}

#[tauri::command]
async fn browse(
    pack: Option<String>,
    sort: Option<String>,
    q: Option<String>,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Result<BrowsePage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (dir, m) = load_manifest()?;
        let entry = zim_entry_for(&m, pack.as_deref())?;
        let vf = VerifiedFile::open_in(&dir, &entry, m.chunk).map_err(|e| e.to_string())?;
        let mut z = Zim::open(vf).map_err(|e| e.to_string())?;
        let needle = q.unwrap_or_default();
        let reverse = sort.as_deref() == Some("za");
        let start = offset.unwrap_or(0) as usize;
        let take = limit.unwrap_or(100).min(500) as usize;
        let (total, more, partial, entries) = if needle.is_empty() {
            let (total, entries) = z.browse(&needle, reverse, start, take).map_err(|e| e.to_string())?;
            (total, false, false, entries)
        } else if let Some((total, more, hits)) = with_db(&dir, &m, &entry, |db| {
            alexandria_search::browse(db.conn(), &needle, reverse, start, take).ok()
        }) {
            let entries = verified_browse_entries(&mut z, hits);
            (total, more, false, entries)
        } else {
            let (total, entries) = z.browse(&needle, reverse, start, take).map_err(|e| e.to_string())?;
            let partial = z.header.article_count as usize > alexandria_zim::FILTER_SCAN_CAP;
            (total, false, partial, entries)
        };
        let items = entries
            .into_iter()
            .map(|e| BrowseItem {
                title: e.title,
                url: e.url,
            })
            .collect();
        Ok(BrowsePage { total, partial, more, items })
    })
    .await
    .map_err(|e| e.to_string())?
}

fn verified_browse_entries(z: &mut Zim<VerifiedFile>, hits: Vec<alexandria_search::Hit>) -> Vec<alexandria_zim::Entry> {
    let mut out: Vec<alexandria_zim::Entry> = Vec::new();
    for h in hits {
        let Some(e) = z.lookup_url(&h.url) else {
            continue;
        };
        if out.iter().any(|f| f.url == e.url) {
            continue;
        }
        out.push(e);
    }
    out
}

const LANGUAGES: &[&str] = &[
    "English", "Spanish", "French", "Portuguese", "German", "Italian", "Dutch", "Russian",
    "Ukrainian", "Polish", "Turkish", "Arabic", "Hebrew", "Persian", "Hindi", "Bengali", "Urdu",
    "Chinese (Simplified)", "Chinese (Traditional)", "Japanese", "Korean", "Vietnamese", "Thai",
    "Indonesian", "Swahili", "Tagalog", "Greek", "Czech", "Romanian", "Hungarian", "Swedish",
];

#[tauri::command]
fn languages() -> Vec<&'static str> {
    LANGUAGES.to_vec()
}

#[tauri::command]
async fn translate(
    text: String,
    target: String,
    on_text: tauri::ipc::Channel<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let gen = llm::next_generation();
        let target = LANGUAGES
            .iter()
            .find(|l| **l == target)
            .ok_or("that language is not offered")?;
        let input: String = text.trim().chars().take(6000).collect();
        if input.is_empty() {
            return Ok(String::new());
        }
        let (dir, m) = load_manifest()?;
        let model = model_path(&dir, &m)?;
        let (system, user) = llm::translate_prompt(target, &input);
        let tpl = llm::template_file(&alexandria_core::cache_dir()).map_err(|e| e.to_string())?;
        llm::run(base_command(llama_bin()?), &model, &tpl, &system, &user, 1800, "0", gen, &mut |t| {
            let _ = on_text.send(t.to_string());
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
struct SearchHit {
    title: String,
    url: String,
    snippet: Option<String>,
    all: bool,
}

fn search_cache_key(dir: &std::path::Path, ie: &Entry) -> String {
    let sig = fs::read(dir.join("MANIFEST.sig")).unwrap_or_default();
    format!("{}::{}::{}", dir.display(), ie.path, alexandria_core::hex(&sig))
}

fn search_with_index(
    dir: &std::path::Path,
    m: &Manifest,
    entry: &Entry,
    q: &str,
) -> Option<Vec<alexandria_search::Hit>> {
    with_index(dir, m, entry, |db| alexandria_search::query(db.conn(), q))
}

fn with_db<T>(
    dir: &std::path::Path,
    m: &Manifest,
    entry: &Entry,
    f: impl FnOnce(&alexandria_search::VerifiedDb) -> Option<T>,
) -> Option<T> {
    let index_path = format!("{}.search.db", entry.path);
    let signed = m.entries.iter().find(|e| e.path == index_path).cloned();
    let local = alexandria_search::local_index_path(dir, entry);
    let key = match &signed {
        Some(ie) => search_cache_key(dir, ie),
        None => format!("local::{}", local.display()),
    };
    let mut guard = SEARCH_CACHE.lock().ok()?;
    let cache = guard.get_or_insert_with(Vec::new);
    if !cache.iter().any(|c| c.key == key) {
        let db = match &signed {
            Some(ie) => {
                let vf = VerifiedFile::open_in(dir, ie, m.chunk).ok()?;
                alexandria_search::open_verified(vf, m.chunk).ok()?
            }
            None => alexandria_search::open_local(&local).ok()?,
        };
        if cache.len() >= MAX_OPEN_INDEXES {
            cache.remove(0);
        }
        cache.push(SearchCache { key: key.clone(), db: Arc::new(Mutex::new(db)) });
    }
    let db = cache.iter().find(|c| c.key == key)?.db.clone();
    drop(guard);
    let conn = db.lock().ok()?;
    f(&conn)
}

fn with_index(
    dir: &std::path::Path,
    m: &Manifest,
    entry: &Entry,
    run: impl Fn(&alexandria_search::VerifiedDb) -> Result<Vec<alexandria_search::Hit>, String>,
) -> Option<Vec<alexandria_search::Hit>> {
    with_db(dir, m, entry, |db| run(db).ok())
}

static PDF_TITLES: Mutex<Option<std::collections::HashMap<String, String>>> = Mutex::new(None);

static STREETS: Mutex<Option<(std::path::PathBuf, Streets)>> = Mutex::new(None);

static ROADS: Mutex<Option<(std::path::PathBuf, Roads)>> = Mutex::new(None);

#[derive(serde::Serialize)]
#[allow(non_snake_case)]
struct StreetHit {
    name: String,
    lat: f32,
    lon: f32,
    place: Option<String>,
}

#[tauri::command]
async fn streets_search(
    q: String,
    lat: Option<f32>,
    lon: Option<f32>,
    limit: Option<usize>,
) -> Result<Vec<StreetHit>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if q.trim().chars().count() < 2 {
            return Ok(Vec::new());
        }
        let (dir, m) = load_manifest()?;
        let file = dir.join("streets.txt");
        let entry = m.entries.iter().find(|e| e.path == "streets.txt").cloned();
        let mut cache = STREETS.lock().map_err(|e| e.to_string())?;
        if cache.as_ref().map(|(p, _)| p != &file).unwrap_or(true) {
            let opened = entry
                .as_ref()
                .and_then(|e| VerifiedFile::open_in(&dir, e, m.chunk).ok())
                .and_then(|vf| Streets::open_verified(vf, m.chunk));
            *cache = opened.map(|s| (file.clone(), s));
        }
        let Some((_, streets)) = cache.as_ref() else {
            return Ok(Vec::new());
        };
        let near = match (lat, lon) {
            (Some(a), Some(b)) => Some((a, b)),
            _ => None,
        };
        let rows = streets.find(&q, near, limit.unwrap_or(8).clamp(1, 20));
        Ok(rows
            .into_iter()
            .map(|(name, lat, lon, place)| StreetHit { name, lat, lon, place })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn roads_entry(m: &Manifest) -> Result<Entry, String> {
    m.entries.iter().find(|e| e.path == "roads-us.roads").cloned().ok_or_else(|| "this library has no roads pack".to_string())
}

#[tauri::command]
async fn route(from: [f64; 2], to: [f64; 2]) -> Result<alexandria_roads::Route, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (dir, m) = load_manifest()?;
        let entry = roads_entry(&m)?;
        let file = dir.join("roads-us.roads");
        let mut cache = ROADS.lock().map_err(|e| e.to_string())?;
        if cache.as_ref().map(|(p, _)| p != &file).unwrap_or(true) {
            let roads = VerifiedFile::open_in(&dir, &entry, m.chunk).map_err(|e| e.to_string()).and_then(Roads::open)?;
            *cache = Some((file.clone(), roads));
        }
        let Some((_, roads)) = cache.as_mut() else {
            return Err("the roads pack could not be opened".to_string());
        };
        roads.route((from[0], from[1]), (to[0], to[1]))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn checked_hits(z: &mut Zim<VerifiedFile>, pack: &Entry, hits: Vec<alexandria_search::Hit>, q: &str) -> Vec<SearchHit> {
    let mut found: Vec<(alexandria_zim::Entry, bool, bool)> = Vec::new();
    for h in hits {
        let Ok(idx) = z.find_index('C', &h.url).or_else(|_| z.find_index('A', &h.url)) else {
            continue;
        };
        let Ok(e) = z.resolve(idx) else {
            continue;
        };
        if found.iter().any(|(f, _, _)| f.url == e.url) {
            continue;
        }
        let pdf = z.mime(e.mime) == Some("application/pdf");
        found.push((e, pdf, h.all));
    }
    let tag = alexandria_core::hex(&pack.hashes_sha256);
    let key = |url: &str| format!("{tag}::{url}");
    let known: std::collections::HashMap<String, String> = PDF_TITLES.lock().ok().and_then(|g| g.clone()).unwrap_or_default();
    let missing: Vec<&alexandria_zim::Entry> = found.iter().filter(|(e, pdf, _)| *pdf && !known.contains_key(&key(&e.url))).map(|(e, _, _)| e).collect();
    let items: Vec<(String, Vec<u8>)> = missing.iter().filter_map(|e| Some((e.url.clone(), z.read_blob(e.cluster, e.blob).ok()?))).collect();
    let urls: Vec<String> = items.iter().map(|(u, _)| u.clone()).collect();
    let titles = alexandria_search::pdf_titles(items);
    let mut known = known;
    if let Ok(mut g) = PDF_TITLES.lock() {
        let cache = g.get_or_insert_with(std::collections::HashMap::new);
        for (u, t) in urls.iter().zip(titles) {
            cache.insert(key(u), t.clone());
            known.insert(key(u), t);
        }
    }
    found
        .into_iter()
        .map(|(e, pdf, all)| {
            if pdf {
                let title = known.get(&key(&e.url)).cloned().unwrap_or_else(|| alexandria_search::pdf_name(&e.url));
                return SearchHit { title, url: e.url, snippet: None, all };
            }
            let snippet = z
                .read_blob(e.cluster, e.blob)
                .ok()
                .and_then(|b| alexandria_search::snippet(&html_to_text(&String::from_utf8_lossy(&b)), q));
            let title = if e.title.is_empty() { e.url.clone() } else { e.title };
            SearchHit { title, url: e.url, snippet, all }
        })
        .collect()
}

#[tauri::command]
async fn search_articles(q: String, pack: Option<String>, limit: Option<usize>) -> Result<Vec<SearchHit>, String> {
    let limit = limit.unwrap_or(40).clamp(1, 40);
    tauri::async_runtime::spawn_blocking(move || {
        let (dir, m) = load_manifest()?;
        let entry = zim_entry_for(&m, pack.as_deref())?;
        let vf = VerifiedFile::open_in(&dir, &entry, m.chunk).map_err(|e| e.to_string())?;
        let mut z = Zim::open(vf).map_err(|e| e.to_string())?;
        let exact = alexandria_search::title_urls(&q).into_iter().find(|u| z.lookup_url(u).is_some());
        if let Some(mut hits) = search_with_index(&dir, &m, &entry, &q) {
            hits.truncate(limit);
            if let Some(url) = exact.clone() {
                hits.retain(|h| h.url != url);
                hits.insert(0, alexandria_search::Hit { title: url.clone(), url, all: true });
                hits.truncate(limit);
            }
            let hits = checked_hits(&mut z, &entry, hits, &q);
            if !hits.is_empty() {
                return Ok(hits);
            }
        }
        let mut out = match exact {
            Some(url) => checked_hits(&mut z, &entry, vec![alexandria_search::Hit { title: url.clone(), url, all: true }], &q),
            None => Vec::new(),
        };
        for h in search(&mut z, &q, limit).map_err(|e| e.to_string())? {
            if out.len() < limit && !out.iter().any(|x| x.url == h.url) {
                out.push(SearchHit { title: h.title, url: h.url, snippet: None, all: true });
            }
        }
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn read_article(url: String, pack: Option<String>) -> Result<Article, String> {
    tauri::async_runtime::spawn_blocking(move || read_article_impl(url, pack))
        .await
        .map_err(|e| e.to_string())?
}

fn base_command(bin: std::path::PathBuf) -> std::process::Command {
    let mut c = std::process::Command::new(bin);
    c.env_clear();
    #[cfg(windows)]
    for key in ["SystemRoot", "TEMP", "TMP"] {
        if let Ok(v) = std::env::var(key) {
            c.env(key, v);
        }
    }
    c
}

fn llama_bin() -> Result<std::path::PathBuf, String> {
    if cfg!(debug_assertions) {
        if let Ok(p) = std::env::var("ALEX_LLAMA") {
            return Ok(std::path::PathBuf::from(p));
        }
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("no executable directory")?;
    for name in ["llama-completion", "llama-completion.exe"] {
        let c = dir.join(name);
        if c.is_file() {
            return Ok(c);
        }
    }
    Err("llama-completion is not installed next to the application".into())
}

#[derive(Serialize)]
struct PersonalEntry {
    name: String,
    kind: String,
    size: u64,
}

fn personal_root() -> Result<PathBuf, String> {
    let lib = library_dir()?;
    let parent = lib.parent().ok_or("library path has no parent")?;
    let root = parent.join("Alexandria-Personal");
    if let Ok(meta) = fs::symlink_metadata(&root) {
        if meta.file_type().is_symlink() {
            return Err("the personal folder cannot be a symlink".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("the personal folder cannot be a reparse point".into());
            }
        }
        if !meta.is_dir() {
            return Err("the personal folder is not a directory".into());
        }
    }
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let parent = parent.canonicalize().map_err(|e| e.to_string())?;
    if root.parent() != Some(parent.as_path()) {
        return Err("the personal folder is outside the library's parent".into());
    }
    Ok(root)
}

fn personal_name(name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains(':')
    {
        return Err("that file name is not allowed".into());
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
    const RESERVED: &[&str] = &[
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    if RESERVED.contains(&stem.as_str()) {
        return Err("that file name is reserved".into());
    }
    Ok(PathBuf::from(name))
}

fn personal_path(name: &str) -> Result<PathBuf, String> {
    let root = personal_root()?;
    let path = root.join(personal_name(name)?);
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let parent = path
        .parent()
        .ok_or("bad file path")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if parent != root {
        return Err("file is outside the personal folder".into());
    }
    Ok(path)
}

fn kind_for(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "heic" | "tif" | "tiff" => "image",
        "pdf" => "pdf",
        "txt" | "md" | "csv" | "json" | "log" => "text",
        _ => "file",
    }
}

fn passive_extension(name: &str) -> Option<&'static str> {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "heic" | "tif" | "tiff" => Some("image"),
        "pdf" => Some("pdf"),
        "txt" | "md" | "csv" | "json" | "log" => Some("text"),
        _ => None,
    }
}

#[tauri::command]
async fn personal_list() -> Result<Vec<PersonalEntry>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let root = personal_root()?;
        let mut out = Vec::new();
        for entry in fs::read_dir(&root).map_err(|e| e.to_string())?.flatten() {
            let meta = match fs::symlink_metadata(entry.path()) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if !meta.is_file() || meta.file_type().is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            out.push(PersonalEntry {
                kind: kind_for(&name).to_string(),
                name,
                size: meta.len(),
            });
        }
        out.sort_by(|a, b| b.name.to_lowercase().cmp(&a.name.to_lowercase()));
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn personal_save(name: String, bytes: Vec<u8>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(
                "files over 64 MB should be copied into the personal folder with Finder".into(),
            );
        }
        let path = personal_path(&name)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        use std::io::Write;
        file.write_all(&bytes).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn clean_image(bytes: &[u8], max: u32) -> Result<Vec<u8>, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|_| "this photo can't be shown".to_string())?;
    let img = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
    let mut out = std::io::Cursor::new(Vec::new());
    img.to_rgba8()
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

#[tauri::command]
async fn personal_image(name: String, max: u32) -> Result<Vec<u8>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = personal_path(&name)?;
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 128 * 1024 * 1024 {
            return Err("file is too large to preview".into());
        }
        clean_image(&fs::read(&path).map_err(|e| e.to_string())?, max.clamp(64, 4096))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn personal_reveal(name: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = personal_path(&name)?;
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err("file not found or is a symlink".into());
        }
        #[cfg(target_os = "macos")]
        let mut cmd = {
            let mut c = std::process::Command::new("open");
            c.arg("-R").arg(&path);
            c
        };
        #[cfg(target_os = "windows")]
        let mut cmd = {
            let mut c = std::process::Command::new("explorer.exe");
            c.arg(format!("/select,\"{}\"", path.display()));
            c
        };
        #[cfg(all(unix, not(target_os = "macos")))]
        let mut cmd = {
            let mut c = std::process::Command::new("xdg-open");
            c.arg(path.parent().unwrap_or(&path));
            c
        };
        cmd.status().map(|_| ()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn personal_open(name: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = personal_path(&name)?;
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err("file not found or is a symlink".into());
        }
        if passive_extension(&name).is_none() {
            return Err("this file type is not opened by the app; use Reveal".into());
        }
        #[cfg(target_os = "macos")]
        let status = std::process::Command::new("open").arg(&path).status();
        #[cfg(target_os = "windows")]
        let status = std::process::Command::new("explorer.exe")
            .arg(&path)
            .status();
        #[cfg(all(unix, not(target_os = "macos")))]
        let status = std::process::Command::new("xdg-open").arg(&path).status();
        status.map(|_| ()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

struct MapState(Arc<Mutex<Option<(String, Pmtiles<VerifiedFile>)>>>);

fn default_map() -> Result<String, String> {
    let (_, m) = load_manifest()?;
    if m.entries.iter().any(|e| e.path == "world.pmtiles") {
        return Ok("world.pmtiles".to_string());
    }
    m.entries
        .iter()
        .find(|e| e.path.ends_with(".pmtiles"))
        .map(|e| e.path.clone())
        .ok_or_else(|| "this library has no .pmtiles map".to_string())
}

fn open_map(path: &str) -> Result<Pmtiles<VerifiedFile>, String> {
    let (dir, m) = load_manifest()?;
    let entry = m
        .entries
        .iter()
        .find(|e| e.path == path)
        .cloned()
        .ok_or_else(|| format!("no map pack named {path}"))?;
    let vf = VerifiedFile::open_in(&dir, &entry, m.chunk).map_err(|e| e.to_string())?;
    Pmtiles::open(vf).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct MapInfo {
    min_zoom: u8,
    max_zoom: u8,
    min_lon: f64,
    min_lat: f64,
    max_lon: f64,
    max_lat: f64,
    global: bool,
    kind: String,
}

#[tauri::command]
async fn map_info(
    state: tauri::State<'_, MapState>,
    pack: Option<String>,
) -> Result<MapInfo, String> {
    let state = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let want = match pack {
            Some(p) => p,
            None => default_map()?,
        };
        let mut guard = state.lock().map_err(|_| "map lock".to_string())?;
        let stale = guard.as_ref().map(|(p, _)| p != &want).unwrap_or(true);
        if stale {
            *guard = Some((want.clone(), open_map(&want)?));
        }
        let p = &guard.as_ref().unwrap().1;
        let kind = match p.tile_type() {
            TileType::Mvt => "mvt",
            TileType::Png => "png",
            TileType::Jpeg => "jpeg",
            TileType::Webp => "webp",
            TileType::Other(_) => "unsupported",
        };
        Ok(MapInfo {
            min_zoom: p.header.min_zoom,
            max_zoom: p.header.max_zoom,
            min_lon: p.min_lon,
            min_lat: p.min_lat,
            max_lon: p.max_lon,
            max_lat: p.max_lat,
            global: p.max_lon - p.min_lon > 300.0,
            kind: kind.to_string(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn map_for_point(lon: f64, lat: f64) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (_, m) = load_manifest()?;
        let mut best: Option<(u8, String)> = None;
        for e in m.entries.iter().filter(|e| e.path.ends_with(".pmtiles")) {
            if let Ok(p) = open_map(&e.path) {
                let inside = lon >= p.min_lon && lon <= p.max_lon && lat >= p.min_lat && lat <= p.max_lat;
                if inside && best.as_ref().map(|(z, _)| p.header.max_zoom > *z).unwrap_or(true) {
                    best = Some((p.header.max_zoom, e.path.clone()));
                }
            }
        }
        Ok(best.map(|(_, path)| path))
    })
    .await
    .map_err(|e| e.to_string())?
}

static GLYPHS: &[(&str, &str, &[u8])] = &[
    ("Noto Sans Regular", "0-255.pbf", include_bytes!("../ui/fonts/Noto Sans Regular/0-255.pbf")),
    ("Noto Sans Regular", "1024-1279.pbf", include_bytes!("../ui/fonts/Noto Sans Regular/1024-1279.pbf")),
    ("Noto Sans Regular", "256-511.pbf", include_bytes!("../ui/fonts/Noto Sans Regular/256-511.pbf")),
    ("Noto Sans Regular", "512-767.pbf", include_bytes!("../ui/fonts/Noto Sans Regular/512-767.pbf")),
    ("Noto Sans Regular", "768-1023.pbf", include_bytes!("../ui/fonts/Noto Sans Regular/768-1023.pbf")),
];

#[tauri::command]
fn map_glyphs(fontstack: String, range: String) -> tauri::ipc::Response {
    let bytes = GLYPHS
        .iter()
        .find(|(f, r, _)| *f == fontstack && *r == range)
        .map(|(_, _, b)| *b);
    tauri::ipc::Response::new(bytes.map(|b| b.to_vec()).unwrap_or_default())
}

#[tauri::command]
async fn map_tile(
    state: tauri::State<'_, MapState>,
    z: u8,
    x: u32,
    y: u32,
) -> Result<tauri::ipc::Response, String> {
    let state = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = state.lock().map_err(|_| "map lock".to_string())?;
        if guard.is_none() {
            let want = default_map()?;
            *guard = Some((want.clone(), open_map(&want)?));
        }
        let p = &mut guard.as_mut().unwrap().1;
        let bytes = tile_or_empty(p.tile(z, x, y))?;
        Ok(tauri::ipc::Response::new(bytes))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn tile_or_empty(r: alexandria_pmtiles::Result<Vec<u8>>) -> Result<Vec<u8>, String> {
    match r {
        Ok(b) => Ok(b),
        Err(alexandria_pmtiles::Error::NotFound) => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn article_target(raw: &str) -> Option<(String, String)> {
    let rest = raw.strip_prefix("alexandria-article:")?;
    let (pack, url) = rest.split_once('/')?;
    Some((percent_decode(pack), percent_decode(url)))
}

fn own_page(raw: &str) -> bool {
    raw == "about:srcdoc"
        || raw == "about:blank"
        || ["tauri://localhost/", "http://tauri.localhost/", "https://tauri.localhost/"]
            .iter()
            .any(|p| raw.starts_with(p))
        || ["tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"].contains(&raw)
}

fn article_nav<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("article-nav")
        .on_navigation(|webview, url| {
            if let Some((pack, target)) = article_target(url.as_str()) {
                let js = format!(
                    "openArticle({}, {})",
                    serde_json::to_string(&target).unwrap_or_default(),
                    serde_json::to_string(&pack).unwrap_or_default()
                );
                let _ = webview.eval(js);
                return false;
            }
            own_page(url.as_str())
        })
        .build()
}

fn main() {
    tauri::Builder::default()
        .plugin(article_nav())
        .manage(MapState(Arc::new(Mutex::new(None))))
        .invoke_handler(tauri::generate_handler![
            library_info,
            read_article,
            ask,
            map_info,
            map_tile,
            map_for_point,
            map_glyphs,
            search_articles,
            streets_search,
            route,
            translate,
            languages,
            browse,
            personal_list,
            personal_save,
            personal_image,
            personal_open,
            personal_reveal
        ])
        .build(tauri::generate_context!())
        .expect("alexandria failed to start")
        .run(|_, _| {});
}

#[cfg(test)]
mod page_tests {
    use super::text_page;

    #[test]
    fn pdf_and_text_pages_use_the_dark_reader_style() {
        let h = text_page(Some("Water <guide>"), "Boil & cool");
        assert!(h.contains("#0e0f11"));
        assert!(h.contains("<h1>Water &lt;guide&gt;</h1>"));
        assert!(h.contains("Boil &amp; cool"));
        assert!(h.contains("background:none"));
        assert!(!text_page(None, "plain").contains("<h1>"));
    }
}

#[cfg(test)]
mod map_tests {
    use super::{map_label, tile_or_empty};

    #[test]
    fn a_dated_protomaps_build_is_the_street_level_planet() {
        assert_eq!(map_label("20261104.pmtiles"), ("World — street level".to_string(), true));
        assert_eq!(map_label("planet.pmtiles"), ("World — street level".to_string(), true));
        assert_eq!(map_label("world.pmtiles"), ("World — overview".to_string(), false));
        assert_eq!(map_label("nyc.pmtiles"), ("nyc".to_string(), false));
        assert_eq!(map_label("2026110.pmtiles"), ("2026110".to_string(), false));
    }
    use alexandria_pmtiles::Error;

    #[test]
    fn missing_tile_is_an_empty_tile_not_an_error() {
        assert_eq!(tile_or_empty(Err(Error::NotFound)).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn real_failures_still_fail() {
        assert!(tile_or_empty(Err(Error::TooLarge)).is_err());
        assert_eq!(tile_or_empty(Ok(vec![1, 2])).unwrap(), vec![1, 2]);
    }
}

#[cfg(test)]
mod library_tests {
    use super::find_libraries;
    use std::fs;

    #[test]
    fn finds_libraries_at_home_on_a_drive_and_one_folder_down() {
        let t = std::env::temp_dir().join(format!("alex-find-{}", std::process::id()));
        let _ = fs::remove_dir_all(&t);
        let lib = |p: &std::path::Path| {
            fs::create_dir_all(p).unwrap();
            fs::write(p.join("MANIFEST"), "x").unwrap();
        };
        lib(&t.join("home/Alexandria"));
        lib(&t.join("Volumes/Alexandria"));
        lib(&t.join("Volumes/MySSD/Alexandria-Full"));
        lib(&t.join("D/Alexandria"));
        fs::create_dir_all(t.join("home/Alexandria-notes")).unwrap();
        lib(&t.join("Volumes/MySSD/deep/Alexandria"));
        let got = find_libraries(Some(&t.join("home")), &[t.join("Volumes")], &[t.join("D")]);
        let rel: Vec<String> = got
            .iter()
            .map(|p| p.strip_prefix(&t).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(rel, ["D/Alexandria", "Volumes/Alexandria", "Volumes/MySSD/Alexandria-Full", "home/Alexandria"]);
        fs::remove_dir_all(&t).unwrap();
    }
}

#[cfg(test)]
mod pack_tests {
    use super::{default_pack, ordered_packs};
    use alexandria_core::Manifest;

    fn manifest(paths: &[&str]) -> Manifest {
        let z = "0".repeat(64);
        let mut t = String::from("alexandria-manifest 1\nrelease v1 2026-10-27\nchunk 1048576\n");
        for p in paths {
            t.push_str(&format!("{z} 1 {p}\n"));
        }
        Manifest::parse(&t).unwrap()
    }

    #[test]
    fn the_encyclopedia_opens_first_not_the_alphabetically_first_pack() {
        let m = manifest(&[
            "appropedia_en_all_maxi_2026-02.zim",
            "ifixit_en_all_2025-12.zim",
            "wikipedia_en-simple_all_nopic_2026-09.zim",
            "wikipedia_en_medicine_nopic_2026-10.zim",
            "zimgit-water_en_2024-08.zim",
            "some-new-community-pack.zim",
        ]);
        let order: Vec<String> = ordered_packs(&m).into_iter().map(|e| e.path).collect();
        assert_eq!(order, [
            "wikipedia_en-simple_all_nopic_2026-09.zim",
            "wikipedia_en_medicine_nopic_2026-10.zim",
            "ifixit_en_all_2025-12.zim",
            "appropedia_en_all_maxi_2026-02.zim",
            "zimgit-water_en_2024-08.zim",
            "some-new-community-pack.zim",
        ]);
        assert_eq!(default_pack(&m).unwrap().path, "wikipedia_en-simple_all_nopic_2026-09.zim");
    }

    #[test]
    fn full_wikipedia_beats_simple_when_both_exist() {
        let m = manifest(&["wikipedia_en-simple_all_nopic_2026-09.zim", "wikipedia_en_all_maxi_2026-08.zim"]);
        assert_eq!(default_pack(&m).unwrap().path, "wikipedia_en_all_maxi_2026-08.zim");
    }
}

#[cfg(test)]
mod roads_tests {
    use super::roads_entry;
    use alexandria_core::Manifest;

    fn manifest(paths: &[&str]) -> Manifest {
        let z = "0".repeat(64);
        let mut t = String::from("alexandria-manifest 1\nrelease v1 2026-10-27\nchunk 1048576\n");
        for p in paths {
            t.push_str(&format!("{z} 1 {p}\n"));
        }
        Manifest::parse(&t).unwrap()
    }

    #[test]
    fn routing_is_refused_when_the_library_has_no_roads_pack() {
        let m = manifest(&["wikipedia_en-simple.zim", "world.pmtiles"]);
        assert!(roads_entry(&m).is_err());
        let m = manifest(&["wikipedia_en-simple.zim", "roads-us.roads"]);
        assert_eq!(roads_entry(&m).unwrap().path, "roads-us.roads");
    }
}

#[cfg(test)]
mod pointer_tests {
    use super::{first_library, library_from_pointer};
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn a_known_library_opens_without_scanning_drives() {
        let known = vec![PathBuf::from("/lib")];
        let got = first_library(known, || panic!("scanned drives although a library was known"));
        assert_eq!(got, Some(PathBuf::from("/lib")));
        assert_eq!(first_library(Vec::new(), || vec![PathBuf::from("/found")]), Some(PathBuf::from("/found")));
    }

    #[test]
    fn the_installer_note_points_the_app_at_any_folder() {
        let t = std::env::temp_dir().join(format!("alex-ptr-{}", std::process::id()));
        let _ = fs::remove_dir_all(&t);
        let lib = t.join("Documents/Offline/Alexandria");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("MANIFEST"), "x").unwrap();
        let note = t.join("library-path");
        fs::write(&note, format!("{}\n", lib.display())).unwrap();
        assert_eq!(library_from_pointer(&note), Some(lib.clone()));
        fs::write(&note, t.join("nothing-here").display().to_string()).unwrap();
        assert_eq!(library_from_pointer(&note), None);
        fs::write(&note, format!("\u{feff}{}", lib.display())).unwrap();
        assert_eq!(library_from_pointer(&note), Some(lib.clone()));
        assert_eq!(library_from_pointer(&t.join("missing")), None);
        fs::remove_dir_all(&t).unwrap();
    }
}

#[cfg(test)]
mod newest_tests {
    use super::{edition_of, newest_release, release_record};
    use std::fs;

    #[test]
    fn an_older_library_than_one_already_opened_is_refused() {
        let t = std::env::temp_dir().join(format!("alex-newest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&t);
        fs::create_dir_all(&t).unwrap();
        let note = t.join("newest-release");
        assert!(newest_release(&note, "2026-10-27").is_ok());
        assert!(newest_release(&note, "2026-10-27").is_ok());
        assert!(newest_release(&note, "2027-01-15").is_ok());
        assert_eq!(fs::read_to_string(&note).unwrap(), "2027-01-15");
        let e = newest_release(&note, "2026-10-27").unwrap_err();
        assert!(e.contains("older"), "{e}");
        assert_eq!(fs::read_to_string(&note).unwrap(), "2027-01-15");
        fs::write(&note, "junk").unwrap();
        assert!(newest_release(&note, "2026-10-27").is_ok());
        fs::remove_dir_all(&t).unwrap();
    }

    #[test]
    fn full_releases_key_a_separate_record() {
        assert_eq!(edition_of("v0.2.0"), "lite");
        assert_eq!(edition_of("v0.2.0-full"), "full");
        assert_eq!(release_record("lite"), "newest-release");
        assert_eq!(release_record("full"), "newest-release-full");
    }

    #[test]
    fn full_and_lite_keep_separate_rollback_records() {
        let t = std::env::temp_dir().join(format!("alex-ed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&t);
        fs::create_dir_all(&t).unwrap();
        let lite = t.join("newest-release");
        let full = t.join("newest-release-full");
        assert!(newest_release(&lite, "2026-11-05").is_ok());
        assert!(newest_release(&full, "2026-11-04").is_ok());
        assert!(newest_release(&full, "2026-11-03").unwrap_err().contains("older"));
        assert!(newest_release(&lite, "2026-11-04").unwrap_err().contains("older"));
        assert_eq!(fs::read_to_string(&lite).unwrap(), "2026-11-05");
        assert_eq!(fs::read_to_string(&full).unwrap(), "2026-11-04");
        fs::remove_dir_all(&t).unwrap();
    }
}

#[cfg(test)]
mod image_tests {
    use super::clean_image;

    fn encoded(w: u32, h: u32, fmt: image::ImageFormat) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([x as u8, y as u8, 90]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, fmt).unwrap();
        out.into_inner()
    }

    #[test]
    fn a_photo_comes_back_as_a_fresh_png_no_larger_than_asked() {
        let png = clean_image(&encoded(640, 480, image::ImageFormat::Jpeg), 320).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let back = image::load_from_memory(&png).unwrap();
        assert_eq!((back.width(), back.height()), (320, 240));
        let small = clean_image(&encoded(100, 50, image::ImageFormat::Png), 320).unwrap();
        assert_eq!(image::load_from_memory(&small).unwrap().width(), 100);
    }

    #[test]
    fn garbage_and_huge_claims_are_refused() {
        assert!(clean_image(b"not an image at all", 320).is_err());
        let mut bomb = encoded(1, 1, image::ImageFormat::Png);
        bomb[16..24].copy_from_slice(&[0, 1, 0x86, 0xa0, 0, 1, 0x86, 0xa0]);
        assert!(clean_image(&bomb, 320).is_err());
    }
}

#[cfg(test)]
mod budget_tests {
    use super::{ImageBudget, IMAGE_ATTEMPT_COST, IMAGE_BUDGET};

    #[test]
    fn failed_lookups_consume_the_budget() {
        let mut b = ImageBudget::new();
        let mut attempts = 0;
        while b.cap().is_some() {
            attempts += 1;
            assert!(b.charge(None).is_none());
        }
        assert_eq!(attempts, IMAGE_BUDGET / IMAGE_ATTEMPT_COST);
        assert!(attempts < 64, "hostile lookups stop long before the 64 cap");
    }

    #[test]
    fn a_good_image_spends_only_its_length() {
        let mut b = ImageBudget::new();
        assert!(b.cap().is_some());
        let uri = "data:image/png;base64,AAAAAAAA".to_string();
        let before = b.left;
        assert_eq!(b.charge(Some(uri.clone())), Some(uri.clone()));
        assert_eq!(b.left, before - uri.len());
    }

    #[test]
    fn a_failed_lookup_charges_the_fixed_cost() {
        let mut b = ImageBudget::new();
        b.cap().unwrap();
        let before = b.left;
        assert!(b.charge(None).is_none());
        assert_eq!(b.left, before - IMAGE_ATTEMPT_COST);
    }
}

#[cfg(test)]
mod nav_tests {
    use super::{article_target, own_page, percent_decode};

    #[test]
    fn only_the_app_pages_may_load_in_the_window() {
        assert!(own_page("tauri://localhost/index.html"));
        assert!(own_page("http://tauri.localhost/index.html"));
        assert!(own_page("https://tauri.localhost/"));
        assert!(own_page("about:srcdoc"));
        assert!(own_page("about:blank"));
        assert!(!own_page("https://example.com/"));
        assert!(!own_page("http://tauri.localhost.example.com/"));
        assert!(!own_page("file:///etc/passwd"));
        assert!(!own_page("javascript:alert(1)"));
    }

    #[test]
    fn percent_decode_recovers_urls() {
        assert_eq!(percent_decode("A%2FWater"), "A/Water");
        assert_eq!(percent_decode("S%C3%A3o_Paulo"), "São_Paulo");
        assert_eq!(percent_decode("a%25b"), "a%b");
        assert_eq!(percent_decode("plain"), "plain");
    }

    #[test]
    fn only_the_article_scheme_is_a_link() {
        assert_eq!(
            article_target("alexandria-article:Wikipedia%20EN/A%2FWater"),
            Some(("Wikipedia EN".to_string(), "A/Water".to_string()))
        );
        assert_eq!(
            article_target("alexandria-article:ifixit.zim/A%2FWater"),
            Some(("ifixit.zim".to_string(), "A/Water".to_string()))
        );
        assert_eq!(article_target("https://example.com/x"), None);
        assert_eq!(article_target("tauri://localhost/index.html"), None);
        assert_eq!(article_target("about:srcdoc#a:A%2FWater"), None);
        assert_eq!(article_target("alexandria-article-x:y"), None);
        assert_eq!(article_target("alexandria-article:A%2FWater"), None);
    }
}

#[cfg(test)]
mod lock_tests {
    use super::with_db;
    use alexandria_core::{Entry, Manifest};
    use std::fs;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn search_does_not_wait_for_a_long_ask_read() {
        let t = std::env::temp_dir().join(format!("alex-lock-{}", std::process::id()));
        let _ = fs::remove_dir_all(&t);
        let dir = t.join("Alexandria");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(t.join("Alexandria-Index")).unwrap();
        let entry = |pack: &str| Entry { hashes_sha256: [0u8; 32], size: 0, path: pack.to_string() };
        for pack in ["a.zim", "b.zim"] {
            let db = t.join("Alexandria-Index").join(format!("{pack}.0000000000000000.search.db"));
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute("CREATE TABLE articles (title TEXT, url TEXT, body TEXT)", []).unwrap();
        }
        let m = Manifest { release: "v".into(), date: "2026-10-27".into(), chunk: 1048576, entries: Vec::new() };
        let (tx, rx) = mpsc::channel();
        let tdir = dir.clone();
        let tml = m.clone();
        let tentry = entry("a.zim");
        let slow = std::thread::spawn(move || {
            with_db(&tdir, &tml, &tentry, |db| {
                tx.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(500));
                let _ = db.conn();
                Some(())
            })
        });
        rx.recv().unwrap();
        let start = Instant::now();
        let got = with_db(&dir, &m, &entry("b.zim"), |db| {
            let _ = db.conn();
            Some(7)
        });
        let elapsed = start.elapsed();
        assert_eq!(got, Some(7));
        assert!(elapsed < Duration::from_millis(400), "search waited {elapsed:?} for the ask read");
        slow.join().unwrap();
        fs::remove_dir_all(&t).unwrap();
    }
}
