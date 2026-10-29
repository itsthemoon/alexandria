use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const DOMAIN: &[u8] = b"alexandria-manifest-v1\0";
pub const CHUNK: u64 = 1 << 20;
const MAX_CHUNK: u64 = 1 << 26;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    BadHeader,
    BadLine(String),
    BadPath(String),
    DuplicatePath(String),
    BadNumber(String),
    BadHex(String),
    SizeMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    HashesMismatch {
        path: String,
    },
    ChunkMismatch {
        index: u64,
    },
    OutOfBounds,
    BadSignature,
    BadKey(String),
    NotAFile(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io: {e}"),
            Error::BadHeader => write!(f, "not an alexandria manifest (v1)"),
            Error::BadLine(_) => write!(f, "bad manifest line"),
            Error::BadPath(_) => write!(f, "unsafe path in manifest"),
            Error::DuplicatePath(p) => write!(f, "duplicate path: {p}"),
            Error::BadNumber(_) => write!(f, "bad number"),
            Error::BadHex(_) => write!(f, "bad hex"),
            Error::SizeMismatch {
                path,
                expected,
                actual,
            } => {
                write!(f, "{path}: expected {expected} bytes, found {actual}")
            }
            Error::HashesMismatch { path } => {
                write!(f, "{path}: hash list does not match manifest")
            }
            Error::ChunkMismatch { index } => write!(f, "chunk {index} failed verification"),
            Error::OutOfBounds => write!(f, "read outside the file"),
            Error::BadSignature => write!(f, "manifest signature is not valid"),
            Error::BadKey(k) => write!(f, "bad public key: {k}"),
            Error::NotAFile(p) => write!(f, "not a regular file: {p}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn hex(bytes: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(D[(b >> 4) as usize] as char);
        s.push(D[(b & 15) as usize] as char);
    }
    s
}

pub fn unhex(s: &str) -> Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        return Err(Error::BadHex(s.into()));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let b = s.as_bytes();
    let val = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    for i in (0..b.len()).step_by(2) {
        let hi = val(b[i]).ok_or_else(|| Error::BadHex(s.into()))?;
        let lo = val(b[i + 1]).ok_or_else(|| Error::BadHex(s.into()))?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

pub fn chunk_count(size: u64, chunk: u64) -> u64 {
    if size == 0 {
        0
    } else {
        (size + chunk - 1) / chunk
    }
}

pub fn hash_file(path: &Path, chunk: u64) -> Result<Vec<u8>> {
    let mut f = File::open(path)?;
    let size = f.metadata()?.len();
    let n = chunk_count(size, chunk);
    let mut out = Vec::with_capacity(n as usize * 32);
    let mut buf = vec![0u8; chunk as usize];
    for _ in 0..n {
        let mut filled = 0usize;
        while filled < buf.len() {
            let r = f.read(&mut buf[filled..])?;
            if r == 0 {
                break;
            }
            filled += r;
        }
        if filled == 0 {
            return Err(Error::SizeMismatch {
                path: path.display().to_string(),
                expected: size,
                actual: (out.len() / 32) as u64 * chunk,
            });
        }
        out.extend_from_slice(&Sha256::digest(&buf[..filled]));
    }
    Ok(out)
}

pub fn write_hashes(path: &Path, chunk: u64) -> Result<PathBuf> {
    let hashes = hash_file(path, chunk)?;
    let out = PathBuf::from(format!("{}.hashes", path.display()));
    let mut f = File::create(&out)?;
    f.write_all(&hashes)?;
    f.sync_all()?;
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub hashes_sha256: [u8; 32],
    pub size: u64,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub release: String,
    pub date: String,
    pub chunk: u64,
    pub entries: Vec<Entry>,
}

fn safe_path(p: &str) -> bool {
    if p.is_empty() || p.starts_with('/') || p.contains('\\') {
        return false;
    }
    for seg in p.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return false;
        }
        if !seg
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        {
            return false;
        }
        if seg.ends_with('.') {
            return false;
        }
        let stem = seg.split('.').next().unwrap_or(seg);
        const RESERVED: [&str; 22] = [
            "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
            "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
        ];
        if RESERVED.contains(&stem) {
            return false;
        }
    }
    true
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest> {
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some("alexandria-manifest 1") {
            return Err(Error::BadHeader);
        }
        let rel = lines.next().ok_or(Error::BadHeader)?.trim();
        let mut rp = rel.split_whitespace();
        if rp.next() != Some("release") {
            return Err(Error::BadLine(rel.into()));
        }
        let release = rp
            .next()
            .ok_or_else(|| Error::BadLine(rel.into()))?
            .to_string();
        let date = rp
            .next()
            .ok_or_else(|| Error::BadLine(rel.into()))?
            .to_string();
        if rp.next().is_some() {
            return Err(Error::BadLine(rel.into()));
        }
        let cl = lines.next().ok_or(Error::BadHeader)?.trim();
        let chunk = cl
            .strip_prefix("chunk ")
            .ok_or_else(|| Error::BadLine(cl.into()))?
            .parse::<u64>()
            .map_err(|_| Error::BadNumber(cl.into()))?;
        if chunk == 0 || chunk > MAX_CHUNK || !chunk.is_power_of_two() {
            return Err(Error::BadNumber(cl.into()));
        }

        let mut entries = Vec::new();
        let mut seen = HashSet::new();
        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() != 3 {
                return Err(Error::BadLine(line.into()));
            }
            if parts[0].len() != 64 {
                return Err(Error::BadHex(parts[0].into()));
            }
            let raw = unhex(parts[0])?;
            let mut h = [0u8; 32];
            h.copy_from_slice(&raw);
            let size = parts[1]
                .parse::<u64>()
                .map_err(|_| Error::BadNumber(parts[1].into()))?;
            let path = parts[2].to_string();
            if !safe_path(&path) {
                return Err(Error::BadPath(path));
            }
            if !seen.insert(path.clone()) {
                return Err(Error::DuplicatePath(path));
            }
            entries.push(Entry {
                hashes_sha256: h,
                size,
                path,
            });
        }
        Ok(Manifest {
            release,
            date,
            chunk,
            entries,
        })
    }

    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str("alexandria-manifest 1\n");
        s.push_str(&format!("release {} {}\n", self.release, self.date));
        s.push_str(&format!("chunk {}\n", self.chunk));
        for e in &self.entries {
            s.push_str(&format!(
                "{} {} {}\n",
                hex(&e.hashes_sha256),
                e.size,
                e.path
            ));
        }
        s
    }
}

