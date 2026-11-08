use std::cell::RefCell;

use alexandria_core::VerifiedFile;

pub const HEADER: &str = "alexandria-streets 1";
pub const HEADER_INDEXED: &str = "alexandria-streets 3";
pub const HEADER_INDEXED_V4: &str = "alexandria-streets 4";

const TAIL_CAP: u64 = 64 * 1024 * 1024;
const PENDING_CAP: usize = 1 << 20;
const SCAN_CAP: u64 = 4 * 1024 * 1024;

struct Row {
    folded: String,
    name: String,
    lat: f32,
    lon: f32,
    place: Option<String>,
}

enum Backend {
    Memory(Vec<Row>),
    Disk {
        file: RefCell<VerifiedFile>,
        chunk: u64,
        size: u64,
        blocks: Vec<(String, u64)>,
        rows: usize,
    },
}

pub struct Streets {
    backend: Backend,
}

fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let c = match c {
            'à'..='å' | 'À'..='Å' => 'a',
            'è'..='ë' | 'È'..='Ë' => 'e',
            'ì'..='ï' | 'Ì'..='Ï' => 'i',
            'ò'..='ö' | 'Ò'..='Ö' => 'o',
            'ù'..='ü' | 'Ù'..='Ü' => 'u',
            'ç' => 'c',
            'Ç' => 'C',
            'ñ' => 'n',
            'Ñ' => 'N',
            'ß' => 's',
            'ý' | 'ÿ' => 'y',
            'ø' | 'Ø' => 'o',
            'æ' => 'a',
            'œ' => 'o',
            'ł' | 'Ł' => 'l',
            other => other,
        };
        for low in c.to_lowercase() {
            out.push(low);
        }
    }
    out
}

fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    let dlat = a.0 - b.0;
    let dlon = (a.1 - b.1) * (a.0.to_radians().cos()).abs().max(0.01);
    dlat * dlat + dlon * dlon
}

fn rank_of(key: &str, q: &str) -> Option<u8> {
    if key == q {
        Some(0)
    } else if key.starts_with(q) {
        Some(1)
    } else if key.contains(q) {
        Some(2)
    } else {
        None
    }
}

fn line_row(line: &str) -> Option<Row> {
    let mut parts = line.split('\t');
    let (Some(key), Some(name), Some(lat), Some(lon)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    if key.is_empty() || name.is_empty() {
        return None;
    }
    let (Ok(lat), Ok(lon)) = (lat.parse::<f32>(), lon.parse::<f32>()) else {
        return None;
    };
    let place = parts
        .next()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    Some(Row {
        folded: key.to_string(),
        name: name.to_string(),
        lat,
        lon,
        place,
    })
}

fn best(
    mut hits: Vec<(u8, f32, String, f32, f32, Option<String>)>,
    limit: usize,
) -> Vec<(String, f32, f32, Option<String>)> {
    hits.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.2.cmp(&b.2))
    });
    let mut out: Vec<(String, f32, f32, Option<String>)> = Vec::new();
    for (_, _, name, lat, lon, place) in hits {
        if out.iter().any(|o| o.0 == name) {
            continue;
        }
        out.push((name, lat, lon, place));
        if out.len() == limit {
            break;
        }
    }
    out
}

impl Streets {
    pub fn parse(bytes: &[u8]) -> Option<Streets> {
        let text = std::str::from_utf8(bytes).ok()?;
        let mut lines = text.lines();
        if lines.next()? != HEADER {
            return None;
        }
        let rows: Vec<Row> = lines.filter_map(line_row).collect();
        Some(Streets {
            backend: Backend::Memory(rows),
        })
    }

