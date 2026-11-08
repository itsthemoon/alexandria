use alexandria_core::{
    hash_file, hex, load_signing_key, parse_verifying_key, sign_bytes, write_hashes,
    Entry, Manifest, VerifiedFile, CHUNK,
};
use alexandria_pmtiles::Pmtiles;
use alexandria_search::streets::Streets;
use alexandria_zim::{html_to_text, search, Source, Zim};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;

mod roads;

fn usage() -> ! {
    eprintln!(
        "usage:\n  alex keygen <seed-file> <pub-file>\n  alex hash <file>\n  alex hash-stream < <file> > <file.hashes>\n  alex manifest <dir> <release> <date>\n  alex sign <dir> <seed-file>\n  alex keys <dir> <root-seed-file> <release-pub-hex>...\n  alex verify <dir> <root-pub-hex>\n  alex article <dir> <pub-key-hex> <url>\n  alex list <dir> <pub-key-hex>\n  alex ask <dir> <pub-key-hex> <question>\n  alex sources <dir> <pub-key-hex> <question>\n  alex tile <dir> <pub-key-hex> <z> <x> <y> <out>\n  alex index <pack.zim> <out.search.db> [--lead <chars>]\n  alex index-library <dir> <root-pub-hex>\n  alex search <dir> <pub-key-hex> <query>\n  alex streets <dir> <pub-key-hex> [--near lat,lon] [--limit n] <query>
  alex roads-build <in.osm.pbf> <out.roads>
  alex route <dir> <pub-key-hex> <lat,lon> <lat,lon> [--json]
  alex route-file <file.roads> <lat,lon> <lat,lon> [--json]"
    );
    exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        usage();
    }
    let r = match args[1].as_str() {
        "keygen" if args.len() == 4 => keygen(&args[2], &args[3]),
        "hash" if args.len() == 3 => hash_cmd(&args[2]),
        "hash-stream" if args.len() == 2 => hash_stream_cmd(),
        "manifest" if args.len() == 5 => manifest_cmd(&args[2], &args[3], &args[4]),
        "sign" if args.len() == 4 => sign_cmd(&args[2], &args[3]),
        "verify" if args.len() == 4 => verify_cmd(&args[2], &args[3]),
        "keys" if args.len() >= 5 => keys_cmd(&args[2], &args[3], &args[4..]),
        "index-library" if args.len() == 4 => index_library_cmd(&args[2], &args[3]),
        "article" if args.len() == 5 => article_cmd(&args[2], &args[3], &args[4]),
        "list" if args.len() == 4 => list_cmd(&args[2], &args[3]),
        "ask" if args.len() >= 5 => ask_cmd(&args[2], &args[3], &args[4..].join(" ")),
        "sources" if args.len() >= 5 => sources_cmd(&args[2], &args[3], &args[4..].join(" ")),
        "tile" if args.len() == 8 => {
            tile_cmd(&args[2], &args[3], &args[4], &args[5], &args[6], &args[7])
        }
        "index" if args.len() == 4 => index_cmd(&args[2], &args[3], None),
        "index" if args.len() == 6 && args[4] == "--lead" => match args[5].parse() {
            Ok(n) => index_cmd(&args[2], &args[3], Some(n)),
            Err(_) => usage(),
        },
        "search" if args.len() >= 5 => search_cmd(&args[2], &args[3], &args[4..].join(" ")),
        "streets" if args.len() >= 5 => streets_cmd(&args[2], &args[3], &args[4..]),
        "roads-build" if args.len() == 4 => roads_build_cmd(&args[2], &args[3]),
        "route" if args.len() == 6 || args.len() == 7 => {
            let json = args.len() == 7 && args[6] == "--json";
            route_cmd(&args[2], &args[3], &args[4], &args[5], json)
        }
        "route-file" if args.len() == 5 || args.len() == 6 => {
            let json = args.len() == 6 && args[5] == "--json";
            route_file_cmd(&args[2], &args[3], &args[4], json)
        }
        _ => usage(),
    };
    if let Err(e) = r {
        eprintln!("error: {e}");
        exit(1);
    }
}

fn keygen(seed_out: &str, pub_out: &str) -> alexandria_core::Result<()> {
    let mut seed = [0u8; 32];
    getrandom(&mut seed)?;
    let key = load_signing_key(&hex(&seed))?;
    {
        use std::io::Write;
        let mut f = {
            let mut o = fs::OpenOptions::new();
            o.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                o.mode(0o600);
            }
            o.open(seed_out)?
        };
        f.write_all(hex(&seed).as_bytes())?;
        f.sync_all()?;
    }
    fs::write(pub_out, hex(key.verifying_key().as_bytes()))?;
    println!("seed written to {seed_out} (keep offline)");
    println!("public key written to {pub_out}");
    Ok(())
}