pub fn load_signing_key(seed_hex: &str) -> Result<SigningKey> {
    let bytes = unhex(seed_hex.trim())?;
    if bytes.len() != 32 {
        return Err(Error::BadKey("seed must be 32 bytes of hex".into()));
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&bytes);
    Ok(SigningKey::from_bytes(&seed))
}

pub fn parse_verifying_key(hex_key: &str) -> Result<VerifyingKey> {
    let bytes = unhex(hex_key.trim())?;
    if bytes.len() != 32 {
        return Err(Error::BadKey("public key must be 32 bytes of hex".into()));
    }
    let mut k = [0u8; 32];
    k.copy_from_slice(&bytes);
    VerifyingKey::from_bytes(&k).map_err(|_| Error::BadKey(hex_key.into()))
}

pub fn sign_bytes(key: &SigningKey, raw: &[u8]) -> [u8; 64] {
    let mut m = Vec::from(DOMAIN);
    m.extend_from_slice(raw);
    key.sign(&m).to_bytes()
}

pub fn verify_bytes(pubkey: &VerifyingKey, raw: &[u8], sig: &[u8; 64]) -> Result<()> {
    let mut m = Vec::from(DOMAIN);
    m.extend_from_slice(raw);
    pubkey
        .verify_strict(&m, &Signature::from_bytes(sig))
        .map_err(|_| Error::BadSignature)
}

pub const KEYS_DOMAIN: &[u8] = b"alexandria-keys-v1\0";
const MAX_KEYS: usize = 16;

pub fn sign_keys(root: &SigningKey, raw: &[u8]) -> [u8; 64] {
    let mut m = Vec::from(KEYS_DOMAIN);
    m.extend_from_slice(raw);
    root.sign(&m).to_bytes()
}

pub fn verify_keys(root: &VerifyingKey, raw: &[u8], sig: &[u8; 64]) -> Result<Vec<VerifyingKey>> {
    let mut m = Vec::from(KEYS_DOMAIN);
    m.extend_from_slice(raw);
    root.verify_strict(&m, &Signature::from_bytes(sig))
        .map_err(|_| Error::BadSignature)?;
    let text = std::str::from_utf8(raw).map_err(|_| Error::BadHeader)?;
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("alexandria-keys 1") {
        return Err(Error::BadHeader);
    }
    let mut keys = Vec::new();
    for line in lines.map(str::trim).filter(|l| !l.is_empty()) {
        if line.len() != 64 || keys.len() == MAX_KEYS {
            return Err(Error::BadLine(line.into()));
        }
        keys.push(parse_verifying_key(line)?);
    }
    Ok(keys)
}

