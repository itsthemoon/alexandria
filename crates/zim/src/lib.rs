use std::io::Read;
use std::sync::Arc;

pub const MAGIC: u32 = 0x044D_495A;
const MAX_CLUSTER: usize = 64 << 20;
const MAX_REDIRECTS: u32 = 16;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    BadMagic(u32),
    BadVersion(u16),
    Truncated,
    BadUtf8,
    BadCluster(u8),
    Decompress(String),
    NotFound,
    RedirectLoop,
    OutOfRange,
    Source(String),
    TooLarge,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io: {e}"),
            Error::BadMagic(m) => write!(f, "not a ZIM (magic {m:#x})"),
            Error::BadVersion(v) => write!(f, "unsupported ZIM major version {v}"),
            Error::Truncated => write!(f, "truncated ZIM"),
            Error::BadUtf8 => write!(f, "invalid UTF-8 in dirent"),
            Error::BadCluster(c) => write!(f, "unsupported cluster compression {c}"),
            Error::Decompress(e) => write!(f, "decompress: {e}"),
            Error::NotFound => write!(f, "article not found"),
            Error::RedirectLoop => write!(f, "redirect loop"),
            Error::OutOfRange => write!(f, "offset out of range"),
            Error::Source(e) => write!(f, "source: {e}"),
            Error::TooLarge => write!(f, "cluster exceeds the size cap"),
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

pub trait Source {
    fn size(&self) -> u64;
    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>>;
}