fn getrandom(buf: &mut [u8]) -> alexandria_core::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Read;
        let mut f = fs::File::open("/dev/urandom")?;
        f.read_exact(buf)?;
        return Ok(());
    }
    #[cfg(not(unix))]
    {
        #[link(name = "bcrypt")]
        extern "system" {
            fn BCryptGenRandom(
                alg: *mut core::ffi::c_void,
                buf: *mut u8,
                len: u32,
                flags: u32,
            ) -> i32;
        }
        let rc =
            unsafe { BCryptGenRandom(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32, 2) };
        if rc != 0 {
            return Err(alexandria_core::Error::BadKey("system RNG failed".into()));
        }
        Ok(())
    }
}

fn hash_cmd(file: &str) -> alexandria_core::Result<()> {
    let p = Path::new(file);
    let h = hash_file(p, CHUNK)?;
    let out = write_hashes(p, CHUNK)?;
    println!("{} chunks -> {}", h.len() / 32, out.display());
    Ok(())
}

fn hash_stream_cmd() -> alexandria_core::Result<()> {
    use std::io::Write;
    let (hashes, size) = alexandria_core::hash_reader(&mut std::io::stdin().lock(), CHUNK)?;
    std::io::stdout().write_all(&hashes)?;
    eprintln!("{size}");
    Ok(())
}

fn manifest_cmd(dir: &str, release: &str, date: &str) -> alexandria_core::Result<()> {
    let dir = Path::new(dir);
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    collect_files(dir, dir, &mut files)?;
    files.sort_by(|a, b| a.1.cmp(&b.1));

    let mut entries = Vec::new();
    for (p, rel) in &files {
        let n = rel.as_str();
        let size = fs::metadata(p)?.len();
        let hashes = write_hashes(p, CHUNK)?;
        let hashes_bytes = fs::read(&hashes)?;
        let root = sha256(&hashes_bytes);
        entries.push(Entry {
            hashes_sha256: root,
            size,
            path: n.to_string(),
        });
        println!("hashed {n} ({size} bytes)");
    }
    let m = Manifest {
        release: release.to_string(),
        date: date.to_string(),
        chunk: CHUNK,
        entries,
    };
    fs::write(dir.join("MANIFEST"), m.to_text())?;
    println!("wrote {}", dir.join("MANIFEST").display());
    Ok(())
}

fn manifest_name_ok(rel: &str) -> bool {
    !rel.is_empty()
        && rel.split('/').all(|seg| {
            !seg.is_empty()
                && !seg.ends_with('.')
                && seg.chars().all(|c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')
                })
        })
}

fn collect_files(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(PathBuf, String)>,
) -> alexandria_core::Result<()> {
    for e in fs::read_dir(dir)? {
        let e = e?;
        let ft = e.file_type()?;
        if ft.is_symlink() {
            continue;
        }
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if ft.is_dir() {
            if name.eq_ignore_ascii_case("Personal") {
                continue;
            }
            collect_files(root, &p, out)?;
            continue;
        }
        if name.ends_with(".hashes")
            || matches!(name.as_str(), "MANIFEST" | "MANIFEST.sig" | "KEYS" | "KEYS.sig")
        {
            continue;
        }
        let rel = p.strip_prefix(root).unwrap_or(&p);
        let rel = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        if !manifest_name_ok(&rel) {
            return Err(alexandria_core::Error::BadPath(rel));
        }
        out.push((p.clone(), rel));
    }
    Ok(())
}

struct RawFile(std::fs::File, u64);

impl Source for RawFile {
    fn size(&self) -> u64 {
        self.1
    }
    fn read_at(&mut self, offset: u64, len: usize) -> alexandria_zim::Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        self.0
            .seek(SeekFrom::Start(offset))
            .map_err(|e| alexandria_zim::Error::Source(e.to_string()))?;
        let mut b = vec![0u8; len];
        self.0
            .read_exact(&mut b)
            .map_err(|e| alexandria_zim::Error::Source(e.to_string()))?;
        Ok(b)
    }
}

fn sqlite_err(e: rusqlite::Error) -> alexandria_core::Error {
    alexandria_core::Error::Io(std::io::Error::new(
        std::io::ErrorKind::Other,
        e.to_string(),
    ))
}

fn zim_err(e: alexandria_zim::Error) -> alexandria_core::Error {
    alexandria_core::Error::Io(std::io::Error::new(
        std::io::ErrorKind::Other,
        e.to_string(),
    ))
}