pub struct Signed {
    pub raw: Vec<u8>,
    pub sig: [u8; 64],
    pub manifest: Manifest,
}

fn read_capped(path: &Path, max: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let (file, meta) = open_regular_nofollow(path)?;
    if meta.len() > max {
        return Err(Error::NotAFile(path.display().to_string()));
    }
    let mut raw = Vec::new();
    file.take(max + 1).read_to_end(&mut raw)?;
    if raw.len() as u64 > max {
        return Err(Error::NotAFile(path.display().to_string()));
    }
    Ok(raw)
}

fn read_sig(path: &Path) -> Result<[u8; 64]> {
    let raw = read_capped(path, 64)?;
    if raw.len() != 64 {
        return Err(Error::BadSignature);
    }
    let mut sig = [0u8; 64];
    sig.copy_from_slice(&raw);
    Ok(sig)
}

pub fn open_signed(dir: &Path, root: &VerifyingKey) -> Result<Signed> {
    let raw = read_capped(&dir.join("MANIFEST"), 1 << 20)?;
    let sig = read_sig(&dir.join("MANIFEST.sig"))?;
    let mut trusted = vec![*root];
    if fs::symlink_metadata(dir.join("KEYS")).is_ok() {
        let keys = read_capped(&dir.join("KEYS"), 4096)?;
        let ksig = read_sig(&dir.join("KEYS.sig"))?;
        trusted.extend(verify_keys(root, &keys, &ksig)?);
    }
    if !trusted.iter().any(|k| verify_bytes(k, &raw, &sig).is_ok()) {
        return Err(Error::BadSignature);
    }
    let manifest = Manifest::parse(&String::from_utf8_lossy(&raw))?;
    Ok(Signed { raw, sig, manifest })
}

pub struct VerifiedFile {
    path: PathBuf,
    file: File,
    hashes: Vec<[u8; 32]>,
    chunk: u64,
    size: u64,
}

fn open_regular_nofollow(path: &Path) -> Result<(File, std::fs::Metadata)> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.custom_flags(0x00200000);
    }
    let file = opts.open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(Error::NotAFile(path.display().to_string()));
    }
    Ok((file, meta))
}

impl VerifiedFile {
    pub fn open_in(root: &Path, entry: &Entry, chunk: u64) -> Result<VerifiedFile> {
        let root = fs::canonicalize(root)?;
        let joined = root.join(&entry.path);
        let real = fs::canonicalize(&joined)?;
        if !real.starts_with(&root) {
            return Err(Error::BadPath(entry.path.clone()));
        }
        VerifiedFile::open(&real, entry, chunk)
    }

