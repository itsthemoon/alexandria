use crate::format;
use crate::{Route, Step};
use alexandria_core::VerifiedFile;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::sync::Arc;

const CACHE_BYTES: usize = 256 << 20;
const XY_SLOTS: usize = 1 << 20;

const RESTRICTED_ENTRY_MS: u32 = 3_600_000;
const MAX_TRIP_M: f64 = 250_000.0;

const ELLIPSE_K: f64 = 3.0;
const ELLIPSE_C: f64 = 50_000.0;
const SETTLED_CAP: usize = 8_000_000;
const PROBE_LIMIT: usize = 2000;
const MAX_SNAP_CANDS: usize = 8;
const WINDOW_CELLS: usize = 2;
const SAME_POINT_M: f64 = 0.01;

struct Slot {
    key: u64,
    data: Arc<Vec<u8>>,
    referenced: bool,
}

struct Cache {
    chunk: u64,
    max_slots: usize,
    slots: Vec<Slot>,
    index: HashMap<u64, usize, FxBuild>,
    hand: usize,
}

impl Cache {
    fn new(chunk: u64) -> Cache {
        Cache {
            chunk,
            max_slots: (CACHE_BYTES as u64 / chunk).max(1) as usize,
            slots: Vec::new(),
            index: HashMap::default(),
            hand: 0,
        }
    }

    fn get(&mut self, file: &mut VerifiedFile, i: u64) -> Result<Arc<Vec<u8>>, String> {
        if let Some(&p) = self.index.get(&i) {
            self.slots[p].referenced = true;
            return Ok(self.slots[p].data.clone());
        }
        let start = i * self.chunk;
        let len = std::cmp::min(self.chunk, file.size().saturating_sub(start)) as usize;
        let arc = Arc::new(file.read_at(start, len).map_err(|e| e.to_string())?);
        if self.slots.len() < self.max_slots {
            let p = self.slots.len();
            self.slots.push(Slot {
                key: i,
                data: arc.clone(),
                referenced: true,
            });
            self.index.insert(i, p);
            return Ok(arc);
        }
        loop {
            let p = self.hand % self.max_slots;
            self.hand = p + 1;
            if self.slots[p].referenced {
                self.slots[p].referenced = false;
                continue;
            }
            self.index.remove(&self.slots[p].key);
            self.slots[p] = Slot {
                key: i,
                data: arc.clone(),
                referenced: true,
            };
            self.index.insert(i, p);
            return Ok(arc);
        }
    }
}

struct Edge {
    src: u32,
    target: u32,
    weight: u32,
    name: u32,
    cls: u8,
    flags: u8,
    geom: u32,
    geomcnt: u16,
}

pub struct Roads {
    file: VerifiedFile,
    chunk: u64,
    node_count: usize,
    edge_count: usize,
    name_count: usize,
    geom_bytes: u64,
    grid_cols: usize,
    grid_rows: usize,
    cell_size: f64,
    min_lat: f64,
    min_lon: f64,
    max_lat: f64,
    max_lon: f64,
    max_speed: f64,
    nodes_off: u64,
    edges_off: u64,
    cells_off: u64,
    geom_off: u64,
    names_off: u64,
    name_off: Vec<u32>,
    cache: Cache,
    xy_cache: Vec<u32>,
}

#[derive(Default)]
struct FxBuild;

impl BuildHasher for FxBuild {
    type Hasher = FxHasher;

    fn build_hasher(&self) -> FxHasher {
        FxHasher(0)
    }
}

struct FxHasher(u64);

impl Hasher for FxHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ b as u64).wrapping_mul(0x517cc1b727220a95);
        }
    }

    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(0x517cc1b727220a95);
    }
}

fn state_key(node: u32, edge: u32) -> u64 {
    ((node as u64) << 32) | edge as u64
}

fn state_node(key: u64) -> u32 {
    (key >> 32) as u32
}

fn micro_to_deg(v: i32) -> f64 {
    v as f64 / 1e6
}

fn hav_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    const R: f64 = 6371000.0;
    let dlat = (b.0 - a.0).to_radians();
    let dlon = (b.1 - a.1).to_radians();
    let la1 = a.0.to_radians();
    let la2 = b.0.to_radians();
    let sdlat = (dlat / 2.0).sin();
    let sdlon = (dlon / 2.0).sin();
    let h = sdlat * sdlat + la1.cos() * la2.cos() * sdlon * sdlon;
    2.0 * R * h.sqrt().asin()
}

fn flat_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    const R: f64 = 6371000.0;
    let lat = (a.0 + b.0) / 2.0;
    let dlat = (b.0 - a.0).to_radians() * R;
    let dlon = (b.1 - a.1).to_radians() * R * lat.to_radians().cos();
    (dlat * dlat + dlon * dlon).sqrt()
}