fn open_index(d: &Path, m: &Manifest, zim: &Entry) -> Option<alexandria_search::VerifiedDb> {
    let index_path = format!("{}.search.db", zim.path);
    match m.entries.iter().find(|e| e.path == index_path) {
        Some(ie) => {
            let vf = VerifiedFile::open_in(d, ie, m.chunk).ok()?;
            alexandria_search::open_verified(vf, m.chunk).ok()
        }
        None => alexandria_search::open_local(&alexandria_search::local_index_path(d, zim)).ok(),
    }
}

fn search_cmd(dir: &str, pub_hex: &str, q: &str) -> alexandria_core::Result<()> {
    let d = Path::new(dir);
    let m = open_manifest(d, pub_hex)?;
    let other = |e: String| alexandria_core::Error::Io(std::io::Error::new(std::io::ErrorKind::Other, e));
    let mut hits = Vec::new();
    let mut indexed = 0;
    for zim in m.entries.iter().filter(|e| e.path.ends_with(".zim")) {
        if let Some(db) = open_index(d, &m, zim) {
            indexed += 1;
            hits.extend(alexandria_search::query(db.conn(), q).map_err(other)?);
        }
    }
    if indexed == 0 {
        return Err(alexandria_core::Error::NotAFile(
            "no search index: run `alex index-library` first".into(),
        ));
    }
    hits.sort_by_key(|h| !h.title.eq_ignore_ascii_case(q.trim()));
    for h in &hits {
        println!("{}\t{}", h.title, h.url);
    }
    println!("{} hits", hits.len());
    Ok(())
}

fn ocr_beside(pack: &Path) -> Option<alexandria_search::ocr::Ocr> {
    let mut name = pack.file_name()?.to_os_string();
    name.push(".ocr");
    std::fs::read(pack.with_file_name(name))
        .ok()
        .map(|b| alexandria_search::ocr::Ocr::parse(&b))
        .filter(|o| !o.is_empty())
}

fn ocr_in(dir: &Path, m: &Manifest, pack: &str) -> Option<alexandria_search::ocr::Ocr> {
    let path = format!("{pack}.ocr");
    let entry = m.entries.iter().find(|e| e.path == path)?;
    let mut vf = VerifiedFile::open_in(dir, entry, m.chunk).ok()?;
    let size = vf.size() as usize;
    if size > 64 * 1024 * 1024 {
        return None;
    }
    let bytes = vf.read_at(0, size).ok()?;
    Some(alexandria_search::ocr::Ocr::parse(&bytes)).filter(|o| !o.is_empty())
}

fn index_cmd(zim_path: &str, out_path: &str, lead: Option<usize>) -> alexandria_core::Result<()> {
    let f = std::fs::File::open(zim_path)?;
    let size = f.metadata()?.len();
    let mut z = Zim::open(RawFile(f, size)).map_err(zim_err)?;
    let ocr = ocr_beside(Path::new(zim_path));
    let (n, total) = build_index(&mut z, Path::new(out_path), lead, ocr.as_ref(), size)?;
    println!("indexed {n} articles from {total} entries -> {out_path}");
    Ok(())
}

fn index_library_cmd(dir: &str, pub_hex: &str) -> alexandria_core::Result<()> {
    let dir = Path::new(dir);
    let m = open_manifest(dir, pub_hex)?;
    let packs: Vec<&Entry> = m.entries.iter().filter(|e| e.path.ends_with(".zim")).collect();
    for (i, e) in packs.iter().enumerate() {
        let out = alexandria_search::local_index_path(dir, e);
        if out.is_file() {
            println!("[{}/{}] {} (search ready)", i + 1, packs.len(), e.path);
            continue;
        }
        println!("[{}/{}] {} (building search)", i + 1, packs.len(), e.path);
        fs::create_dir_all(out.parent().unwrap_or(dir))?;
        let part = out.with_extension("db.part");
        let vf = VerifiedFile::open_in(dir, e, m.chunk)?;
        let pack_bytes = vf.size();
        let mut z = Zim::open(vf).map_err(zim_err)?;
        let ocr = ocr_in(dir, &m, &e.path);
        let (n, _) = build_index(&mut z, &part, None, ocr.as_ref(), pack_bytes)?;
        fs::rename(&part, &out)?;
        println!("      {n} articles");
    }
    println!("search ready for {} packs", packs.len());
    Ok(())
}

const INDEX_SCHEMA_VERSION: i64 = 1;

fn remove_db_files(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}

