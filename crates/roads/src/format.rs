use std::io::Write;

pub const MAGIC: &[u8; 8] = b"ALXROAD1";
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 64;
pub const NODE_REC: usize = 16;
pub const EDGE_REC: usize = 24;

pub const C_MOTORWAY: u8 = 0;
pub const C_MOTORWAY_LINK: u8 = 1;
pub const C_TRUNK: u8 = 2;
pub const C_TRUNK_LINK: u8 = 3;
pub const C_PRIMARY: u8 = 4;
pub const C_PRIMARY_LINK: u8 = 5;
pub const C_SECONDARY: u8 = 6;
pub const C_SECONDARY_LINK: u8 = 7;
pub const C_TERTIARY: u8 = 8;
pub const C_TERTIARY_LINK: u8 = 9;
pub const C_UNCLASSIFIED: u8 = 10;
pub const C_RESIDENTIAL: u8 = 11;
pub const C_LIVING_STREET: u8 = 12;
pub const C_SERVICE: u8 = 13;
pub const C_FERRY: u8 = 14;
pub const C_OTHER: u8 = 15;

pub const F_ROUNDABOUT: u8 = 1;
pub const F_CIRCULAR: u8 = 2;
pub const F_LINK: u8 = 4;
pub const F_FERRY: u8 = 8;
pub const F_RESTRICTED: u8 = 16;

pub fn class_speed_kmh(c: u8) -> f64 {
    match c {
        C_MOTORWAY => 90.0,
        C_MOTORWAY_LINK => 45.0,
        C_TRUNK => 85.0,
        C_TRUNK_LINK => 40.0,
        C_PRIMARY => 65.0,
        C_PRIMARY_LINK => 30.0,
        C_SECONDARY => 55.0,
        C_SECONDARY_LINK => 25.0,
        C_TERTIARY => 40.0,
        C_TERTIARY_LINK => 20.0,
        C_UNCLASSIFIED => 25.0,
        C_RESIDENTIAL => 25.0,
        C_LIVING_STREET => 10.0,
        C_SERVICE => 15.0,
        C_FERRY => 5.0,
        _ => 25.0,
    }
}

pub fn class_is_link(c: u8) -> bool {
    matches!(
        c,
        C_MOTORWAY_LINK | C_TRUNK_LINK | C_PRIMARY_LINK | C_SECONDARY_LINK | C_TERTIARY_LINK
    )
}

pub fn write_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            return;
        }
    }
}

pub fn read_varint(b: &[u8], pos: &mut usize) -> Result<u64, String> {
    let mut out: u64 = 0;
    let mut shift: u32 = 0;
    loop {
        if *pos >= b.len() {
            return Err("truncated varint".into());
        }
        let byte = b[*pos];
        *pos += 1;
        if shift >= 64 {
            return Err("varint overflow".into());
        }
        out |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(out);
        }
        shift += 7;
    }
}

pub fn zigzag_encode(v: i32) -> u64 {
    (((v << 1) ^ (v >> 31)) as u32) as u64
}

pub fn zigzag_decode(v: u64) -> i32 {
    ((v >> 1) as i32) ^ -((v & 1) as i32)
}

pub fn push_geometry(blob: &mut Vec<u8>, pts: &[(i32, i32)]) -> u32 {
    let off = blob.len() as u32;
    if pts.len() <= 2 {
        return off;
    }
    let mut prev = (pts[0].0 as i64, pts[0].1 as i64);
    for &p in &pts[1..pts.len() - 1] {
        write_varint(blob, zigzag_encode((p.0 as i64 - prev.0) as i32));
        write_varint(blob, zigzag_encode((p.1 as i64 - prev.1) as i32));
        prev = (p.0 as i64, p.1 as i64);
    }
    off
}

pub fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