impl Source for alexandria_core::VerifiedFile {
    fn size(&self) -> u64 {
        alexandria_core::VerifiedFile::size(self)
    }

    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        alexandria_core::VerifiedFile::read_at(self, offset, len)
            .map_err(|e| Error::Source(e.to_string()))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Header {
    pub major: u16,
    pub minor: u16,
    pub article_count: u32,
    pub cluster_count: u32,
    pub url_ptr_pos: u64,
    pub title_ptr_pos: u64,
    pub cluster_ptr_pos: u64,
    pub mime_list_pos: u64,
    pub main_page: u32,
    pub checksum_pos: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub mime: u16,
    pub namespace: char,
    pub revision: u32,
    pub cluster: u32,
    pub blob: u32,
    pub url: String,
    pub title: String,
    pub redirect: Option<u32>,
}

impl Entry {
    pub fn is_redirect(&self) -> bool {
        self.redirect.is_some()
    }
}

pub struct Zim<S: Source> {
    src: S,
    pub header: Header,
    mimes: Vec<String>,
    cache: Option<(u32, Arc<Vec<u8>>)>,
}

fn u16le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
fn u64le(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

impl<S: Source> Zim<S> {
    pub fn open(mut src: S) -> Result<Zim<S>> {
        let size = src.size();
        if size < 80 {
            return Err(Error::Truncated);
        }
        let h = src.read_at(0, 80)?;
        let magic = u32le(&h, 0);
        if magic != MAGIC {
            return Err(Error::BadMagic(magic));
        }
        let major = u16le(&h, 4);
        if major != 5 && major != 6 {
            return Err(Error::BadVersion(major));
        }
        let header = Header {
            major,
            minor: u16le(&h, 6),
            article_count: u32le(&h, 24),
            cluster_count: u32le(&h, 28),
            url_ptr_pos: u64le(&h, 32),
            title_ptr_pos: u64le(&h, 40),
            cluster_ptr_pos: u64le(&h, 48),
            mime_list_pos: u64le(&h, 56),
            main_page: u32le(&h, 64),
            checksum_pos: u64le(&h, 72),
        };
        if header.url_ptr_pos >= size
            || header.cluster_ptr_pos >= size
            || header.mime_list_pos >= size
        {
            return Err(Error::Truncated);
        }
        let mimes = read_mimes(&mut src, header.mime_list_pos, size)?;
        Ok(Zim {
            src,
            header,
            mimes,
            cache: None,
        })
    }

    pub fn size(&self) -> u64 {
        self.src.size()
    }

    pub fn mime(&self, id: u16) -> Option<&str> {
        self.mimes.get(id as usize).map(|s| s.as_str())
    }

    fn dirent_offset(&mut self, index: u32) -> Result<u64> {
        if index >= self.header.article_count {
            return Err(Error::OutOfRange);
        }
        let at = self.header.url_ptr_pos + index as u64 * 8;
        let b = self.src.read_at(at, 8)?;
        Ok(u64le(&b, 0))
    }

    pub fn entry(&mut self, index: u32) -> Result<Entry> {
        let off = self.dirent_offset(index)?;
        let size = self.src.size();
        if off >= size {
            return Err(Error::OutOfRange);
        }
        let want = std::cmp::min(4096, (size - off) as usize);
        let buf = self.src.read_at(off, want)?;
        parse_dirent(&buf)
    }

    pub fn scan_entries(&mut self) -> Result<Vec<Entry>> {
        self.scan_entries_up_to(8_000_000)
    }

    pub fn scan_entries_up_to(&mut self, cap: usize) -> Result<Vec<Entry>> {
        let n = self.header.article_count as usize;
        if n > cap {
            return Err(Error::OutOfRange);
        }
        let first = self.src.read_at(self.header.url_ptr_pos, 8)?;
        let start = u64le(&first, 0);
        let size = self.src.size();
        if start >= size {
            return Err(Error::OutOfRange);
        }
        let mut out = Vec::new();
        let mut buf: Vec<u8> = Vec::new();
        let mut pos = 0usize;
        let mut file_off = start;
        while out.len() < n {
            if buf.len().saturating_sub(pos) < 8192 && file_off < size {
                let want = std::cmp::min(1 << 20, (size - file_off) as usize);
                if want == 0 {
                    return Err(Error::Truncated);
                }
                let block = self.src.read_at(file_off, want)?;
                if block.is_empty() {
                    return Err(Error::Truncated);
                }
                file_off += block.len() as u64;
                if pos > 0 {
                    buf.drain(..pos);
                    pos = 0;
                }
                buf.extend_from_slice(&block);
            }
            let (e, used) = match parse_dirent_used(&buf[pos..]) {
                Ok(e) => e,
                Err(Error::Truncated) => {
                    if buf.len().saturating_sub(pos) > 64 * 1024 {
                        return Err(Error::Truncated);
                    }
                    let want = std::cmp::min(1 << 20, (size - file_off) as usize);
                    if want == 0 {
                        return Err(Error::Truncated);
                    }
                    let block = self.src.read_at(file_off, want)?;
                    file_off += block.len() as u64;
                    buf.extend_from_slice(&block);
                    continue;
                }
                Err(err) => return Err(err),
            };
            if pos + used > buf.len() {
                if buf.len().saturating_sub(pos) > 64 * 1024 {
                    return Err(Error::Truncated);
                }
                let want = std::cmp::min(1 << 20, (size - file_off) as usize);
                if want == 0 {
                    return Err(Error::Truncated);
                }
                let block = self.src.read_at(file_off, want)?;
                file_off += block.len() as u64;
                buf.extend_from_slice(&block);
                continue;
            }
            pos += used;
            out.push(e);
        }
        Ok(out)
    }

    pub fn find_index(&mut self, namespace: char, url: &str) -> Result<u32> {
        let mut lo = 0u32;
        let mut hi = self.header.article_count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let e = self.entry(mid)?;
            let key = (e.namespace, e.url.as_str());
            let want = (namespace, url);
            if key < want {
                lo = mid + 1;
            } else if key > want {
                hi = mid;
            } else {
                return Ok(mid);
            }
        }
        Err(Error::NotFound)
    }

    pub fn find(&mut self, namespace: char, url: &str) -> Result<Entry> {
        let i = self.find_index(namespace, url)?;
        self.entry(i)
    }

    pub fn main_page(&mut self) -> Result<Entry> {
        if self.header.main_page == u32::MAX {
            return Err(Error::NotFound);
        }
        self.entry(self.header.main_page)
    }

    pub fn resolve(&mut self, index: u32) -> Result<Entry> {
        let mut e = self.entry(index)?;
        let mut hops = 0;
        while let Some(next) = e.redirect {
            hops += 1;
            if hops > MAX_REDIRECTS {
                return Err(Error::RedirectLoop);
            }
            e = self.entry(next)?;
        }
        Ok(e)
    }

    pub fn read_article(&mut self, index: u32) -> Result<Vec<u8>> {
        let e = self.resolve(index)?;
        self.read_blob(e.cluster, e.blob)
    }

    pub fn read_entry_blob(&mut self, e: &Entry) -> Result<Vec<u8>> {
        self.read_blob(e.cluster, e.blob)
    }

    fn cluster_bounds(&mut self, cluster: u32) -> Result<(u64, u64)> {
        if cluster >= self.header.cluster_count {
            return Err(Error::OutOfRange);
        }
        let at = self.header.cluster_ptr_pos + cluster as u64 * 8;
        let b = self.src.read_at(at, 8)?;
        let start = u64le(&b, 0);
        let end = if cluster + 1 < self.header.cluster_count {
            let b = self.src.read_at(at + 8, 8)?;
            u64le(&b, 0)
        } else if self.header.checksum_pos > start {
            self.header.checksum_pos
        } else {
            self.src.size()
        };
        if end <= start || end > self.src.size() {
            return Err(Error::OutOfRange);
        }
        Ok((start, end))
    }

    fn load_cluster(&mut self, cluster: u32) -> Result<Arc<Vec<u8>>> {
        if let Some((idx, data)) = &self.cache {
            if *idx == cluster {
                return Ok(Arc::clone(data));
            }
        }
        let (start, end) = self.cluster_bounds(cluster)?;
        let span = end - start;
        if span > (MAX_CLUSTER as u64) + 1 {
            return Err(Error::TooLarge);
        }
        let raw = self.src.read_at(start, span as usize)?;
        if raw.is_empty() {
            return Err(Error::Truncated);
        }
        if raw[0] & 0x10 != 0 {
            return Err(Error::BadCluster(raw[0]));
        }
        let kind = raw[0] & 0x0F;
        let plain = match kind {
            0 | 1 => {
                if raw.len() - 1 > MAX_CLUSTER {
                    return Err(Error::TooLarge);
                }
                raw[1..].to_vec()
            }
            5 => {
                let mut dec = ruzstd::decoding::StreamingDecoder::new(&raw[1..])
                    .map_err(|e| Error::Decompress(format!("{e:?}")))?;
                let mut out = Vec::new();
                let mut take = dec.by_ref().take(MAX_CLUSTER as u64 + 1);
                take.read_to_end(&mut out)?;
                if out.len() > MAX_CLUSTER {
                    return Err(Error::TooLarge);
                }
                out
            }
            other => return Err(Error::BadCluster(other)),
        };
        if plain.len() < 4 {
            return Err(Error::Truncated);
        }
        let arc = Arc::new(plain);
        self.cache = Some((cluster, Arc::clone(&arc)));
        Ok(arc)
    }

    pub fn read_blob(&mut self, cluster: u32, blob: u32) -> Result<Vec<u8>> {
        let plain = self.load_cluster(cluster)?;
        let first = u32le(&plain, 0) as usize;
        if first < 4 || first > plain.len() || first % 4 != 0 {
            return Err(Error::Truncated);
        }
        let blobs = first / 4 - 1;
        if blob as usize >= blobs {
            return Err(Error::OutOfRange);
        }
        let table = blob as usize * 4;
        if table + 8 > first {
            return Err(Error::Truncated);
        }
        let s = u32le(&plain, table) as usize;
        let e = u32le(&plain, table + 4) as usize;
        if s < first || e < s || e > plain.len() {
            return Err(Error::Truncated);
        }
        Ok(plain[s..e].to_vec())
    }
}

fn read_mimes<S: Source>(src: &mut S, at: u64, size: u64) -> Result<Vec<String>> {
    if at >= size {
        return Err(Error::Truncated);
    }
    let want = std::cmp::min(64 * 1024, (size - at) as usize);
    let buf = src.read_at(at, want)?;
    let mut out = Vec::new();
    let mut cur = Vec::new();
    for &b in &buf {
        if b == 0 {
            if cur.is_empty() {
                return Ok(out);
            }
            out.push(String::from_utf8(cur.clone()).map_err(|_| Error::BadUtf8)?);
            cur.clear();
        } else {
            cur.push(b);
        }
    }
    Err(Error::Truncated)
}

fn parse_dirent(buf: &[u8]) -> Result<Entry> {
    parse_dirent_used(buf).map(|(e, _)| e)
}

fn parse_dirent_used(buf: &[u8]) -> Result<(Entry, usize)> {
    if buf.len() < 12 {
        return Err(Error::Truncated);
    }
    let mime = u16le(buf, 0);
    let param_len = buf[2] as usize;
    let namespace = buf[3] as char;
    let revision = u32le(buf, 4);
    let (redirect, cluster, blob, mut p) = if mime == 0xFFFF {
        (Some(u32le(buf, 8)), 0, 0, 12usize)
    } else {
        if buf.len() < 16 {
            return Err(Error::Truncated);
        }
        (None, u32le(buf, 8), u32le(buf, 12), 16usize)
    };
    let url_end = buf[p..]
        .iter()
        .position(|&b| b == 0)
        .ok_or(Error::Truncated)?
        + p;
    let url = String::from_utf8(buf[p..url_end].to_vec()).map_err(|_| Error::BadUtf8)?;
    p = url_end + 1;
    let title_end = buf[p..]
        .iter()
        .position(|&b| b == 0)
        .ok_or(Error::Truncated)?
        + p;
    let title = String::from_utf8(buf[p..title_end].to_vec()).map_err(|_| Error::BadUtf8)?;
    let title = if title.is_empty() { url.clone() } else { title };
    p = title_end + 1;
    if p + param_len > buf.len() {
        return Err(Error::Truncated);
    }
    Ok((
        Entry {
            mime,
            namespace,
            revision,
            cluster,
            blob,
            url,
            title,
            redirect,
        },
        p + param_len,
    ))
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub index: u32,
    pub title: String,
    pub url: String,
    pub score: u32,
}

fn terms(q: &str) -> Vec<String> {
    q.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 3)
        .map(|t| t.to_string())
        .collect()
}