impl Roads {
    pub fn open(mut file: VerifiedFile) -> Result<Roads, String> {
        let size = file.size();
        let chunk = file.chunk();
        if size < format::HEADER_LEN as u64 {
            return Err("file too small".into());
        }
        let h = file.read_at(0, format::HEADER_LEN).map_err(|e| e.to_string())?;
        if &h[0..8] != format::MAGIC {
            return Err("not an alexandria roads file".into());
        }
        let version = u16::from_le_bytes([h[8], h[9]]);
        if version != format::VERSION {
            return Err(format!("unsupported roads version {version}"));
        }
        let node_count = format::u32_at(&h, 12) as usize;
        let edge_count = format::u32_at(&h, 16) as usize;
        let name_count = format::u32_at(&h, 20) as usize;
        let geom_bytes = format::u32_at(&h, 24) as u64;
        let cell_count = format::u32_at(&h, 28) as usize;
        let grid_cols = format::u32_at(&h, 32) as usize;
        let grid_rows = format::u32_at(&h, 36) as usize;
        let min_lat = format::i32_at(&h, 40);
        let min_lon = format::i32_at(&h, 44);
        let max_lat = format::i32_at(&h, 48);
        let max_lon = format::i32_at(&h, 52);
        let cell_size = format::u32_at(&h, 56);
        let max_speed_x10 = format::u32_at(&h, 60);
        if grid_cols == 0 || grid_rows == 0 || cell_count != grid_cols * grid_rows {
            return Err("bad grid dimensions".into());
        }
        if min_lat > max_lat || min_lon > max_lon || cell_size == 0 {
            return Err("bad bounds".into());
        }

        let nodes_off = format::HEADER_LEN as u64;
        let nodes_len = node_count as u64 * format::NODE_REC as u64;
        let edges_off = nodes_off + nodes_len;
        let edges_len = edge_count as u64 * format::EDGE_REC as u64;
        let cells_off = edges_off + edges_len;
        let cells_len = (cell_count as u64 + 1) * 4;
        let name_off_off = cells_off + cells_len;
        let name_off_len = (name_count as u64 + 1) * 4;
        let names_blob_off = name_off_off + name_off_len;
        let names_blob_len = if name_count == 0 {
            0
        } else {
            let mut nb = vec![0u8; 4];
            nb.copy_from_slice(
                &file
                    .read_at(name_off_off + name_count as u64 * 4, 4)
                    .map_err(|e| e.to_string())?,
            );
            format::u32_at(&nb, 0) as u64
        };
        let geom_off = names_blob_off + names_blob_len;
        if geom_off + geom_bytes > size {
            return Err("section offsets exceed file size".into());
        }

        let noff_raw = file.read_at(name_off_off, name_off_len as usize).map_err(|e| e.to_string())?;
        let mut name_off = Vec::with_capacity(name_count + 1);
        for i in 0..name_count + 1 {
            name_off.push(format::u32_at(&noff_raw, i * 4));
        }
        if name_count > 0 && name_off[name_count] as u64 != names_blob_len {
            return Err("name offsets disagree".into());
        }

        Ok(Roads {
            max_speed: max_speed_x10 as f64 / 10.0 / 3.6,
            file,
            chunk,
            node_count,
            edge_count,
            name_count,
            geom_bytes,
            grid_cols,
            grid_rows,
            cell_size: cell_size as f64 / 1e6,
            min_lat: micro_to_deg(min_lat),
            min_lon: micro_to_deg(min_lon),
            max_lat: micro_to_deg(max_lat),
            max_lon: micro_to_deg(max_lon),
            nodes_off,
            edges_off,
            cells_off,
            geom_off,
            names_off: names_blob_off,
            name_off,
            cache: Cache::new(chunk),
            xy_cache: vec![u32::MAX; XY_SLOTS * 3],
        })
    }

    pub fn route(&mut self, from: (f64, f64), to: (f64, f64)) -> Result<Route, String> {
        if self.node_count == 0 {
            return Err("empty graph".into());
        }
        if hav_m(from, to) > MAX_TRIP_M {
            return Err("Directions cover trips up to 250 km in a straight line for now".into());
        }
        let s = self.snap(from, "start")?;
        let t = self.snap(to, "end")?;
        if let Some(r) = self.direct(&s, &t)? {
            return Ok(r);
        }
        let (path, from, goal) = self.search(&s, &t)?;
        let to_tail = t.fwd && self.xy(goal as usize)? == self.xy(t.tail as usize)?;
        self.render(&s, &t, &path, from == s.head, to_tail)
    }

    fn read_span(&mut self, off: u64, len: usize) -> Result<Vec<u8>, String> {
        let end = off.checked_add(len as u64).ok_or("read out of range")?;
        if end > self.file.size() {
            return Err("read out of range".into());
        }
        let mut out = Vec::with_capacity(len);
        let mut pos = off;
        while pos < end {
            let ci = pos / self.chunk;
            let c = self.cache.get(&mut self.file, ci)?;
            let within = (pos % self.chunk) as usize;
            let take = std::cmp::min(c.len() - within, (end - pos) as usize);
            out.extend_from_slice(&c[within..within + take]);
            pos += take as u64;
        }
        Ok(out)
    }

    fn node(&mut self, i: usize) -> Result<(i32, i32, u32, u32), String> {
        if i >= self.node_count {
            return Err("node index out of range".into());
        }
        let off = self.nodes_off + i as u64 * format::NODE_REC as u64;
        let ci = off / self.chunk;
        let c = self.cache.get(&mut self.file, ci)?;
        let within = (off % self.chunk) as usize;
        if within + format::NODE_REC <= c.len() {
            let b = &c[within..within + format::NODE_REC];
            let first = format::u32_at(b, 8);
            let degree = format::u32_at(b, 12);
            if first.checked_add(degree).ok_or("node edge range overflow")? as usize > self.edge_count {
                return Err("node edge range exceeds edge count".into());
            }
            return Ok((format::i32_at(b, 0), format::i32_at(b, 4), first, degree));
        }
        let raw = self.read_span(off, format::NODE_REC)?;
        let first = format::u32_at(&raw, 8);
        let degree = format::u32_at(&raw, 12);
        if first.checked_add(degree).ok_or("node edge range overflow")? as usize > self.edge_count {
            return Err("node edge range exceeds edge count".into());
        }
        Ok((format::i32_at(&raw, 0), format::i32_at(&raw, 4), first, degree))
    }

    fn edge(&mut self, i: usize) -> Result<Edge, String> {
        if i >= self.edge_count {
            return Err("edge index out of range".into());
        }
        let off = self.edges_off + i as u64 * format::EDGE_REC as u64;
        let ci = off / self.chunk;
        let c = self.cache.get(&mut self.file, ci)?;
        let within = (off % self.chunk) as usize;
        if within + format::EDGE_REC <= c.len() {
            let b = &c[within..within + format::EDGE_REC];
            let src = format::u32_at(b, 0);
            let target = format::u32_at(b, 4);
            let name = format::u32_at(b, 12);
            let geom = format::u32_at(b, 18);
            if src as usize >= self.node_count || target as usize >= self.node_count {
                return Err("edge endpoint out of range".into());
            }
            if name as usize >= self.name_count {
                return Err("edge name out of range".into());
            }
            if geom as u64 > self.geom_bytes {
                return Err("edge geometry offset out of range".into());
            }
            return Ok(Edge {
                src,
                target,
                weight: format::u32_at(b, 8),
                name,
                cls: b[16],
                flags: b[17],
                geom,
                geomcnt: u16::from_le_bytes([b[22], b[23]]),
            });
        }
        let raw = self.read_span(off, format::EDGE_REC)?;
        let src = format::u32_at(&raw, 0);
        let target = format::u32_at(&raw, 4);
        let name = format::u32_at(&raw, 12);
        let geom = format::u32_at(&raw, 18);
        if src as usize >= self.node_count || target as usize >= self.node_count {
            return Err("edge endpoint out of range".into());
        }
        if name as usize >= self.name_count {
            return Err("edge name out of range".into());
        }
        if geom as u64 > self.geom_bytes {
            return Err("edge geometry offset out of range".into());
        }
        Ok(Edge {
            src,
            target,
            weight: format::u32_at(&raw, 8),
            name,
            cls: raw[16],
            flags: raw[17],
            geom,
            geomcnt: u16::from_le_bytes([raw[22], raw[23]]),
        })
    }