pub fn i32_at(b: &[u8], i: usize) -> i32 {
    i32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

pub struct Grid {
    pub cols: u32,
    pub rows: u32,
    pub min_lat: i32,
    pub min_lon: i32,
    pub cell_size: u32,
}

impl Grid {
    pub fn cells(&self) -> u64 {
        self.cols as u64 * self.rows as u64
    }

    pub fn cell_of(&self, lat: f64, lon: f64) -> Option<(u32, u32)> {
        let ilat = (lat * 1e6).round() as i32;
        let ilon = (lon * 1e6).round() as i32;
        if ilat < self.min_lat || ilat > self.max_lat() || ilon < self.min_lon || ilon > self.max_lon() {
            return None;
        }
        let col = ((ilon - self.min_lon) as u32) / self.cell_size;
        let row = ((ilat - self.min_lat) as u32) / self.cell_size;
        Some((col.min(self.cols - 1), row.min(self.rows - 1)))
    }

    fn max_lat(&self) -> i32 {
        self.min_lat + (self.rows as i64 * self.cell_size as i64) as i32
    }

    fn max_lon(&self) -> i32 {
        self.min_lon + (self.cols as i64 * self.cell_size as i64) as i32
    }
}

pub fn order_by_cell(
    lat: &[i32],
    lon: &[i32],
    grid: &Grid,
) -> (Vec<u32>, Vec<u32>) {
    let n = lat.len();
    let cell_count = grid.cells() as usize;
    let mut counts = vec![0u32; cell_count];
    let mut cell_of_node = vec![0u32; n];
    for i in 0..n {
        let col = (((lon[i] - grid.min_lon) as u32) / grid.cell_size).min(grid.cols - 1);
        let row = (((lat[i] - grid.min_lat) as u32) / grid.cell_size).min(grid.rows - 1);
        let c = row * grid.cols + col;
        cell_of_node[i] = c;
        counts[c as usize] += 1;
    }
    let mut cell_first = vec![0u32; cell_count + 1];
    for c in 0..cell_count {
        cell_first[c + 1] = cell_first[c] + counts[c];
    }
    let mut next = cell_first.clone();
    let mut perm = vec![0u32; n];
    for i in 0..n {
        let c = cell_of_node[i] as usize;
        perm[i] = next[c];
        next[c] += 1;
    }
    (perm, cell_first)
}

pub struct Writer {
    pub node_lat: Vec<i32>,
    pub node_lon: Vec<i32>,
    pub first_edge: Vec<u32>,
    pub degree: Vec<u32>,
    pub edge_source: Vec<u32>,
    pub edge_target: Vec<u32>,
    pub edge_weight_ms: Vec<u32>,
    pub edge_name: Vec<u32>,
    pub edge_class: Vec<u8>,
    pub edge_flags: Vec<u8>,
    pub edge_geom: Vec<u32>,
    pub edge_geom_count: Vec<u16>,
    pub geometry: Vec<u8>,
    pub names: Vec<Vec<u8>>,
    pub cell_first: Vec<u32>,
    pub grid_cols: u32,
    pub grid_rows: u32,
    pub cell_size: u32,
    pub min_lat: i32,
    pub min_lon: i32,
    pub max_lat: i32,
    pub max_lon: i32,
    pub max_speed_x10: u32,
}

impl Writer {
    pub fn new() -> Writer {
        Writer {
            node_lat: Vec::new(),
            node_lon: Vec::new(),
            first_edge: Vec::new(),
            degree: Vec::new(),
            edge_source: Vec::new(),
            edge_target: Vec::new(),
            edge_weight_ms: Vec::new(),
            edge_name: Vec::new(),
            edge_class: Vec::new(),
            edge_flags: Vec::new(),
            edge_geom: Vec::new(),
            edge_geom_count: Vec::new(),
            geometry: Vec::new(),
            names: Vec::new(),
            cell_first: Vec::new(),
            grid_cols: 0,
            grid_rows: 0,
            cell_size: 1,
            min_lat: 0,
            min_lon: 0,
            max_lat: 0,
            max_lon: 0,
            max_speed_x10: 0,
        }
    }

    pub fn write(&self, path: &std::path::Path) -> Result<(), String> {
        let node_count = self.node_lat.len();
        let edge_count = self.edge_target.len();
        let name_count = self.names.len();
        if self.node_lon.len() != node_count
            || self.first_edge.len() != node_count
            || self.degree.len() != node_count
        {
            return Err("node arrays disagree".into());
        }
        if self.edge_source.len() != edge_count
            || self.edge_weight_ms.len() != edge_count
            || self.edge_name.len() != edge_count
            || self.edge_class.len() != edge_count
            || self.edge_flags.len() != edge_count
            || self.edge_geom.len() != edge_count
            || self.edge_geom_count.len() != edge_count
        {
            return Err("edge arrays disagree".into());
        }
        let cell_count = self.grid_cols as u64 * self.grid_rows as u64;
        if self.cell_first.len() as u64 != cell_count + 1 {
            return Err("cell_first disagrees with grid".into());
        }
        if node_count as u64 >= u32::MAX as u64 || edge_count as u64 >= u32::MAX as u64 {
            return Err("graph too large for this format version".into());
        }
        for &name in &self.edge_name {
            if name as usize >= name_count {
                return Err("edge name index out of range".into());
            }
        }
        for (&off, &cnt) in self.edge_geom.iter().zip(self.edge_geom_count.iter()) {
            if cnt > 0 && off as u64 >= self.geometry.len() as u64 {
                return Err("edge geometry offset out of range".into());
            }
        }

        let mut out: Vec<u8> = Vec::with_capacity(HEADER_LEN + node_count * NODE_REC + edge_count * EDGE_REC);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(node_count as u32).to_le_bytes());
        out.extend_from_slice(&(edge_count as u32).to_le_bytes());
        out.extend_from_slice(&(name_count as u32).to_le_bytes());
        out.extend_from_slice(&(self.geometry.len() as u32).to_le_bytes());
        out.extend_from_slice(&(cell_count as u32).to_le_bytes());
        out.extend_from_slice(&self.grid_cols.to_le_bytes());
        out.extend_from_slice(&self.grid_rows.to_le_bytes());
        out.extend_from_slice(&self.min_lat.to_le_bytes());
        out.extend_from_slice(&self.min_lon.to_le_bytes());
        out.extend_from_slice(&self.max_lat.to_le_bytes());
        out.extend_from_slice(&self.max_lon.to_le_bytes());
        out.extend_from_slice(&self.cell_size.to_le_bytes());
        out.extend_from_slice(&self.max_speed_x10.to_le_bytes());
        debug_assert_eq!(out.len(), HEADER_LEN);

        for i in 0..node_count {
            out.extend_from_slice(&self.node_lat[i].to_le_bytes());
            out.extend_from_slice(&self.node_lon[i].to_le_bytes());
            out.extend_from_slice(&self.first_edge[i].to_le_bytes());
            out.extend_from_slice(&self.degree[i].to_le_bytes());
        }
        for i in 0..edge_count {
            out.extend_from_slice(&self.edge_source[i].to_le_bytes());
            out.extend_from_slice(&self.edge_target[i].to_le_bytes());
            out.extend_from_slice(&self.edge_weight_ms[i].to_le_bytes());
            out.extend_from_slice(&self.edge_name[i].to_le_bytes());
            out.push(self.edge_class[i]);
            out.push(self.edge_flags[i]);
            out.extend_from_slice(&self.edge_geom[i].to_le_bytes());
            out.extend_from_slice(&self.edge_geom_count[i].to_le_bytes());
        }
        out.extend_from_slice(&cell_first_bytes(&self.cell_first));

        let mut name_offsets = Vec::with_capacity((name_count + 1) * 4);
        let mut blob: Vec<u8> = Vec::new();
        let mut off = 0u32;
        name_offsets.push(0u32);
        for n in &self.names {
            off += n.len() as u32;
            name_offsets.push(off);
            blob.extend_from_slice(n);
        }
        for o in &name_offsets {
            out.extend_from_slice(&o.to_le_bytes());
        }
        out.extend_from_slice(&blob);
        out.extend_from_slice(&self.geometry);

        let mut f = std::fs::File::create(path).map_err(|e| e.to_string())?;
        f.write_all(&out).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn cell_first_bytes(v: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}