pub fn search<S: Source>(z: &mut Zim<S>, query: &str, limit: usize) -> Result<Vec<Hit>> {
    let terms = terms(query);
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let entries = z.scan_entries()?;
    let mut hits = Vec::new();
    let scan = std::cmp::min(entries.len(), 60_000);
    for (i, e) in entries.into_iter().take(scan).enumerate() {
        if e.mime == 0xFFFF || e.mime >= 0xFFFD {
            continue;
        }
        let title = e.title.to_lowercase();
        let url = e.url.to_lowercase();
        let mut score = 0u32;
        for t in &terms {
            if title.contains(t) {
                score += 3;
            }
            if url.contains(t) {
                score += 1;
            }
        }
        if score > 0 {
            hits.push(Hit {
                index: i as u32,
                title: e.title.clone(),
                url: e.url.clone(),
                score,
            });
        }
    }
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(a.title.len().cmp(&b.title.len()))
    });
    hits.truncate(limit);
    Ok(hits)
}

fn find_from(h: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    h.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

pub fn html_to_text(html: &str) -> String {
    let b = html.as_bytes();
    let mut out = String::with_capacity(html.len() / 2);
    let mut i = 0usize;
    let mut skip: Option<(String, u32)> = None;

    while i < b.len() {
        if b[i] == b'<' {
            if b[i..].starts_with(b"<!--") {
                match find_from(b, i, b"-->") {
                    Some(e) => {
                        i = e + 3;
                        continue;
                    }
                    None => break,
                }
            }
            let Some(end) = tag_end(b, i) else { break };
            let raw = &html[i + 1..end];
            let closing = raw.starts_with('/');
            let self_closing = raw.trim_end().ends_with('/');
            let name = tag_name(raw);

            if let Some((skip_name, depth)) = skip.as_mut() {
                if name.eq_ignore_ascii_case(skip_name) && !self_closing {
                    if closing {
                        *depth -= 1;
                        if *depth == 0 {
                            skip = None;
                        }
                    } else {
                        *depth += 1;
                    }
                }
                i = end + 1;
                continue;
            }

            if !closing && should_skip(&name, raw) {
                if !self_closing {
                    skip = Some((name.to_string(), 1));
                }
                i = end + 1;
                continue;
            }

            if closing {
                if is_block(&name) {
                    line_break(&mut out, 1);
                }
            } else if name == "br" {
                out.push('\n');
            } else if name == "li" {
                line_break(&mut out, 1);
                out.push_str("• ");
            } else if name.starts_with('h') && name.len() == 2 {
                line_break(&mut out, 2);
            } else if is_block(&name) {
                line_break(&mut out, 1);
            }
            i = end + 1;
            continue;
        }

        let start = i;
        while i < b.len() && b[i] != b'<' {
            i += 1;
        }
        if skip.is_some() {
            continue;
        }
        let text = decode_entities(&html[start..i]);
        let mut last_space = false;
        for ch in text.chars() {
            if ch.is_whitespace() {
                if !last_space {
                    out.push(if ch == '\n' { '\n' } else { ' ' });
                    last_space = true;
                }
            } else {
                out.push(ch);
                last_space = false;
            }
        }
    }

    tidy_lines(&out)
}

fn tag_end(b: &[u8], start: usize) -> Option<usize> {
    let mut quote: Option<u8> = None;
    let mut i = start + 1;
    while i < b.len() {
        let c = b[i];
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => {
                if c == b'"' || c == b'\'' {
                    quote = Some(c);
                } else if c == b'>' {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

fn tag_name(raw: &str) -> String {
    let mut s = raw.trim_start_matches('/').trim_start();
    let end = s
        .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .unwrap_or(s.len());
    s = &s[..end];
    s.to_ascii_lowercase()
}

fn should_skip(name: &str, raw: &str) -> bool {
    if matches!(
        name,
        "script" | "style" | "noscript" | "svg" | "nav" | "head" | "footer" | "template"
    ) {
        return true;
    }
    let compact: String = raw
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    compact.contains("display:none")
        || compact.contains("visibility:hidden")
        || compact.contains("mw-footer")
        || compact.contains("printfooter")
        || compact.contains("catlinks")
}

fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "section"
            | "article"
            | "header"
            | "aside"
            | "main"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "li"
            | "table"
            | "tr"
            | "td"
            | "th"
            | "blockquote"
            | "pre"
            | "dl"
            | "dt"
            | "dd"
            | "figure"
            | "figcaption"
            | "hr"
    )
}

fn line_break(out: &mut String, n: usize) {
    let existing = out.chars().rev().take_while(|&c| c == '\n').count();
    for _ in existing..n {
        out.push('\n');
    }
}

fn tidy_lines(s: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut blank = 0;
    for line in s.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 || lines.is_empty() {
                continue;
            }
            lines.push(String::new());
        } else {
            blank = 0;
            if line
                .trim_start()
                .starts_with("This article is issued from Wikipedia")
            {
                continue;
            }
            lines.push(line.to_string());
        }
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n").trim().to_string()
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while let Some(found) = s[i..].find('&') {
        let amp = i + found;
        out.push_str(&s[i..amp]);
        let rest = &s[amp + 1..];
        let semi = rest.find(';').filter(|&d| d <= 10);
        let decoded = semi.and_then(|d| {
            let ent = &rest[..d];
            let c = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" | "#160" => Some(' '),
                "eacute" => Some('é'),
                "egrave" => Some('è'),
                "agrave" => Some('à'),
                "ccedil" => Some('ç'),
                "ocirc" => Some('ô'),
                "icirc" => Some('î'),
                "ecirc" => Some('ê'),
                "acirc" => Some('â'),
                "uuml" => Some('ü'),
                "ouml" => Some('ö'),
                "auml" => Some('ä'),
                "szlig" => Some('ß'),
                "ntilde" => Some('ñ'),
                "rsquo" => Some('’'),
                "lsquo" => Some('‘'),
                "ldquo" => Some('“'),
                "rdquo" => Some('”'),
                "ndash" => Some('–'),
                "mdash" => Some('—'),
                "hellip" => Some('…'),
                _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                    u32::from_str_radix(&ent[2..], 16)
                        .ok()
                        .and_then(char::from_u32)
                }
                _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            };
            c.map(|c| (c, d))
        });
        if let Some((c, d)) = decoded {
            out.push(c);
            i = amp + 1 + d + 1;
        } else {
            out.push('&');
            i = amp + 1;
        }
    }
    out.push_str(&s[i..]);
    out
}
pub fn article_html(html: &str) -> String {
    let fragment = content_inner(html)
        .or_else(|| element_inner(html, "body"))
        .unwrap_or(html);
    let cleaned = scrub_fragment(fragment);
    format!("{ARTICLE_HEAD}\n<body>{cleaned}</body></html>")
}