    fn edge_polyline(&mut self, ei: usize) -> Result<Vec<(f64, f64)>, String> {
        let e = self.edge(ei)?;
        let (slat, slon, ..) = self.node(e.src as usize)?;
        let mut pts = vec![(micro_to_deg(slat), micro_to_deg(slon))];
        let cnt = e.geomcnt as usize;
        if cnt > 0 {
            let off = self.geom_off + e.geom as u64;
            let max_bytes = ((cnt as u64) * 11).min(self.file.size().saturating_sub(off)) as usize;
            let raw = self.read_span(off, max_bytes)?;
            let mut pos = 0usize;
            let mut prev = (slat as i64, slon as i64);
            for _ in 0..cnt {
                let dl = format::read_varint(&raw, &mut pos)?;
                let dn = format::read_varint(&raw, &mut pos)?;
                prev.0 += format::zigzag_decode(dl) as i64;
                prev.1 += format::zigzag_decode(dn) as i64;
                pts.push((prev.0 as f64 / 1e6, prev.1 as f64 / 1e6));
            }
        }
        let (tlat, tlon, ..) = self.node(e.target as usize)?;
        pts.push((micro_to_deg(tlat), micro_to_deg(tlon)));
        Ok(pts)
    }

    fn scan_cells(
        &mut self,
        p: (f64, f64),
        cc: usize,
        cr: usize,
        radius: usize,
        out: &mut Vec<(f64, u32, usize, usize, f64, (f64, f64))>,
    ) -> Result<(), String> {
        let r0 = cr.saturating_sub(radius);
        let r1 = (cr + radius).min(self.grid_rows - 1);
        let c0 = cc.saturating_sub(radius);
        let c1 = (cc + radius).min(self.grid_cols - 1);
        for r in r0..=r1 {
            for c in c0..=c1 {
                let cell = r * self.grid_cols + c;
                let (a, b) = self.cell_bound(cell)?;
                for n in a..b {
                    let (_, _, first, degree) = self.node(n)?;
                    for e in first as usize..(first + degree) as usize {
                        let (d, frac, proj) = self.project(p, e)?;
                        let tgt = self.edge(e)?.target as usize;
                        out.push((d, e as u32, n, tgt, frac, proj));
                    }
                }
            }
        }
        Ok(())
    }