    pub fn open_verified(mut vf: VerifiedFile, chunk: u64) -> Option<Streets> {
        let size = vf.size();
        if size < 64 {
            let bytes = vf.read_at(0, size as usize).ok()?;
            return Streets::parse(&bytes);
        }
        let head = vf.read_at(0, 64).ok()?;
        let nl = head.iter().position(|&c| c == b'\n').unwrap_or(head.len());
        let head = std::str::from_utf8(&head[..nl]).ok()?.trim().to_string();
        let indexed = head.starts_with(HEADER_INDEXED) || head.starts_with(HEADER_INDEXED_V4);
        if !indexed {
            if size > TAIL_CAP {
                return None;
            }
            let bytes = vf.read_at(0, size as usize).ok()?;
            return Streets::parse(&bytes);
        }
        let mut parts = head.split_whitespace();
        parts.next()?;
        parts.next()?;
        let index_at: u64 = parts.next()?.parse().ok()?;
        let rows: usize = parts.next()?.parse().ok()?;
        if index_at > size || size - index_at > TAIL_CAP {
            return None;
        }
        let tail = vf.read_at(index_at, (size - index_at) as usize).ok()?;
        let tail = String::from_utf8_lossy(&tail);
        let mut blocks = Vec::new();
        for line in tail.lines() {
            let mut p = line.split('\t');
            let (Some(key), Some(offset)) = (p.next(), p.next()) else {
                continue;
            };
            let Ok(offset) = offset.parse::<u64>() else {
                continue;
            };
            blocks.push((key.to_string(), offset));
        }
        let sorted = blocks.windows(2).all(|w| w[0].0 <= w[1].0);
        let increasing = blocks.windows(2).all(|w| w[0].1 < w[1].1);
        let in_bounds = blocks.iter().all(|(_, o)| *o < size);
        if !sorted || !increasing || !in_bounds {
            return None;
        }
        Some(Streets {
            backend: Backend::Disk {
                file: RefCell::new(vf),
                chunk,
                size,
                blocks,
                rows,
            },
        })
    }

    pub fn len(&self) -> usize {
        match &self.backend {
            Backend::Memory(rows) => rows.len(),
            Backend::Disk { rows, .. } => *rows,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn find(
        &self,
        query: &str,
        near: Option<(f32, f32)>,
        limit: usize,
    ) -> Vec<(String, f32, f32, Option<String>)> {
        let q = fold(query.trim());
        if q.chars().count() < 2 {
            return Vec::new();
        }
        let away = |lat: f32, lon: f32| match near {
            Some(p) => distance(p, (lat, lon)),
            None => 0.0,
        };
        match &self.backend {
            Backend::Memory(rows) => {
                let hits = rows
                    .iter()
                    .filter_map(|r| {
                        let rank = rank_of(&r.folded, &q)?;
                        Some((
                            rank,
                            away(r.lat, r.lon),
                            r.name.clone(),
                            r.lat,
                            r.lon,
                            r.place.clone(),
                        ))
                    })
                    .collect();
                best(hits, limit)
            }
            Backend::Disk {
                file,
                chunk,
                size,
                blocks,
                ..
            } => self.find_on_disk(&q, &away, limit, file, *chunk, *size, blocks),
        }
    }

    fn find_on_disk(
        &self,
        q: &str,
        away: &dyn Fn(f32, f32) -> f32,
        limit: usize,
        file: &RefCell<VerifiedFile>,
        chunk: u64,
        size: u64,
        blocks: &[(String, u64)],
    ) -> Vec<(String, f32, f32, Option<String>)> {
        let start = match blocks.partition_point(|(key, _)| key.as_str() <= q) {
            0 => 0,
            n => n - 1,
        };
        let Some((_, offset)) = blocks.get(start) else {
            return Vec::new();
        };
        let mut hits: Vec<(u8, f32, String, f32, f32, Option<String>)> = Vec::new();
        let mut pending: Vec<u8> = Vec::new();
        let mut pos = *offset;
        let budget = SCAN_CAP.max(chunk);
        let mut scanned = 0u64;
        'outer: while pos < size {
            if scanned >= budget {
                break;
            }
            let want = chunk.min(size - pos) as usize;
            let bytes = match file.borrow_mut().read_at(pos, want) {
                Ok(b) => b,
                Err(_) => break,
            };
            pos += bytes.len() as u64;
            scanned += bytes.len() as u64;
            pending.extend_from_slice(&bytes);
            let mut consumed = 0usize;
            loop {
                let Some(idx) = pending[consumed..].iter().position(|&c| c == b'\n') else {
                    break;
                };
                let end = consumed + idx;
                let line = &pending[consumed..end];
                consumed = end + 1;
                let Ok(line) = std::str::from_utf8(line) else {
                    continue;
                };
                let Some(row) = line_row(line) else {
                    continue;
                };
                if row.folded.as_str() < q {
                    continue;
                }
                if !row.folded.starts_with(q) {
                    break 'outer;
                }
                let rank = if row.folded == q { 0 } else { 1 };
                hits.push((rank, away(row.lat, row.lon), row.name, row.lat, row.lon, row.place));
                if hits.len() >= 20_000 {
                    break 'outer;
                }
            }
            pending.drain(..consumed);
            if pending.len() > PENDING_CAP {
                break;
            }
        }
        best(hits, limit)
    }
}