const ARTICLE_HEAD: &str = r#"<!doctype html><html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data:;">
<style>
:root { color-scheme: dark; }
body { margin: 0 auto; max-width: 40em; padding: 0 28px 48px; background: #0e0f11; color: #d9dbdf;
       font: 17px/1.65 Georgia, 'Times New Roman', serif; }
h1, h2, h3, h4, h5, h6 { color: #f3f4f6; line-height: 1.2; margin: 1.25em 0 .45em; }
h1 { font-size: 2em; } h2 { font-size: 1.55em; border-bottom: 1px solid #2c2e33; padding-bottom: .2em; }
h3 { font-size: 1.25em; } h4 { font-size: 1.08em; }
p { margin: .7em 0; }
a { color: inherit; text-decoration: none; }
a.al { color: #c7cad0; cursor: pointer; border-bottom: 1px solid rgba(199,202,208,.35); }
a.al:hover { color: #ffffff; border-bottom-color: #ffffff; }
ul, ol { margin: .6em 0 .9em 1.4em; padding: 0; }
li { margin: .25em 0; }
blockquote { margin: 1em 0; padding: .2em 0 .2em 1em; border-left: 3px solid #4a4d53; color: #aeb1b7; }
table { border-collapse: collapse; margin: .8em 0; max-width: 100%; font-size: .9em; }
th, td { border: 1px solid #2c2e33; padding: .35em .55em; vertical-align: top; text-align: left; }
th { background: #18191c; }
img { max-width: 100%; height: auto; }
code, pre { font: 13px/1.45 ui-monospace, Menlo, monospace; background: #18191c; }
pre { padding: 10px; overflow-x: auto; white-space: pre-wrap; }
sup, sub { font-size: .7em; }
.mw-editsection, .navbox, .vertical-navbox, .catlinks, .printfooter, .mw-footer,
.mw-jump-link, .mw-indicators, #toc, .toc, .mw-empty-elt { display: none !important; }
.infobox, .thumb, .thumbinner, figure { max-width: 100%; }
.infobox { float: right; margin: 0 0 1em 1.4em; max-width: 320px; }
table [style*="background"], table[style*="background"], caption { background: #18191c !important; color: #e8e9ec !important; }
@media (max-width: 720px) { .infobox { float: none; margin: 1em 0; max-width: 100%; } }
</style></head>"#;

fn content_inner(html: &str) -> Option<&str> {
    let needle = "id=\"mw-content-text\"";
    let at = html.find(needle)?;
    let open = html[..at].rfind('<')?;
    let end = tag_end(html.as_bytes(), open)?;
    let name = tag_name(&html[open + 1..end]);
    if name.is_empty() {
        return None;
    }
    let close = matching_close(html, end + 1, &name)?;
    Some(&html[end + 1..close])
}

fn element_inner<'a>(html: &'a str, name: &str) -> Option<&'a str> {
    let b = html.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        let Some(open) = html[i..].find('<').map(|p| i + p) else {
            break;
        };
        if html[open..].starts_with("<!--") {
            i = open + 4;
            continue;
        }
        let Some(end) = tag_end(b, open) else { break };
        let raw = &html[open + 1..end];
        if !raw.starts_with('/') && tag_name(raw).eq_ignore_ascii_case(name) {
            let close = matching_close(html, end + 1, name)?;
            return Some(&html[end + 1..close]);
        }
        i = end + 1;
    }
    None
}

fn matching_close(html: &str, start: usize, name: &str) -> Option<usize> {
    let b = html.as_bytes();
    let mut i = start;
    let mut depth = 1usize;
    while i < b.len() {
        let Some(open) = html[i..].find('<').map(|p| i + p) else {
            return None;
        };
        if html[open..].starts_with("<!--") {
            i = open + 4;
            continue;
        }
        let end = tag_end(b, open)?;
        let raw = &html[open + 1..end];
        let closing = raw.starts_with('/');
        let self_closing = raw.trim_end().ends_with('/');
        let tag = tag_name(raw);
        if tag.eq_ignore_ascii_case(name) {
            if closing {
                depth -= 1;
                if depth == 0 {
                    return Some(open);
                }
            } else if !self_closing {
                depth += 1;
            }
        }
        i = end + 1;
    }
    None
}

fn scrub_fragment(html: &str) -> String {
    let b = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;
    while i < b.len() {
        if b[i] != b'<' {
            let start = i;
            while i < b.len() && b[i] != b'<' {
                i += 1;
            }
            out.push_str(&html[start..i]);
            continue;
        }
        if html[i..].starts_with("<!--") {
            match html[i..].find("-->") {
                Some(p) => {
                    i += p + 3;
                    continue;
                }
                None => break,
            }
        }
        let Some(end) = tag_end(b, i) else { break };
        let raw = &html[i + 1..end];
        let closing = raw.starts_with('/');
        let self_closing = raw.trim_end().ends_with('/');
        let name = tag_name(raw);
        if name.is_empty() {
            i = end + 1;
            continue;
        }

        if matches!(
            name.as_str(),
            "script"
                | "style"
                | "noscript"
                | "template"
                | "iframe"
                | "object"
                | "embed"
                | "form"
                | "input"
                | "button"
                | "select"
                | "option"
                | "textarea"
                | "link"
                | "meta"
                | "base"
                | "svg"
                | "math"
                | "canvas"
                | "audio"
                | "video"
                | "source"
                | "track"
        ) {
            if !self_closing && !closing {
                if let Some(close) = skip_element(html, end + 1, &name) {
                    i = close;
                } else {
                    i = end + 1;
                }
            } else {
                i = end + 1;
            }
            continue;
        }
        if name == "img" {
            i = end + 1;
            continue;
        }
        if name == "a" {
            if closing {
                out.push_str("</a>");
            } else {
                let target = attrs(raw, "a")
                    .into_iter()
                    .find(|(k, _)| k == "href")
                    .and_then(|(_, v)| internal_link(&v));
                match target {
                    Some(t) => out.push_str(&format!(
                        "<a class=\"al\" href=\"#a:{}\">",
                        percent_encode(&t)
                    )),
                    None => out.push_str("<a class=\"ext\">"),
                }
            }
            i = end + 1;
            continue;
        }
        if closing {
            out.push_str(&format!("</{name}>"));
        } else if let Some(tag) = clean_tag(raw, &name, self_closing) {
            out.push_str(&tag);
        }
        i = end + 1;
    }
    out
}

fn find_ascii_case_insensitive(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= hay.len() || hay.len() - from < needle.len() {
        return None;
    }
    let last = hay.len() - needle.len();
    for i in from..=last {
        if hay[i].eq_ignore_ascii_case(&needle[0])
            && hay[i..i + needle.len()]
                .iter()
                .zip(needle)
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            return Some(i);
        }
    }
    None
}

fn skip_element(html: &str, start: usize, name: &str) -> Option<usize> {
    let close = format!("</{name}");
    let at = find_ascii_case_insensitive(html.as_bytes(), close.as_bytes(), start)?;
    let end = tag_end(html.as_bytes(), at)?;
    Some(end + 1)
}

fn attrs(raw: &str, name: &str) -> Vec<(String, String)> {
    let rest = raw.trim_start_matches('/').trim_start();
    let rest = &rest[name.len().min(rest.len())..];
    let b = rest.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let key_start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        let key = rest[key_start..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() || b[i] != b'=' {
            continue;
        }
        i += 1;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let quote = if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
            let q = b[i];
            i += 1;
            Some(q)
        } else {
            None
        };
        let val_start = i;
        if let Some(q) = quote {
            while i < b.len() && b[i] != q {
                i += 1;
            }
        } else {
            while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'/' {
                i += 1;
            }
        }
        let val = rest[val_start..i.min(rest.len())].to_string();
        if i < b.len() && quote.is_some() {
            i += 1;
        }
        out.push((key, val));
    }
    out
}

fn escape_attr(v: &str) -> String {
    v.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

fn clean_tag(raw: &str, name: &str, self_closing: bool) -> Option<String> {
    let allowed = [
        "class", "id", "style", "title", "colspan", "rowspan", "width", "height", "align",
        "valign", "scope", "dir", "lang", "alt",
    ];
    let mut out = String::from("<");
    out.push_str(name);
    for (key, val) in attrs(raw, name) {
        if allowed.contains(&key.as_str()) {
            out.push_str(&format!(" {key}=\"{}\"", escape_attr(&val)));
        }
    }
    if self_closing {
        out.push_str(" /");
    }
    out.push('>');
    Some(out)
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

pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'(' | b')' | b',' | b'!') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn internal_link(href: &str) -> Option<String> {
    let h = href.trim();
    if h.is_empty() || h.starts_with('#') || h.starts_with("//") {
        return None;
    }
    let first = h.find(|c| c == '/' || c == '?' || c == '#').unwrap_or(h.len());
    if h[..first].contains(':') {
        return None;
    }
    let mut p = h.split(['#', '?']).next().unwrap_or("");
    loop {
        if let Some(r) = p.strip_prefix("./") {
            p = r;
        } else if let Some(r) = p.strip_prefix("../") {
            p = r;
        } else {
            break;
        }
    }
    let p = p
        .strip_prefix("A/")
        .or_else(|| p.strip_prefix("C/"))
        .unwrap_or(p);
    let url = percent_decode(p);
    if url.is_empty() || url.contains('\0') {
        None
    } else {
        Some(url)
    }
}

pub fn chat_prompt(context: &str, question: &str) -> String {
    let system = if context.is_empty() {
        "You are an offline assistant. Be brief and concrete."
    } else {
        "Answer only from the excerpts. If they do not contain the answer, say you do not know."
    };
    format!(
        "<|im_start|>system\n{system}<|im_end|>\n<|im_start|>user\n{context}Question: {question}<|im_end|>\n<|im_start|>assistant\n"
    )
}