    fn snap(&mut self, p: (f64, f64), which: &str) -> Result<Snap, String> {
        let (cc, cr) = match self.cell_of(p.0, p.1) {
            Some(c) => c,
            None => return Err(format!("No road found near the {which}")),
        };
        let free_end = |frac: f64| if which == "start" { frac } else { 1.0 - frac };
        let mut best: Option<(f64, u32, usize, usize, f64, (f64, f64))> = None;
        for radius in 0usize..=2 {
            let r0 = cr.saturating_sub(radius);
            let r1 = (cr + radius).min(self.grid_rows - 1);
            let c0 = cc.saturating_sub(radius);
            let c1 = (cc + radius).min(self.grid_cols - 1);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    let cell = r * self.grid_cols + c;
                    let (a, b) = self.cell_bound(cell)?;
                    for n in a..b {
                        let (_, _, first, degree) = self.node(n)?;
                        for e in first as usize..(first + degree) as usize {
                            let (d, frac, proj) = self.project(p, e)?;
                            let better = match &best {
                                None => true,
                                Some((bd, _, _, _, bf, _)) => {
                                    d < *bd - SAME_POINT_M || (d <= *bd + SAME_POINT_M && free_end(frac) > free_end(*bf))
                                }
                            };
                            if better {
                                let tgt = self.edge(e)?.target as usize;
                                best = Some((d, e as u32, n, tgt, frac, proj));
                            }
                        }
                    }
                }
            }
            if radius >= 1 {
                if let Some((d, ..)) = best {
                    if d < 50.0 {
                        break;
                    }
                }
            }
        }
        let Some((d, ei, tail, head, frac, proj)) = best else {
            return Err(format!("No road found near the {which}"));
        };
        if d > 1000.0 {
            return Err(format!("No road found near the {which}"));
        }
        let forward = which == "start";
        let mut win = None;
        let mut seen = BitSet::new(self.node_count);
        let mut visited = Vec::new();
        let first = if forward { head } else { tail };
        if !self.probe(first, forward, cc, cr, &mut win, &mut seen, &mut visited)? {
            let mut cands: Vec<(f64, u32, usize, usize, f64, (f64, f64))> = Vec::new();
            self.scan_cells(p, cc, cr, 2, &mut cands)?;
            let bucket = |d: f64| (d / SAME_POINT_M).floor() as i64;
            cands.sort_by(|x, y| bucket(x.0).cmp(&bucket(y.0)).then(free_end(y.4).total_cmp(&free_end(x.4))));
            let mut probes = 0;
            for cand in cands {
                if cand.0 > 1000.0 || probes >= MAX_SNAP_CANDS {
                    break;
                }
                let at = if forward { cand.3 } else { cand.2 };
                if seen.test(at) {
                    continue;
                }
                probes += 1;
                visited.clear();
                if self.probe(at, forward, cc, cr, &mut win, &mut seen, &mut visited)? {
                    let (_, ei2, tail2, head2, frac2, point2) = cand;
                    let fwd = self.has_edge(tail2, head2)?;
                    let back = self.twin_target(head2, tail2)?;
                    return Ok(Snap {
                        tail: tail2 as u32,
                        head: head2 as u32,
                        edge: ei2,
                        frac: frac2,
                        point: point2,
                        fwd,
                        rev: back.is_some(),
                        back: back.unwrap_or(tail2) as u32,
                    });
                }
            }
        }
        let fwd = self.has_edge(tail, head)?;
        let back = self.twin_target(head, tail)?;
        Ok(Snap {
            tail: tail as u32,
            head: head as u32,
            edge: ei,
            frac,
            point: proj,
            fwd,
            rev: back.is_some(),
            back: back.unwrap_or(tail) as u32,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn probe(
        &mut self,
        start: usize,
        forward: bool,
        cc: usize,
        cr: usize,
        win: &mut Option<Window>,
        seen: &mut BitSet,
        visited: &mut Vec<u32>,
    ) -> Result<bool, String> {
        if forward {
            return self.reaches_big(start, seen, visited);
        }
        if win.is_none() {
            *win = Some(self.window(cc, cr)?);
        }
        let w = win.as_ref().unwrap();
        let mut stack = vec![start as u32];
        while let Some(u) = stack.pop() {
            let uu = u as usize;
            if seen.test(uu) {
                continue;
            }
            seen.set(uu);
            visited.push(u);
            if visited.len() >= PROBE_LIMIT || !w.inside(uu) || w.open.contains(&u) {
                for &v in visited.iter() {
                    seen.clear(v as usize);
                }
                return Ok(true);
            }
            if let Some(ps) = w.preds.get(&u) {
                for &q in ps {
                    if !seen.test(q as usize) {
                        stack.push(q);
                    }
                }
            }
        }
        Ok(false)
    }

    fn window(&mut self, cc: usize, cr: usize) -> Result<Window, String> {
        let r0 = cr.saturating_sub(WINDOW_CELLS);
        let r1 = (cr + WINDOW_CELLS).min(self.grid_rows - 1);
        let c0 = cc.saturating_sub(WINDOW_CELLS);
        let c1 = (cc + WINDOW_CELLS).min(self.grid_cols - 1);
        let mut inner = Vec::new();
        let mut ring = Vec::new();
        for r in r0..=r1 {
            for c in c0..=c1 {
                let span = self.cell_bound(r * self.grid_cols + c)?;
                let edge_cell = r.abs_diff(cr) == WINDOW_CELLS || c.abs_diff(cc) == WINDOW_CELLS;
                if edge_cell { ring.push(span) } else { inner.push(span) }
            }
        }
        inner.sort();
        let mut w = Window { inner, preds: HashMap::new(), open: HashSet::new() };
        let spans: Vec<(usize, usize, bool)> =
            w.inner.iter().map(|&(a, b)| (a, b, true)).chain(ring.iter().map(|&(a, b)| (a, b, false))).collect();
        for (a, b, is_inner) in spans {
            for u in a..b {
                let (_, _, first, degree) = self.node(u)?;
                for e in first as usize..(first + degree) as usize {
                    let v = self.edge(e)?.target as usize;
                    if w.inside(v) {
                        w.preds.entry(v as u32).or_default().push(u as u32);
                    } else if is_inner && !ring.iter().any(|&(x, y)| x <= v && v < y) && self.has_edge(v, u)? {
                        w.open.insert(u as u32);
                    }
                }
            }
        }
        Ok(w)
    }

    fn reaches_big(&mut self, start: usize, seen: &mut BitSet, visited: &mut Vec<u32>) -> Result<bool, String> {
        let mut stack = vec![start as u32];
        while let Some(u) = stack.pop() {
            let uu = u as usize;
            if seen.test(uu) {
                continue;
            }
            seen.set(uu);
            visited.push(u);
            if visited.len() >= PROBE_LIMIT {
                for &v in visited.iter() {
                    seen.clear(v as usize);
                }
                return Ok(true);
            }
            let (_, _, first, degree) = self.node(uu)?;
            for e in first as usize..(first + degree) as usize {
                let tgt = self.edge(e)?.target as usize;
                if !seen.test(tgt) {
                    stack.push(tgt as u32);
                }
            }
        }
        Ok(false)
    }

    fn cell_of(&self, lat: f64, lon: f64) -> Option<(usize, usize)> {
        if lat < self.min_lat || lat > self.max_lat || lon < self.min_lon || lon > self.max_lon {
            return None;
        }
        let col = (((lon - self.min_lon) / self.cell_size) as usize).min(self.grid_cols - 1);
        let row = (((lat - self.min_lat) / self.cell_size) as usize).min(self.grid_rows - 1);
        Some((col, row))
    }

    fn cell_bound(&mut self, cell: usize) -> Result<(usize, usize), String> {
        let raw = self.read_span(self.cells_off + cell as u64 * 4, 8)?;
        let a = format::u32_at(&raw, 0) as usize;
        let b = format::u32_at(&raw, 4) as usize;
        if b < a || b > self.node_count {
            return Err("cell index out of range".into());
        }
        Ok((a, b))
    }

    fn xy(&mut self, i: usize) -> Result<(i32, i32), String> {
        let s = (i & (XY_SLOTS - 1)) * 3;
        if self.xy_cache[s] == i as u32 {
            return Ok((self.xy_cache[s + 1] as i32, self.xy_cache[s + 2] as i32));
        }
        let (lat, lon, _, _) = self.node(i)?;
        self.xy_cache[s] = i as u32;
        self.xy_cache[s + 1] = lat as u32;
        self.xy_cache[s + 2] = lon as u32;
        Ok((lat, lon))
    }

    fn twin_target(&mut self, a: usize, b: usize) -> Result<Option<usize>, String> {
        let want = self.xy(b)?;
        let (_, _, first, degree) = self.node(a)?;
        for e in first as usize..(first + degree) as usize {
            let tgt = self.edge(e)?.target as usize;
            if self.xy(tgt)? == want {
                return Ok(Some(tgt));
            }
        }
        Ok(None)
    }

    fn has_edge(&mut self, a: usize, b: usize) -> Result<bool, String> {
        let (_, _, first, degree) = self.node(a)?;
        for e in first as usize..(first + degree) as usize {
            if self.edge(e)?.target as usize == b {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn project(&mut self, p: (f64, f64), ei: usize) -> Result<(f64, f64, (f64, f64)), String> {
        let pts = self.edge_polyline(ei)?;
        if pts.len() < 2 {
            return Ok((f64::INFINITY, 0.0, p));
        }
        let mut seg_lens = Vec::with_capacity(pts.len() - 1);
        for w in pts.windows(2) {
            seg_lens.push(flat_m(w[0], w[1]));
        }
        let total: f64 = seg_lens.iter().sum();
        let mut best_dist = f64::INFINITY;
        let mut best_frac = 0.0;
        let mut best_point = pts[0];
        let mut cum = 0.0f64;
        for (i, w) in pts.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            let l = seg_lens[i];
            let t = if l <= 1e-9 {
                0.0
            } else {
                let k = ((a.0 + b.0) / 2.0).to_radians().cos();
                let (bx, by) = ((b.1 - a.1) * k, b.0 - a.0);
                let (px, py) = ((p.1 - a.1) * k, p.0 - a.0);
                let len2 = bx * bx + by * by;
                if len2 <= 0.0 { 0.0 } else { ((px * bx + py * by) / len2).clamp(0.0, 1.0) }
            };
            let proj = (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
            let d = flat_m(p, proj);
            if d < best_dist {
                best_dist = d;
                best_frac = if total <= 1e-9 { 0.0 } else { (cum + t * l) / total };
                best_point = proj;
            }
            cum += l;
        }
        Ok((best_dist, best_frac, best_point))
    }

    fn h(&mut self, node: usize, end_pt: (f64, f64)) -> Result<f64, String> {
        let (lat, lon, ..) = self.node(node)?;
        Ok(hav_m((micro_to_deg(lat), micro_to_deg(lon)), end_pt) / self.max_speed * 999.0)
    }

    fn h_xy(&self, xy: (i32, i32), end_pt: (f64, f64)) -> f64 {
        hav_m((micro_to_deg(xy.0), micro_to_deg(xy.1)), end_pt) / self.max_speed * 999.0
    }

    fn search(&mut self, s: &Snap, t: &Snap) -> Result<(Vec<usize>, u32, u32), String> {
        let end_pt = t.point;
        let start_pt = s.point;
        let ell_bound = ELLIPSE_K * hav_m(start_pt, end_pt) + ELLIPSE_C;
        let t_edge_weight = self.edge(t.edge as usize)?.weight;
        let t_tail_xy = self.xy(t.tail as usize)?;
        let t_head_xy = self.xy(t.head as usize)?;
        let mut g: HashMap<u64, u64, FxBuild> = HashMap::default();
        let mut prev: HashMap<u64, (u64, u32), FxBuild> = HashMap::default();
        let mut edat: HashMap<u32, (u32, u32, u8), FxBuild> = HashMap::default();
        let mut open: BinaryHeap<HeapItem> = BinaryHeap::new();

        for (node, in_edge, cost, from_node) in self.start_seeds(s)? {
            let key = state_key(node, in_edge);
            let better = match g.get(&key) {
                Some(&old) => cost < old as u32,
                None => true,
            };
            if better {
                g.insert(key, cost as u64);
                prev.insert(key, (u64::MAX, from_node));
                open.push(HeapItem(cost as f64 + self.h(node as usize, end_pt)?, key));
            }
        }
        if open.is_empty() {
            return Err("No route between these places".into());
        }

        let mut best = u64::MAX;
        let mut best_goal = u64::MAX;
        let mut settled_count = 0usize;

        while let Some(it) = open.pop() {
            if it.0 as u64 >= best {
                break;
            }
            let key = it.1;
            let v = match g.get(&key) {
                Some(&v) => v,
                None => continue,
            };
            if v >> 63 == 1 {
                continue;
            }
            let node = state_node(key) as usize;
            let in_edge = key as u32 as usize;
            let (nlat, nlon, first, degree) = self.node(node)?;
            let np = (micro_to_deg(nlat), micro_to_deg(nlon));
            if hav_m(np, start_pt) + hav_m(np, end_pt) > ell_bound {
                continue;
            }
            g.insert(key, v | (1 << 63));
            settled_count += 1;
            if settled_count > SETTLED_CAP {
                if best_goal != u64::MAX {
                    break;
                }
                return Err("No route found between these places".into());
            }
            let gg = v as u32;
            let (pa, e_or_from) = prev[&key];
            let a_node = if pa == u64::MAX { e_or_from } else { state_node(pa) };
            let a_xy = self.xy(a_node as usize)?;
            let (_, _, in_flags) = self.edge_c(in_edge, &mut edat)?;
            let in_restricted = in_flags & format::F_RESTRICTED != 0;
            let here = self.xy(node)?;
            if here == t_tail_xy && t.fwd && self.reaches_xy_c(node, t_head_xy, &mut edat)? {
                if t_head_xy != a_xy || self.uturn_allowed(first, degree, here, &mut edat)? {
                    let total = (gg as u64).saturating_add((t.frac * t_edge_weight as f64).round() as u64);
                    if total < best {
                        best = total;
                        best_goal = key;
                    }
                }
            }
            if here == t_head_xy && t.rev && self.reaches_xy_c(node, t_tail_xy, &mut edat)? {
                if t_tail_xy != a_xy || self.uturn_allowed(first, degree, here, &mut edat)? {
                    let total = (gg as u64).saturating_add(((1.0 - t.frac) * t_edge_weight as f64).round() as u64);
                    if total < best {
                        best = total;
                        best_goal = key;
                    }
                }
            }
            if (gg as f64) >= best as f64 {
                continue;
            }
            for e in first as usize..(first + degree) as usize {
                let (tgt, w, fl) = self.edge_c(e, &mut edat)?;
                let txy = self.xy(tgt as usize)?;
                if txy == a_xy && !self.uturn_allowed(first, degree, here, &mut edat)? {
                    continue;
                }
                let is_restricted = fl & format::F_RESTRICTED != 0;
                let penalty = if is_restricted && !in_restricted { RESTRICTED_ENTRY_MS } else { 0 };
                let ng = gg.saturating_add(w).saturating_add(penalty);
                let nkey = state_key(tgt, e as u32);
                let better = match g.get(&nkey) {
                    Some(&old) => old >> 63 == 0 && ng < old as u32,
                    None => true,
                };
                if better {
                    g.insert(nkey, ng as u64);
                    prev.insert(nkey, (key, e as u32));
                    open.push(HeapItem(ng as f64 + self.h_xy(txy, end_pt), nkey));
                }
            }
        }

        if best_goal == u64::MAX {
            return Err("No route between these places".into());
        }

        let mut edges: Vec<usize> = Vec::new();
        let mut cur = best_goal;
        let mut start = state_node(cur);
        loop {
            let (p, e) = prev[&cur];
            if p == u64::MAX {
                break;
            }
            edges.push(e as usize);
            cur = p;
            start = state_node(cur);
        }
        edges.reverse();
        Ok((edges, start, state_node(best_goal)))
    }

    fn edge_c(&mut self, ei: usize, edat: &mut HashMap<u32, (u32, u32, u8), FxBuild>) -> Result<(u32, u32, u8), String> {
        if let Some(&d) = edat.get(&(ei as u32)) {
            return Ok(d);
        }
        let e = self.edge(ei)?;
        let d = (e.target, e.weight, e.flags);
        edat.insert(ei as u32, d);
        Ok(d)
    }

    fn reaches_xy_c(&mut self, a: usize, want: (i32, i32), edat: &mut HashMap<u32, (u32, u32, u8), FxBuild>) -> Result<bool, String> {
        let (_, _, first, degree) = self.node(a)?;
        for e in first as usize..(first + degree) as usize {
            let (tgt, _, _) = self.edge_c(e, edat)?;
            if self.xy(tgt as usize)? == want {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn uturn_allowed(&mut self, first: u32, degree: u32, uxy: (i32, i32), edat: &mut HashMap<u32, (u32, u32, u8), FxBuild>) -> Result<bool, String> {
        if degree <= 2 {
            return Ok(true);
        }
        let mut seen: Vec<(i32, i32)> = Vec::new();
        let mut bidir = 0usize;
        for e in first as usize..(first + degree) as usize {
            let (w, _, _) = self.edge_c(e, edat)?;
            let wxy = self.xy(w as usize)?;
            if seen.contains(&wxy) {
                continue;
            }
            seen.push(wxy);
            if self.reaches_xy_c(w as usize, uxy, edat)? {
                bidir += 1;
            }
        }
        Ok(bidir <= 1)
    }

    fn start_seeds(&mut self, s: &Snap) -> Result<Vec<(u32, u32, u32, u32)>, String> {
        let len_ms = self.edge(s.edge as usize)?.weight as f64;
        let mut out = Vec::new();
        if s.fwd {
            out.push((s.head, s.edge, ((1.0 - s.frac) * len_ms).round() as u32, s.tail));
        }
        if s.rev {
            if let Some(e) = self.edge_to(s.head as usize, s.back as usize)? {
                out.push((s.back, e as u32, (s.frac * len_ms).round() as u32, s.head));
            }
        }
        Ok(out)
    }

    fn edge_to(&mut self, a: usize, b: usize) -> Result<Option<usize>, String> {
        let (_, _, first, degree) = self.node(a)?;
        for e in first as usize..(first + degree) as usize {
            if self.edge(e)?.target as usize == b {
                return Ok(Some(e));
            }
        }
        Ok(None)
    }

    fn render(&mut self, s: &Snap, t: &Snap, path: &[usize], from_head: bool, to_tail: bool) -> Result<Route, String> {
        let mut pts: Vec<(f64, f64)> = Vec::new();
        let pl = self.edge_polyline(s.edge as usize)?;
        pts.extend(sub_segment(&pl, s.frac, if from_head { 1.0 } else { 0.0 }));
        let mut geom: Vec<Vec<(f64, f64)>> = Vec::with_capacity(path.len());
        let mut class = Vec::with_capacity(path.len());
        let mut flags = Vec::with_capacity(path.len());
        let mut name_idx = Vec::with_capacity(path.len());
        for &e in path {
            let pl = self.edge_polyline(e)?;
            let ed = self.edge(e)?;
            geom.push(pl);
            class.push(ed.cls);
            flags.push(ed.flags);
            name_idx.push(ed.name);
        }
        for pl in &geom {
            pts.extend_from_slice(pl);
        }
        let pl = self.edge_polyline(t.edge as usize)?;
        pts.extend(sub_segment(&pl, if to_tail { 0.0 } else { 1.0 }, t.frac));

        let mut distance_m = 0.0f64;
        let mut line: Vec<[f64; 2]> = Vec::new();
        if let Some(&p0) = pts.first() {
            line.push([p0.1, p0.0]);
        }
        for w in pts.windows(2) {
            distance_m += hav_m(w[0], w[1]);
            line.push([w[1].1, w[1].0]);
        }

        let mut duration_s = 0.0f64;
        for &e in path {
            duration_s += self.edge(e)?.weight as f64 / 1000.0;
        }
        duration_s += self.edge(s.edge as usize)?.weight as f64 / 1000.0 * if from_head { 1.0 - s.frac } else { s.frac };
        duration_s += self.edge(t.edge as usize)?.weight as f64 / 1000.0 * if to_tail { t.frac } else { 1.0 - t.frac };

        let (steps, turn_penalty) = if path.is_empty() {
            let name_idx = self.edge(s.edge as usize)?.name;
            let name = self.name_of(name_idx);
            let first = if name.is_empty() {
                "Head out".to_string()
            } else {
                format!("Head out on {name}")
            };
            (
                vec![
                    Step {
                        text: first,
                        distance_m,
                        at: line.first().copied().unwrap_or([0.0, 0.0]),
                    },
                    Step {
                        text: "Arrive at destination".to_string(),
                        distance_m: 0.0,
                        at: line.last().copied().unwrap_or([0.0, 0.0]),
                    },
                ],
                0.0,
            )
        } else {
            self.build_steps(path, &geom, &class, &flags, &name_idx)?
        };
        duration_s += turn_penalty;
        Ok(Route {
            distance_m,
            duration_s,
            steps,
            line,
        })
    }

    fn direct(&mut self, s: &Snap, t: &Snap) -> Result<Option<Route>, String> {
        let tf = if t.edge == s.edge {
            t.frac
        } else if self.xy(t.tail as usize)? == self.xy(s.head as usize)? && self.xy(t.head as usize)? == self.xy(s.tail as usize)? {
            1.0 - t.frac
        } else {
            return Ok(None);
        };
        let forward = tf >= s.frac;
        if (forward && !s.fwd) || (!forward && !s.rev) {
            return Ok(None);
        }
        let ed = self.edge(s.edge as usize)?;
        let pl = self.edge_polyline(s.edge as usize)?;
        let seg = sub_segment(&pl, s.frac, tf);
        let mut distance_m = 0.0f64;
        let mut line: Vec<[f64; 2]> = seg.iter().take(1).map(|p| [p.1, p.0]).collect();
        for w in seg.windows(2) {
            distance_m += hav_m(w[0], w[1]);
            line.push([w[1].1, w[1].0]);
        }
        let duration_s = ed.weight as f64 / 1000.0 * (tf - s.frac).abs();
        let (steps, _) = self.build_steps(&[s.edge as usize], &[seg], &[ed.cls], &[ed.flags], &[ed.name])?;
        Ok(Some(Route { distance_m, duration_s, steps, line }))
    }

    fn name_of(&mut self, idx: u32) -> String {
        if idx as usize >= self.name_count {
            return String::new();
        }
        let a = self.name_off[idx as usize] as u64;
        let b = self.name_off[idx as usize + 1] as u64;
        if b > a && b - a <= (1 << 16) {
            match self.read_span(self.names_off + a, (b - a) as usize) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(_) => String::new(),
            }
        } else {
            String::new()
        }
    }

    fn build_steps(
        &mut self,
        path: &[usize],
        geom: &[Vec<(f64, f64)>],
        class: &[u8],
        flags: &[u8],
        name_idx: &[u32],
    ) -> Result<(Vec<Step>, f64), String> {
        if path.is_empty() {
            return Ok((
                vec![Step {
                    text: "Arrive at destination".into(),
                    distance_m: 0.0,
                    at: [0.0, 0.0],
                }],
                0.0,
            ));
        }
        let names: Vec<String> = name_idx.iter().map(|&n| self.name_of(n)).collect();
        let mut entry_b = Vec::with_capacity(path.len());
        let mut exit_b = Vec::with_capacity(path.len());
        for pl in geom {
            entry_b.push(entry_bearing(pl));
            exit_b.push(exit_bearing(pl));
        }
        let is_round = |i: usize| flags[i] & (format::F_ROUNDABOUT | format::F_CIRCULAR) != 0;
        let is_link = |i: usize| format::class_is_link(class[i]);

        let mut turn_penalty = 0.0;
        let mut on_roundabout = is_round(0);
        let mut exits_passed = 0usize;
        let mut boundaries: Vec<(String, usize)> = Vec::new();
        let first = if on_roundabout {
            "Enter the roundabout".to_string()
        } else if names[0].is_empty() {
            "Head out".to_string()
        } else {
            format!("Head out on {}", names[0])
        };
        boundaries.push((first, 0));

        for i in 1..path.len() {
            let prev_round = is_round(i - 1);
            let cur_round = is_round(i);
            let angle = turn_angle(exit_b[i - 1], entry_b[i]);
            turn_penalty += turn_penalty_s(angle.abs());
            let mut maneuver: Option<String> = None;

            if cur_round && !prev_round {
                on_roundabout = true;
                exits_passed = 0;
                maneuver = Some("Enter the roundabout".to_string());
            } else if prev_round && !cur_round {
                let exit_no = exits_passed + 1;
                let dest = &names[i];
                maneuver = Some(if dest.is_empty() {
                    format!("At the roundabout, take the {} exit", ordinal(exit_no))
                } else {
                    format!("At the roundabout, take the {} exit onto {dest}", ordinal(exit_no))
                });
                on_roundabout = false;
            } else if on_roundabout && cur_round {
                let node = self.edge(path[i])?.src as usize;
                exits_passed += self.non_roundabout_exits(node)?;
            } else {
                let name_change = name_idx[i] != name_idx[i - 1];
                let ramping = is_link(i) && !is_link(i - 1);
                let merging = is_link(i - 1) && !is_link(i);
                let a = angle.abs();
                if a > 150.0 {
                    maneuver = Some("Make a U-turn".to_string());
                } else if ramping {
                    maneuver = Some(if names[i].is_empty() {
                        "Take the ramp".to_string()
                    } else {
                        format!("Take the ramp onto {}", names[i])
                    });
                } else if merging {
                    maneuver = Some(if names[i].is_empty() {
                        "Merge onto the road ahead".to_string()
                    } else {
                        format!("Keep {} onto {}", dir_of(angle), names[i])
                    });
                } else if name_change && a < 30.0 {
                    maneuver = Some(if names[i].is_empty() {
                        "Continue straight".to_string()
                    } else {
                        format!("Continue onto {}", names[i])
                    });
                } else if name_change || a > 30.0 {
                    let w = turn_word(a, angle);
                    maneuver = Some(if names[i].is_empty() {
                        format!("Turn {w}")
                    } else {
                        format!("Turn {w} onto {}", names[i])
                    });
                }
            }
            if let Some(m) = maneuver {
                boundaries.push((m, i));
            }
        }

        let mut steps = Vec::with_capacity(boundaries.len() + 1);
        for k in 0..boundaries.len() {
            let start = boundaries[k].1;
            let end = if k + 1 < boundaries.len() {
                boundaries[k + 1].1
            } else {
                path.len()
            };
            let mut dist = 0.0;
            for j in start..end {
                for w in geom[j].windows(2) {
                    dist += hav_m(w[0], w[1]);
                }
            }
            let at = geom[start].first().copied().unwrap_or((0.0, 0.0));
            steps.push(Step {
                text: boundaries[k].0.clone(),
                distance_m: dist,
                at: [at.1, at.0],
            });
        }
        let end_at = geom.last().and_then(|g| g.last()).copied().unwrap_or((0.0, 0.0));
        steps.push(Step {
            text: "Arrive at destination".into(),
            distance_m: 0.0,
            at: [end_at.1, end_at.0],
        });
        Ok((steps, turn_penalty))
    }

    fn non_roundabout_exits(&mut self, node: usize) -> Result<usize, String> {
        let (_, _, first, degree) = self.node(node)?;
        let mut c = 0;
        for e in first as usize..(first + degree) as usize {
            if self.edge(e)?.flags & (format::F_ROUNDABOUT | format::F_CIRCULAR) == 0 {
                c += 1;
            }
        }
        Ok(c)
    }
}

fn bearing(a: (f64, f64), b: (f64, f64)) -> f64 {
    let dlat = b.0 - a.0;
    let dlon = b.1 - a.1;
    let lat = ((a.0 + b.0) / 2.0).to_radians();
    let e = dlon * lat.cos();
    let mut d = e.atan2(dlat).to_degrees();
    if d < 0.0 {
        d += 360.0;
    }
    d
}

fn entry_bearing(pl: &[(f64, f64)]) -> f64 {
    if pl.len() >= 2 {
        bearing(pl[0], pl[1])
    } else {
        0.0
    }
}

fn exit_bearing(pl: &[(f64, f64)]) -> f64 {
    let n = pl.len();
    if n >= 2 {
        bearing(pl[n - 2], pl[n - 1])
    } else {
        0.0
    }
}

fn turn_angle(b_in: f64, b_out: f64) -> f64 {
    let mut d = b_out - b_in;
    while d > 180.0 {
        d -= 360.0;
    }
    while d < -180.0 {
        d += 360.0;
    }
    d
}

fn dir_of(angle: f64) -> &'static str {
    if angle < 0.0 {
        "left"
    } else {
        "right"
    }
}

fn turn_word(a: f64, signed: f64) -> String {
    let dir = dir_of(signed);
    if a < 45.0 {
        format!("slight {dir}")
    } else if a < 135.0 {
        dir.to_string()
    } else {
        format!("sharp {dir}")
    }
}

fn turn_penalty_s(a: f64) -> f64 {
    if a > 150.0 {
        20.0
    } else if a > 45.0 {
        7.5
    } else if a > 15.0 {
        2.0
    } else {
        0.0
    }
}

fn ordinal(n: usize) -> String {
    match n % 100 {
        11 | 12 | 13 => format!("{n}th"),
        _ => match n % 10 {
            1 => format!("{n}st"),
            2 => format!("{n}nd"),
            3 => format!("{n}rd"),
            _ => format!("{n}th"),
        },
    }
}

struct Snap {
    tail: u32,
    head: u32,
    edge: u32,
    frac: f64,
    point: (f64, f64),
    fwd: bool,
    rev: bool,
    back: u32,
}

struct BitSet {
    words: Vec<u64>,
}

impl BitSet {
    fn new(n: usize) -> BitSet {
        BitSet {
            words: vec![0u64; (n + 63) / 64],
        }
    }
    fn test(&self, i: usize) -> bool {
        self.words[i >> 6] >> (i & 63) & 1 == 1
    }
    fn set(&mut self, i: usize) {
        self.words[i >> 6] |= 1 << (i & 63);
    }
    fn clear(&mut self, i: usize) {
        self.words[i >> 6] &= !(1 << (i & 63));
    }
}

struct Window {
    inner: Vec<(usize, usize)>,
    preds: HashMap<u32, Vec<u32>>,
    open: HashSet<u32>,
}

impl Window {
    fn inside(&self, n: usize) -> bool {
        let i = self.inner.partition_point(|&(a, _)| a <= n);
        i > 0 && n < self.inner[i - 1].1
    }
}

#[derive(PartialEq, Clone, Copy)]
struct HeapItem(f64, u64);

impl Eq for HeapItem {}

impl std::cmp::Ord for HeapItem {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.0.total_cmp(&self.0).then_with(|| o.1.cmp(&self.1))
    }
}

impl std::cmp::PartialOrd for HeapItem {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

fn sub_segment(pl: &[(f64, f64)], f0: f64, f1: f64) -> Vec<(f64, f64)> {
    if pl.len() < 2 {
        return vec![];
    }
    let mut seg_lens = Vec::with_capacity(pl.len() - 1);
    let mut total = 0.0;
    for w in pl.windows(2) {
        let l = flat_m(w[0], w[1]);
        seg_lens.push(l);
        total += l;
    }
    if total <= 1e-9 {
        return vec![pl[0], pl[pl.len() - 1]];
    }
    let lo = f0.min(f1);
    let hi = f0.max(f1);
    let forward = f1 > f0;
    let mut out = vec![point_at(pl, &seg_lens, total, if forward { lo } else { hi })];
    for (i, w) in pl.windows(2).enumerate() {
        let s = cum(&seg_lens, i) / total;
        let e = (cum(&seg_lens, i) + seg_lens[i]) / total;
        if e > lo && s < hi {
            let cl = s.max(lo);
            let ce = e.min(hi);
            let t0 = (cl - s) / (e - s).max(1e-12);
            let t1 = (ce - s) / (e - s).max(1e-12);
            if forward {
                out.push(interpolate(w[0], w[1], t0));
                out.push(interpolate(w[0], w[1], t1));
            } else {
                out.push(interpolate(w[0], w[1], t1));
                out.push(interpolate(w[0], w[1], t0));
            }
        }
    }
    out.push(point_at(pl, &seg_lens, total, if forward { hi } else { lo }));
    out.dedup();
    out
}

fn cum(seg_lens: &[f64], i: usize) -> f64 {
    seg_lens[..i].iter().sum()
}

fn point_at(pl: &[(f64, f64)], seg_lens: &[f64], total: f64, f: f64) -> (f64, f64) {
    let target = f * total;
    let mut c = 0.0;
    for (i, w) in pl.windows(2).enumerate() {
        let l = seg_lens[i];
        if target <= c + l || i == seg_lens.len() - 1 {
            let t = if l <= 1e-9 { 0.0 } else { ((target - c) / l).clamp(0.0, 1.0) };
            return interpolate(w[0], w[1], t);
        }
        c += l;
    }
    pl[pl.len() - 1]
}

fn interpolate(a: (f64, f64), b: (f64, f64), t: f64) -> (f64, f64) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}