fn build_index<S: Source>(
    z: &mut Zim<S>,
    out_path: &Path,
    lead: Option<usize>,
    ocr: Option<&alexandria_search::ocr::Ocr>,
    pack_bytes: u64,
) -> alexandria_core::Result<(usize, usize)> {
    let total = z.header.article_count as usize;
    let mut conn = rusqlite::Connection::open(out_path).map_err(sqlite_err)?;
    let (done, stale) = match conn.query_row(
        "SELECT schema FROM progress LIMIT 1",
        [],
        |r| r.get::<_, i64>(0),
    ) {
        Ok(v) if v == INDEX_SCHEMA_VERSION => (
            conn.query_row(
                "SELECT seen, kept FROM progress WHERE total = ?1 AND bytes = ?2 ORDER BY seen DESC LIMIT 1",
                rusqlite::params![total as i64, pack_bytes as i64],
                |r| Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)? as usize)),
            )
            .ok(),
            false,
        ),
        Ok(_) => (None, true),
        Err(_) => (
            None,
            conn.query_row("SELECT COUNT(*) FROM progress", [], |r| r.get::<_, i64>(0))
                .map(|n| n > 0)
                .unwrap_or(false),
        ),
    };
    if done.is_none() {
        drop(conn);
        remove_db_files(out_path);
        if stale {
            eprintln!("      index schema version mismatch; rebuilding from scratch");
        }
        conn = rusqlite::Connection::open(out_path).map_err(sqlite_err)?;
    }
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA temp_store=MEMORY;
        PRAGMA wal_autocheckpoint=0;
        CREATE TABLE IF NOT EXISTS progress(seen INTEGER NOT NULL, kept INTEGER NOT NULL,
        total INTEGER NOT NULL, bytes INTEGER NOT NULL, schema INTEGER NOT NULL DEFAULT 1);
        CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE VIRTUAL TABLE IF NOT EXISTS articles USING fts5(title, body, content='',
        tokenize='unicode61 remove_diacritics 2');
        CREATE TABLE IF NOT EXISTS docs(id INTEGER PRIMARY KEY, title TEXT NOT NULL, url TEXT NOT NULL);",
    )
    .map_err(sqlite_err)?;
    let (mut seen, mut inserted) = done.unwrap_or((0, 0));
    if seen > 0 {
        eprintln!("      resuming after {inserted} of about {total} entries");
    }
    #[cfg(debug_assertions)]
    let stop_after: Option<usize> = std::env::var("ALEX_INDEX_STOP_AFTER")
        .ok()
        .and_then(|v| v.parse().ok());
    #[cfg(not(debug_assertions))]
    let stop_after: Option<usize> = None;
    let mut candidates: Vec<(String, u32, u32, u32, u32)> = Vec::new();
    for i in 0..total {
        let Ok(e) = z.entry(i as u32) else { continue };
        if e.mime == 0xFFFF || e.mime >= 0xFFFD || e.title.is_empty() {
            continue;
        }
        if !matches!(z.mime(e.mime).unwrap_or(""), "text/html" | "text/plain" | "application/pdf") {
            continue;
        }
        candidates.push((e.title, i as u32, e.cluster, e.blob, 0));
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    for (rank, c) in candidates.iter_mut().enumerate() {
        c.4 = rank as u32;
    }
    candidates.sort_by(|a, b| (a.2, a.3).cmp(&(b.2, b.3)).then(a.1.cmp(&b.1)));
    let ordered: Vec<(u32, u32)> = candidates.into_iter().map(|c| (c.1, c.4)).collect();

    let mut tx = conn.unchecked_transaction().map_err(sqlite_err)?;
    for (i, (url_idx, rank)) in ordered.into_iter().enumerate().skip(seen) {
        seen = i + 1;
        let Ok(e) = z.entry(url_idx) else { continue };
        let mime = z.mime(e.mime).unwrap_or("").to_string();
        let Ok(raw) = z.read_entry_blob(&e) else { continue };
        let (title, text) = match mime.as_str() {
            "text/html" => (e.title.clone(), html_to_text(&String::from_utf8_lossy(&raw))),
            "text/plain" => (e.title.clone(), String::from_utf8_lossy(&raw).to_string()),
            _ => match alexandria_search::pdf_text(&raw).filter(|(_, t)| t.split_whitespace().count() >= 20) {
                Some((meta, text)) => (alexandria_search::pdf_title(meta.as_deref(), &text, &e.title), text),
                None => match ocr.and_then(|o| o.text(&e.url)) {
                    Some(text) => (alexandria_search::pdf_name(&e.title), text.to_string()),
                    None => continue,
                },
            },
        };
        if text.chars().filter(|c| c.is_alphanumeric()).count() < 20 {
            continue;
        }
        let text: String = match lead {
            Some(n) => text.chars().take(n).collect(),
            None => text,
        };
        tx.execute(
            "INSERT INTO docs(id, title, url) VALUES (?1, ?2, ?3)",
            rusqlite::params![rank as i64, title.as_str(), e.url.as_str()],
        )
        .map_err(sqlite_err)?;
        tx.execute(
            "INSERT INTO articles(rowid, title, body) VALUES (?1, ?2, ?3)",
            rusqlite::params![rank as i64, title, text],
        )
        .map_err(sqlite_err)?;
        inserted += 1;
        let stop = stop_after.map(|n| inserted >= n).unwrap_or(false);
        if inserted % 1000 == 0 || stop {
            tx.execute("DELETE FROM progress", []).map_err(sqlite_err)?;
            tx.execute(
                "INSERT INTO progress(seen, kept, total, bytes, schema) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![seen as i64, inserted as i64, total as i64, pack_bytes as i64, INDEX_SCHEMA_VERSION],
            )
            .map_err(sqlite_err)?;
            tx.commit().map_err(sqlite_err)?;
            conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);")
                .map_err(sqlite_err)?;
            if stop {
                return Err(alexandria_core::Error::NotAFile("stopped for test".into()));
            }
            eprintln!("      indexed {inserted}/{total}");
            tx = conn.unchecked_transaction().map_err(sqlite_err)?;
        }
    }
    tx.execute("DELETE FROM meta", []).map_err(sqlite_err)?;
    tx.execute("INSERT INTO meta(key, value) VALUES ('order', 'title')", [])
        .map_err(sqlite_err)?;
    tx.execute(
        "INSERT INTO meta(key, value) VALUES ('articles', ?1)",
        [inserted as i64],
    )
    .map_err(sqlite_err)?;
    tx.execute("DELETE FROM progress", []).map_err(sqlite_err)?;
    tx.commit().map_err(sqlite_err)?;
    conn = rusqlite::Connection::open(out_path).map_err(sqlite_err)?;
    conn.execute_batch(
        "PRAGMA temp_store=FILE; INSERT INTO articles(articles) VALUES('optimize'); PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;",
    )
    .map_err(sqlite_err)?;
    Ok((inserted, total))
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

fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

fn sign_cmd(dir: &str, seed_file: &str) -> alexandria_core::Result<()> {
    let dir = Path::new(dir);
    let raw = fs::read(dir.join("MANIFEST"))?;
    let m = Manifest::parse(&String::from_utf8_lossy(&raw))?;
    let seed = fs::read_to_string(seed_file)?;
    let key = load_signing_key(&seed)?;
    let sig = sign_bytes(&key, &raw);
    fs::write(dir.join("MANIFEST.sig"), sig)?;
    println!(
        "signed {} ({} entries)",
        dir.join("MANIFEST").display(),
        m.entries.len()
    );
    Ok(())
}

fn verify_cmd(dir: &str, pub_hex: &str) -> alexandria_core::Result<()> {
    let dir = Path::new(dir);
    let m = open_manifest(dir, pub_hex)?;
    println!("manifest signature ok: release {} {}", m.release, m.date);

    for e in &m.entries {
        let mut f = VerifiedFile::open_in(dir, e, m.chunk)?;
        f.verify_all()?;
        println!("ok {} ({} bytes)", e.path, e.size);
    }
    println!("{} files verified", m.entries.len());
    Ok(())
}

fn open_manifest(dir: &Path, pub_hex: &str) -> alexandria_core::Result<Manifest> {
    let root = parse_verifying_key(pub_hex)?;
    Ok(alexandria_core::open_signed(dir, &root)?.manifest)
}

fn keys_cmd(dir: &str, root_seed_file: &str, keys: &[String]) -> alexandria_core::Result<()> {
    let dir = Path::new(dir);
    let root = load_signing_key(&fs::read_to_string(root_seed_file)?)?;
    let mut text = String::from("alexandria-keys 1\n");
    for k in keys {
        let k = parse_verifying_key(k)?;
        text.push_str(&hex(k.as_bytes()));
        text.push('\n');
    }
    let sig = alexandria_core::sign_keys(&root, text.as_bytes());
    alexandria_core::verify_keys(&root.verifying_key(), text.as_bytes(), &sig)?;
    fs::write(dir.join("KEYS"), &text)?;
    fs::write(dir.join("KEYS.sig"), sig)?;
    println!("wrote KEYS with {} release key(s), signed by root {}", keys.len(),
        hex(root.verifying_key().as_bytes()));
    Ok(())
}

fn list_cmd(dir: &str, pub_hex: &str) -> alexandria_core::Result<()> {
    let m = open_manifest(Path::new(dir), pub_hex)?;
    println!("release {} {}", m.release, m.date);
    for e in &m.entries {
        println!("{:>12}  {}", e.size, e.path);
    }
    Ok(())
}