    pub fn open(path: &Path, entry: &Entry, chunk: u64) -> Result<VerifiedFile> {
        let (file, meta) = open_regular_nofollow(path)?;
        if meta.len() != entry.size {
            return Err(Error::SizeMismatch {
                path: path.display().to_string(),
                expected: entry.size,
                actual: meta.len(),
            });
        }
        let hashes_path = PathBuf::from(format!("{}.hashes", path.display()));
        let (hashes_file, hmeta) =
            open_regular_nofollow(&hashes_path).map_err(|_| Error::HashesMismatch {
                path: hashes_path.display().to_string(),
            })?;
        let n = chunk_count(entry.size, chunk) as usize;
        let want = n.checked_mul(32).ok_or(Error::HashesMismatch {
            path: hashes_path.display().to_string(),
        })?;
        if hmeta.len() != want as u64 {
            return Err(Error::HashesMismatch {
                path: hashes_path.display().to_string(),
            });
        }
        let mut raw = Vec::with_capacity(want);
        hashes_file
            .take(want as u64)
            .read_to_end(&mut raw)
            .map_err(|_| Error::HashesMismatch {
                path: hashes_path.display().to_string(),
            })?;
        if raw.len() != want {
            return Err(Error::HashesMismatch {
                path: hashes_path.display().to_string(),
            });
        }
        if Sha256::digest(&raw).as_slice() != entry.hashes_sha256 {
            return Err(Error::HashesMismatch {
                path: hashes_path.display().to_string(),
            });
        }
        let mut hashes = Vec::with_capacity(n);
        for c in raw.chunks_exact(32) {
            let mut h = [0u8; 32];
            h.copy_from_slice(c);
            hashes.push(h);
        }
        Ok(VerifiedFile {
            path: path.to_path_buf(),
            file,
            hashes,
            chunk,
            size: entry.size,
        })
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    fn read_chunk(&mut self, index: u64) -> Result<Vec<u8>> {
        let start = index * self.chunk;
        let len = std::cmp::min(self.chunk, self.size - start);
        let mut buf = vec![0u8; len as usize];
        self.file.seek(SeekFrom::Start(start))?;
        self.file.read_exact(&mut buf)?;
        if Sha256::digest(&buf).as_slice() != self.hashes[index as usize] {
            return Err(Error::ChunkMismatch { index });
        }
        Ok(buf)
    }

    pub fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let end = offset.checked_add(len as u64).ok_or(Error::OutOfBounds)?;
        if end > self.size {
            return Err(Error::OutOfBounds);
        }
        let mut out = Vec::with_capacity(len);
        let mut pos = offset;
        while pos < end {
            let ci = pos / self.chunk;
            let buf = self.read_chunk(ci)?;
            let from = (pos - ci * self.chunk) as usize;
            let take = std::cmp::min(buf.len() - from, (end - pos) as usize);
            out.extend_from_slice(&buf[from..from + take]);
            pos += take as u64;
        }
        Ok(out)
    }

    pub fn verify_all(&mut self) -> Result<()> {
        for i in 0..self.hashes.len() as u64 {
            self.read_chunk(i)?;
        }
        Ok(())
    }

    pub fn matches(&self, path: &Path) -> bool {
        let Ok((mut f, meta)) = open_regular_nofollow(path) else {
            return false;
        };
        if meta.len() != self.size {
            return false;
        }
        let mut buf = vec![0u8; self.chunk as usize];
        for (i, want) in self.hashes.iter().enumerate() {
            let len = std::cmp::min(self.chunk, self.size - i as u64 * self.chunk) as usize;
            if f.read_exact(&mut buf[..len]).is_err() || Sha256::digest(&buf[..len]).as_slice() != want {
                return false;
            }
        }
        true
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Some(h) = std::env::var_os("HOME") {
            return Path::new(&h).join("Library/Application Support/Alexandria");
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(h) = std::env::var_os("LOCALAPPDATA") {
            return Path::new(&h).join("Alexandria");
        }
    }
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
        return Path::new(&x).join("alexandria");
    }
    if let Some(h) = std::env::var_os("HOME") {
        return Path::new(&h).join(".config/alexandria");
    }
    std::env::temp_dir().join("alexandria-config")
}

pub fn cache_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Some(h) = std::env::var_os("HOME") {
            return Path::new(&h).join("Library/Caches/Alexandria");
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(h) = std::env::var_os("LOCALAPPDATA") {
            return Path::new(&h).join("Alexandria").join("cache");
        }
    }
    if let Some(x) = std::env::var_os("XDG_CACHE_HOME") {
        return Path::new(&x).join("alexandria");
    }
    if let Some(h) = std::env::var_os("HOME") {
        return Path::new(&h).join(".cache/alexandria");
    }
    std::env::temp_dir().join("alexandria-cache")
}

pub fn cached_copy(root: &Path, entry: &Entry, chunk: u64, dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let name = format!("{}-{}.bin", entry.path.rsplit('/').next().unwrap_or("entry"), hex(&entry.hashes_sha256)[..16].to_string());
    let dest = dir.join(&name);
    let part = dir.join(format!("{name}.part"));

    let _ = fs::remove_file(&part);
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let derived = n.ends_with(".bin") || n.ends_with(".part");
            if derived && n != name {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    let mut vf = VerifiedFile::open_in(root, entry, chunk)?;
    if vf.matches(&dest) {
        return Ok(dest);
    }
    let _ = fs::remove_file(&dest);
    let mut f = {
        let mut o = fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        o.open(&part)?
    };
    let mut off = 0u64;
    while off < entry.size {
        let len = std::cmp::min(chunk, entry.size - off) as usize;
        f.write_all(&vf.read_at(off, len)?)?;
        off += len as u64;
    }
    f.sync_all()?;
    drop(f);
    fs::rename(&part, &dest)?;
    Ok(dest)
}