fn article_cmd(dir: &str, pub_hex: &str, url: &str) -> alexandria_core::Result<()> {
    let d = Path::new(dir);
    let m = open_manifest(d, pub_hex)?;
    let packs: Vec<Entry> = m.entries.iter().filter(|e| e.path.ends_with(".zim")).cloned().collect();
    let mut tried = 0;
    for entry in &packs {
        let vf = VerifiedFile::open_in(d, entry, m.chunk)?;
        let mut z = Zim::open(vf).map_err(|e| alexandria_core::Error::BadPath(format!("{}: {e}", entry.path)))?;
        tried += 1;
        let Ok(idx) = z.find_index('C', url).or_else(|_| z.find_index('A', url)) else {
            continue;
        };
        let body = z
            .read_article(idx)
            .map_err(|e| alexandria_core::Error::BadPath(format!("{url}: {e}")))?;
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    Err(alexandria_core::Error::NotAFile(format!("{url} is in none of the {tried} packs")))
}

fn streets_cmd(dir: &str, pub_hex: &str, rest: &[String]) -> alexandria_core::Result<()> {
    let mut near = None;
    let mut limit = 8usize;
    let mut words = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--near" => {
                i += 1;
                let parts: Vec<f32> = rest
                    .get(i)
                    .map(|v| v.split(',').filter_map(|n| n.parse().ok()).collect())
                    .unwrap_or_default();
                if parts.len() == 2 {
                    near = Some((parts[0], parts[1]));
                }
            }
            "--limit" => {
                i += 1;
                if let Some(n) = rest.get(i).and_then(|v| v.parse().ok()) {
                    limit = n;
                }
            }
            w => words.push(w.to_string()),
        }
        i += 1;
    }
    let query = words.join(" ");
    let d = Path::new(dir);
    let m = open_manifest(d, pub_hex)?;
    let Some(entry) = m.entries.iter().find(|e| e.path == "streets.txt").cloned() else {
        return Ok(());
    };
    let vf = VerifiedFile::open_in(d, &entry, m.chunk)?;
    let s = Streets::open_verified(vf, m.chunk).ok_or_else(|| {
        alexandria_core::Error::NotAFile(format!("no street index at {}", entry.path))
    })?;
    for (name, lat, lon, place) in s.find(&query, near, limit) {
        match place {
            Some(p) => println!("{name}\t{lat}\t{lon}\t{p}"),
            None => println!("{name}\t{lat}\t{lon}"),
        }
    }
    Ok(())
}

fn verified_copy(root: &Path, entry: &Entry, chunk: u64) -> alexandria_core::Result<PathBuf> {
    let p = alexandria_core::cached_copy(root, entry, chunk, &alexandria_core::cache_dir())?;
    eprintln!("model verified: {} bytes", entry.size);
    Ok(p)
}

fn ask_sources(d: &Path, m: &Manifest, question: &str) -> alexandria_core::Result<Vec<(String, String, String)>> {
    let zims: Vec<Entry> = m.entries.iter().filter(|e| e.path.ends_with(".zim")).cloned().collect();
    let mut all: Vec<(String, alexandria_search::Pick)> = Vec::new();
    std::thread::scope(|s| {
        let handles: Vec<_> = zims
            .iter()
            .map(|zim| {
                s.spawn(move || -> Vec<(String, alexandria_search::Pick)> {
                    let Some(db) = open_index(d, m, zim) else { return Vec::new() };
                    let Ok(vf) = VerifiedFile::open_in(d, zim, m.chunk) else { return Vec::new() };
                    let Ok(z) = Zim::open(vf) else { return Vec::new() };
                    let z = std::sync::Mutex::new(z);
                    let ocr = ocr_in(d, m, &zim.path);
                    let found = alexandria_search::ask_candidates(db.conn(), &zim.path, question, &|url| {
                        let (raw, mime, e_url) = {
                            let mut z = z.lock().unwrap();
                            let idx = z.find_index('C', url).or_else(|_| z.find_index('A', url)).ok()?;
                            let e = z.resolve(idx).ok()?;
                            let mime = z.mime(e.mime).map(str::to_string);
                            let raw = z.read_entry_blob(&e).ok()?;
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
                    });
                    found.unwrap_or_default().into_iter().map(|p| (zim.path.clone(), p)).collect()
                })
            })
            .collect();
        for h in handles {
            if let Ok(v) = h.join() {
                all.extend(v);
            }
        }
    });
    let mut picked: Vec<(String, String)> = alexandria_search::pick_sources(all, 3, question).into_iter().map(|(z, p)| (z, p.url)).collect();
    if picked.is_empty() {
        if let Some(zim) = zims.first() {
            let mut z = Zim::open(VerifiedFile::open_in(d, zim, m.chunk)?).map_err(|e| alexandria_core::Error::NotAFile(format!("zim: {e}")))?;
            let hits = search(&mut z, question, 3).map_err(|e| alexandria_core::Error::NotAFile(format!("search: {e}")))?;
            picked = hits.into_iter().map(|h| (zim.path.clone(), h.url)).collect();
        }
    }
    let mut out = Vec::new();
    for (pack, url) in picked {
        let Some(zim) = zims.iter().find(|e| e.path == pack) else { continue };
        let mut z = Zim::open(VerifiedFile::open_in(d, zim, m.chunk)?).map_err(|e| alexandria_core::Error::NotAFile(format!("zim: {e}")))?;
        let Ok(idx) = z.find_index('C', &url).or_else(|_| z.find_index('A', &url)) else { continue };
        let Ok(e) = z.resolve(idx) else { continue };
        let pdf = z.mime(e.mime) == Some("application/pdf");
        let Ok(raw) = z.read_article(idx) else { continue };
        let (title, text) = if pdf {
            match alexandria_search::pdf_text(&raw).filter(|(_, t)| t.split_whitespace().count() >= 20) {
                Some((meta, text)) => (alexandria_search::pdf_title(meta.as_deref(), &text, &e.title), text),
                None => match ocr_in(d, m, &pack).and_then(|o| o.text(&e.url).map(str::to_string)) {
                    Some(text) => (alexandria_search::pdf_name(&e.title), text),
                    None => continue,
                },
            }
        } else {
            (if e.title.is_empty() { e.url.clone() } else { e.title.clone() }, html_to_text(&String::from_utf8_lossy(&raw)))
        };
        let passage = alexandria_search::best_passage(&prompts::compact(&text), question, 1500);
        out.push((title, pack, passage));
    }
    Ok(out)
}

fn sources_cmd(dir: &str, pub_hex: &str, question: &str) -> alexandria_core::Result<()> {
    let d = Path::new(dir);
    let m = open_manifest(d, pub_hex)?;
    for (title, pack, _) in ask_sources(d, &m, question)? {
        println!("{pack}\t{title}");
    }
    Ok(())
}

fn ask_cmd(dir: &str, pub_hex: &str, question: &str) -> alexandria_core::Result<()> {
    let d = Path::new(dir);
    let m = open_manifest(d, pub_hex)?;
    let entry = m
        .entries
        .iter()
        .filter(|e| e.path.ends_with(".gguf"))
        .next()
        .ok_or_else(|| alexandria_core::Error::NotAFile("no .gguf model in manifest".into()))?
        .clone();

    let mut context = String::new();
    let mut sources: Vec<String> = Vec::new();
    for (title, _, passage) in ask_sources(d, &m, question)? {
        context.push_str(&format!("[{}] {title}\n{passage}\n\n", sources.len() + 1));
        sources.push(title);
    }

    let tmp = verified_copy(d, &entry, m.chunk)?;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let (system, user) = prompts::ask_prompt(&context, question.trim());
    let tpl = alexandria_core::cache_dir().join("lfm3-chat-cli.jinja");
    fs::create_dir_all(alexandria_core::cache_dir())?;
    fs::write(&tpl, CHAT_TEMPLATE)?;
    let out = base_command(llama_bin().map_err(alexandria_core::Error::NotAFile)?)
        .arg("-m")
        .arg(&tmp)
        .arg("--jinja")
        .arg("--chat-template-file")
        .arg(&tpl)
        .args(["-cnv", "-st", "--no-warmup", "--simple-io", "-c", "8192", "--no-display-prompt"])
        .arg("-sys")
        .arg(&system)
        .arg("-p")
        .arg(&user)
        .args(["-n", "600", "--temp", "0.1", "--top-k", "50", "--repeat-penalty", "1.1"])
        .arg("-t")
        .arg(threads.to_string())
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| alexandria_core::Error::NotAFile(format!("llama-completion: {e}")))?;

    let produced = !String::from_utf8_lossy(&out.stdout).trim().is_empty();
    if !out.status.success() {
        if !produced {
            let err = String::from_utf8_lossy(&out.stderr);
            let tail: Vec<&str> = err.lines().rev().take(5).collect();
            return Err(alexandria_core::Error::NotAFile(format!(
                "llama-completion failed ({}): {}",
                out.status,
                tail.into_iter().rev().collect::<Vec<_>>().join(" / ")
            )));
        }
        eprintln!("note: llama-completion exited with {} after answering", out.status);
    }
    if !sources.is_empty() {
        eprintln!("sources: {}", sources.join(" | "));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let text = prompts::visible_answer(text.split(" [end of text]").next().unwrap_or(""));
    println!("{text}");
    Ok(())
}

#[path = "../../app/src/prompts.rs"]
#[allow(dead_code)]
mod prompts;
use prompts::CHAT_TEMPLATE;

fn tile_cmd(
    dir: &str,
    pub_hex: &str,
    z: &str,
    x: &str,
    y: &str,
    out: &str,
) -> alexandria_core::Result<()> {
    let d = Path::new(dir);
    let m = open_manifest(d, pub_hex)?;
    let entry = m
        .entries
        .iter()
        .find(|e| e.path.ends_with(".pmtiles"))
        .ok_or_else(|| alexandria_core::Error::NotAFile("no .pmtiles in manifest".into()))?
        .clone();
    let vf = VerifiedFile::open_in(d, &entry, m.chunk)?;
    let mut p =
        Pmtiles::open(vf).map_err(|e| alexandria_core::Error::NotAFile(format!("pmtiles: {e}")))?;
    let z: u8 = z
        .parse()
        .map_err(|_| alexandria_core::Error::BadNumber(z.into()))?;
    let x: u32 = x
        .parse()
        .map_err(|_| alexandria_core::Error::BadNumber(x.into()))?;
    let y: u32 = y
        .parse()
        .map_err(|_| alexandria_core::Error::BadNumber(y.into()))?;
    let bytes = p
        .tile(z, x, y)
        .map_err(|e| alexandria_core::Error::NotAFile(format!("tile: {e}")))?;
    fs::write(out, &bytes)?;
    println!("wrote {out} ({} bytes)", bytes.len());
    Ok(())
}

fn roads_err(e: String) -> alexandria_core::Error {
    alexandria_core::Error::Io(std::io::Error::new(std::io::ErrorKind::Other, e))
}

fn parse_latlon(s: &str) -> alexandria_core::Result<(f64, f64)> {
    let mut parts = s.split(',');
    let lat = parts
        .next()
        .and_then(|v| v.parse::<f64>().ok())
        .ok_or_else(|| alexandria_core::Error::BadNumber(s.into()))?;
    let lon = parts
        .next()
        .and_then(|v| v.parse::<f64>().ok())
        .ok_or_else(|| alexandria_core::Error::BadNumber(s.into()))?;
    Ok((lat, lon))
}

fn print_route(route: &alexandria_roads::Route, json: bool) {
    if json {
        println!("{}", serde_json::to_string(route).unwrap_or_default());
        return;
    }
    println!("distance: {:.1} km", route.distance_m / 1000.0);
    println!("duration: {:.1} min", route.duration_s / 60.0);
    for (i, s) in route.steps.iter().enumerate() {
        let dist = if s.distance_m >= 1000.0 {
            format!("{:.1} km", s.distance_m / 1000.0)
        } else {
            format!("{:.0} m", s.distance_m)
        };
        println!("  {}. {} ({})", i + 1, s.text, dist);
    }
}

fn roads_build_cmd(in_p: &str, out_p: &str) -> alexandria_core::Result<()> {
    roads::build(in_p, out_p).map_err(roads_err)?;
    println!("wrote {out_p}");
    Ok(())
}

fn route_with(vf: VerifiedFile, from: (f64, f64), to: (f64, f64), json: bool) -> alexandria_core::Result<()> {
    let mut r = alexandria_roads::Roads::open(vf).map_err(roads_err)?;
    let route = r.route(from, to).map_err(roads_err)?;
    print_route(&route, json);
    Ok(())
}

fn route_cmd(dir: &str, pub_hex: &str, from: &str, to: &str, json: bool) -> alexandria_core::Result<()> {
    let d = Path::new(dir);
    let m = open_manifest(d, pub_hex)?;
    let entry = m
        .entries
        .iter()
        .find(|e| e.path == "roads-us.roads")
        .cloned()
        .ok_or_else(|| alexandria_core::Error::NotAFile("no roads-us.roads in this library".into()))?;
    let vf = VerifiedFile::open_in(d, &entry, m.chunk)?;
    route_with(vf, parse_latlon(from)?, parse_latlon(to)?, json)
}

fn route_file_cmd(file: &str, from: &str, to: &str, json: bool) -> alexandria_core::Result<()> {
    let p = Path::new(file);
    let size = fs::metadata(p)?.len();
    let hashes = alexandria_core::write_hashes(p, CHUNK)?;
    let raw = fs::read(&hashes)?;
    let entry = Entry {
        hashes_sha256: sha256(&raw),
        size,
        path: p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "roads".into()),
    };
    let vf = VerifiedFile::open(p, &entry, CHUNK)?;
    route_with(vf, parse_latlon(from)?, parse_latlon(to)?, json)
}
