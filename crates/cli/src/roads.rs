use alexandria_roads::{push_geometry, order_by_cell, Grid, Writer};
use osmpbf::{Element, ElementReader};
use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

const MILES_TO_KM: f64 = 1.609344;
const CELL_SIZE: u32 = 20000;

fn flat_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    const R: f64 = 6371000.0;
    let lat = (a.0 + b.0) / 2.0;
    let dlat = (b.0 - a.0).to_radians() * R;
    let dlon = (b.1 - a.1).to_radians() * R * lat.to_radians().cos();
    (dlat * dlat + dlon * dlon).sqrt()
}

#[derive(Clone)]
struct Tags {
    k: Vec<String>,
    v: Vec<String>,
}

impl Tags {
    fn new() -> Tags {
        Tags { k: Vec::new(), v: Vec::new() }
    }

    fn collect<'a>(it: impl Iterator<Item = (&'a str, &'a str)>) -> Tags {
        let mut t = Tags::new();
        for (k, v) in it {
            t.k.push(k.to_string());
            t.v.push(v.to_string());
        }
        t
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.k
            .iter()
            .position(|k| k == key)
            .map(|i| self.v[i].as_str())
    }
}

fn class_of(highway: &str) -> Option<u8> {
    use alexandria_roads::*;
    Some(match highway {
        "motorway" => C_MOTORWAY,
        "motorway_link" => C_MOTORWAY_LINK,
        "trunk" => C_TRUNK,
        "trunk_link" => C_TRUNK_LINK,
        "primary" => C_PRIMARY,
        "primary_link" => C_PRIMARY_LINK,
        "secondary" => C_SECONDARY,
        "secondary_link" => C_SECONDARY_LINK,
        "tertiary" => C_TERTIARY,
        "tertiary_link" => C_TERTIARY_LINK,
        "unclassified" => C_UNCLASSIFIED,
        "residential" => C_RESIDENTIAL,
        "living_street" => C_LIVING_STREET,
        "service" => C_SERVICE,
        "winter_road" | "ice_road" => C_UNCLASSIFIED,
        _ => return None,
    })
}

const BLACKLIST: &[&str] = &[
    "no", "private", "agricultural", "forestry", "emergency", "psv", "taxi", "share_taxi", "minibus", "bus",
    "foot", "emergency_vehicle", "restricted", "military", "official",
];

const RESTRICTED_ACCESS: &[&str] = &["destination", "customers", "delivery", "permit", "residents", "unknown"];

const RESTRICTED_HIGHWAY: &[&str] = &[
    "motorway", "motorway_link", "trunk", "trunk_link", "primary", "primary_link",
    "secondary", "secondary_link", "tertiary", "tertiary_link", "residential",
    "living_street", "unclassified", "service", "winter_road", "ice_road",
];

fn effective_access(t: &Tags) -> Option<&str> {
    for key in ["motorcar", "motor_vehicle", "vehicle", "access"] {
        if let Some(v) = t.get(key) {
            return Some(v);
        }
    }
    None
}

fn access_allowed(t: &Tags) -> bool {
    match effective_access(t) {
        Some(v) => !BLACKLIST.contains(&v),
        None => true,
    }
}

fn parse_maxspeed(s: &str) -> f64 {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return 0.0;
    }
    let mut n: f64 = digits.parse().unwrap_or(0.0);
    if s.contains("mph") || s.contains("mp/h") {
        n *= MILES_TO_KM;
    }
    if n > 200.0 {
        return 0.0;
    }
    n
}

fn maxspeed_country(s: &str) -> f64 {
    let lower = s.to_ascii_lowercase();
    if lower == "none" || lower == "walk" {
        return 0.0;
    }
    if let Some((_, tail)) = lower.split_once(':') {
        return match tail {
            "urban" => 50.0,
            "rural" => 90.0,
            "trunk" => 110.0,
            "motorway" => 130.0,
            _ => 0.0,
        };
    }
    0.0
}

fn surface_cap(surface: Option<&str>) -> f64 {
    match surface {
        Some("gravel" | "dirt" | "unpaved" | "ground" | "grass" | "pebblestone" | "compacted"
        | "fine_gravel" | "earth" | "mud" | "sand" | "clay") => 40.0,
        Some("cobblestone") => 30.0,
        _ => 0.0,
    }
}

struct Classified {
    class: u8,
    flags: u8,
    fwd_eff: f64,
    bwd_eff: f64,
    fwd: bool,
    bwd: bool,
    name: String,
    ferry_ms: Option<u32>,
}

fn classify(_way_id: i64, t: &Tags) -> Option<Classified> {
    let highway = t.get("highway").unwrap_or("");
    let route = t.get("route").unwrap_or("");
    let is_ferry = route == "ferry";
    if highway.is_empty() && !is_ferry {
        return None;
    }
    if t.get("area") == Some("yes") {
        return None;
    }
    let construction = t.get("construction").unwrap_or("");
    if construction == "construction" || construction == "proposed" {
        return None;
    }

    let (class, flags) = if is_ferry {
        (alexandria_roads::C_FERRY, alexandria_roads::F_FERRY)
    } else {
        let c = class_of(highway)?;
        let mut f = 0u8;
        let junction = t.get("junction").unwrap_or("");
        if junction == "roundabout" {
            f |= alexandria_roads::F_ROUNDABOUT;
        } else if junction == "circular" {
            f |= alexandria_roads::F_CIRCULAR;
        }
        if alexandria_roads::class_is_link(c) {
            f |= alexandria_roads::F_LINK;
        }
        if let Some(acc) = effective_access(t) {
            if RESTRICTED_ACCESS.contains(&acc) && RESTRICTED_HIGHWAY.contains(&highway) {
                f |= alexandria_roads::F_RESTRICTED;
            }
        }
        (c, f)
    };

    if !access_allowed(t) {
        return None;
    }

    let oneway = t.get("oneway").unwrap_or("");
    let mut fwd = true;
    let mut bwd = true;
    if oneway == "reversible" || oneway == "alternating" {
        return None;
    }
    if oneway == "-1" {
        fwd = false;
    } else if oneway == "yes" || oneway == "1" || oneway == "true" {
        bwd = false;
    } else if class == alexandria_roads::C_MOTORWAY || flags & alexandria_roads::F_ROUNDABOUT != 0
        || flags & alexandria_roads::F_CIRCULAR != 0
    {
        if oneway != "no" {
            bwd = false;
        }
    }

    let ferry_ms = if is_ferry {
        if let Some(d) = t.get("duration") {
            parse_duration(d).map(|s| (s * 1000.0).round() as u32)
        } else {
            None
        }
    } else {
        None
    };

    let base_speed = if is_ferry {
        5.0
    } else {
        alexandria_roads::class_speed_kmh(class)
    };

    let mut speed_f = base_speed;
    let mut speed_b = base_speed;
    let mf = t
        .get("maxspeed:forward")
        .or_else(|| t.get("maxspeed"))
        .map(|s| parse_maxspeed(s).max(maxspeed_country(s)));
    let mb = t
        .get("maxspeed:backward")
        .or_else(|| t.get("maxspeed"))
        .map(|s| parse_maxspeed(s).max(maxspeed_country(s)));
    if let Some(n) = mf {
        if n > 0.0 {
            speed_f = n * 0.8;
        }
    }
    if let Some(n) = mb {
        if n > 0.0 {
            speed_b = n * 0.8;
        }
    }
    if let Some(surf) = t.get("surface") {
        let cap = surface_cap(Some(surf));
        if cap > 0.0 {
            speed_f = speed_f.min(cap);
            speed_b = speed_b.min(cap);
        }
    }
    let mut penalty = 1.0f64;
    if let Some(service) = t.get("service") {
        if matches!(service, "parking_aisle" | "driveway" | "drive-through" | "drive-thru" | "parking" | "alley") {
            penalty = 0.5;
        }
    }
    speed_f = speed_f.max(1.0);
    speed_b = speed_b.max(1.0);

    let name = t
        .get("name")
        .or_else(|| t.get("ref"))
        .unwrap_or("")
        .to_string();

    Some(Classified {
        class,
        flags,
        fwd_eff: speed_f * penalty,
        bwd_eff: speed_b * penalty,
        fwd,
        bwd,
        name,
        ferry_ms,
    })
}

fn parse_duration(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("PT") {
        let mut total = 0.0;
        let mut num = String::new();
        for ch in rest.chars() {
            if ch.is_ascii_digit() || ch == '.' {
                num.push(ch);
                continue;
            }
            let v: f64 = num.parse().ok()?;
            num.clear();
            total += v * match ch {
                'H' => 3600.0,
                'M' => 60.0,
                'S' => 1.0,
                _ => return None,
            };
        }
        return if num.is_empty() && !rest.is_empty() { Some(total) } else { None };
    }
    let parts: Vec<u32> = s.split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    match parts.as_slice() {
        [m] => Some(*m as f64 * 60.0),
        [h, m] => Some(*h as f64 * 3600.0 + *m as f64 * 60.0),
        [h, m, sec] => Some(*h as f64 * 3600.0 + *m as f64 * 60.0 + *sec as f64),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum RKind {
    NoTurn,
    Only,
}

struct Restriction {
    via_node: u64,
    from_way: u64,
    to_way: u64,
    kind: RKind,
}

fn collect_restrictions(in_path: &str) -> Result<Vec<Restriction>, String> {
    let mut out = Vec::new();
    let reader = ElementReader::from_path(in_path).map_err(|e| e.to_string())?;
    reader
        .for_each(|el| {
            if let Element::Relation(rel) = el {
                let tags = Tags::collect(rel.tags());
                let rel_type = tags.get("type").unwrap_or("");
                let applies = rel_type == "restriction"
                    || rel_type == "restriction:motorcar"
                    || rel_type == "restriction:motor_vehicle"
                    || rel_type == "restriction:vehicle";
                if !applies {
                    return;
                }
                if let Some(except) = tags.get("except") {
                    if except
                        .split(';')
                        .any(|e| matches!(e.trim(), "motorcar" | "motor_vehicle" | "vehicle"))
                    {
                        return;
                    }
                }
                let val = tags.get("restriction").unwrap_or("");
                let kind = if val.starts_with("only_") {
                    RKind::Only
                } else if val.starts_with("no_") {
                    RKind::NoTurn
                } else {
                    return;
                };
                let mut via_node = 0u64;
                let mut from_way = 0u64;
                let mut to_way = 0u64;
                for m in rel.members() {
                    match (m.role(), m.member_type) {
                        (Ok("via"), osmpbf::RelMemberType::Node) => via_node = m.member_id as u64,
                        (Ok("from"), osmpbf::RelMemberType::Way) => from_way = m.member_id as u64,
                        (Ok("to"), osmpbf::RelMemberType::Way) => to_way = m.member_id as u64,
                        _ => {}
                    }
                }
                if via_node != 0 && from_way != 0 && to_way != 0 {
                    out.push(Restriction {
                        via_node,
                        from_way,
                        to_way,
                        kind,
                    });
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(out)
}

pub fn build(in_path: &str, out_path: &str) -> Result<(), String> {
    let out = Path::new(out_path);
    let tmp = PathBuf::from(format!("{}.tmp-{}", out_path, std::process::id()));
    fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let ways_path = tmp.join("ways.bin");
    let refs_path = tmp.join("refs.bin");
    let nodes_path = tmp.join("nodes.bin");
    let nodesxy_path = tmp.join("nodesxy.bin");

    let result = build_inner(in_path, out, &ways_path, &refs_path, &nodes_path, &nodesxy_path);
    let _ = fs::remove_dir_all(&tmp);
    result
}

fn build_inner(
    in_path: &str,
    out: &Path,
    ways_path: &Path,
    refs_path: &Path,
    nodes_path: &Path,
    nodesxy_path: &Path,
) -> Result<(), String> {
    eprintln!("pass 1: ways");
    let mut ways_w = BufWriter::new(fs::File::create(ways_path).map_err(|e| e.to_string())?);
    let mut refs_w = BufWriter::new(fs::File::create(refs_path).map_err(|e| e.to_string())?);
    let mut way_count = 0u64;
    let mut ref_count = 0u64;
    let reader = ElementReader::from_path(in_path).map_err(|e| e.to_string())?;
    reader
        .for_each(|el| {
            if let Element::Way(w) = el {
                let tags = Tags::collect(w.tags());
                if let Some(c) = classify(w.id(), &tags) {
                    let refs: Vec<i64> = w.refs().collect();
                    if refs.len() < 2 {
                        return;
                    }
                    if refs.iter().any(|&r| r < 0) {
                        return;
                    }
                    ways_w.write_all(&(refs.len() as u32).to_le_bytes()).unwrap();
                    ways_w.write_all(&(w.id() as u64).to_le_bytes()).unwrap();
                    ways_w.write_all(&[c.class, c.flags]).unwrap();
                    ways_w
                        .write_all(&((c.fwd_eff * 10.0).round() as u32).to_le_bytes())
                        .unwrap();
                    ways_w
                        .write_all(&((c.bwd_eff * 10.0).round() as u32).to_le_bytes())
                        .unwrap();
                    ways_w.write_all(&(c.fwd as u8).to_le_bytes()).unwrap();
                    ways_w.write_all(&(c.bwd as u8).to_le_bytes()).unwrap();
                    let ferry = c.ferry_ms.unwrap_or(0);
                    ways_w.write_all(&ferry.to_le_bytes()).unwrap();
                    ways_w.write_all(&(c.name.len() as u16).to_le_bytes()).unwrap();
                    ways_w.write_all(c.name.as_bytes()).unwrap();
                    for (i, &r) in refs.iter().enumerate() {
                        let flag = if i == 0 || i == refs.len() - 1 { 1u64 } else { 0u64 };
                        let packed = ((r as u64) << 1) | flag;
                        refs_w.write_all(&packed.to_le_bytes()).unwrap();
                        ways_w.write_all(&(r as u64).to_le_bytes()).unwrap();
                        ref_count += 1;
                    }
                    way_count += 1;
                }
            }
        })
        .map_err(|e| e.to_string())?;
    ways_w.flush().map_err(|e| e.to_string())?;
    refs_w.flush().map_err(|e| e.to_string())?;
    drop(ways_w);
    drop(refs_w);
    eprintln!("pass 1 done: {way_count} ways, {ref_count} refs");

    eprintln!("sort refs");
    sort_and_aggregate(refs_path, nodes_path).map_err(|e| e.to_string())?;
    let node_count = fs::metadata(nodes_path).map_err(|e| e.to_string())?.len() / 16;
    eprintln!("referenced nodes: {node_count}");

    eprintln!("pass 2: node coords");
    fill_coords(in_path, nodes_path, nodesxy_path).map_err(|e| e.to_string())?;

    eprintln!("pass 3: restrictions");
    let restrictions = collect_restrictions(in_path)?;
    eprintln!("  {} turn restrictions", restrictions.len());

    eprintln!("pass 4: build graph");
    build_graph(ways_path, nodesxy_path, out, &restrictions).map_err(|e| e.to_string())?;
    Ok(())
}

fn sort_and_aggregate(refs_path: &Path, nodes_path: &Path) -> std::io::Result<()> {
    let size = fs::metadata(refs_path)?.len();
    let count = size / 8;
    let chunk = 50_000_000u64;
    let mut chunk_files = Vec::new();
    let mut f = BufReader::new(fs::File::open(refs_path)?);
    let mut remaining = count;
    let mut cidx = 0u32;
    while remaining > 0 {
        let n = remaining.min(chunk) as usize;
        let mut buf = vec![0u64; n];
        let mut raw = vec![0u8; n * 8];
        f.read_exact(&mut raw)?;
        for i in 0..n {
            buf[i] = u64::from_le_bytes([raw[i * 8], raw[i * 8 + 1], raw[i * 8 + 2], raw[i * 8 + 3], raw[i * 8 + 4], raw[i * 8 + 5], raw[i * 8 + 6], raw[i * 8 + 7]]);
        }
        buf.sort_unstable();
        let cf = refs_path.with_extension(format!("chunk{cidx}"));
        let mut w = BufWriter::new(fs::File::create(&cf)?);
        for v in &buf {
            w.write_all(&v.to_le_bytes())?;
        }
        w.flush()?;
        chunk_files.push(cf);
        remaining -= n as u64;
        cidx += 1;
        eprintln!("  sorted chunk {cidx}");
    }
    drop(f);

    let mut readers: Vec<BufReader<fs::File>> = chunk_files
        .iter()
        .map(|p| BufReader::new(fs::File::open(p).unwrap()))
        .collect();
    let mut heads: Vec<u64> = Vec::with_capacity(readers.len());
    for r in readers.iter_mut() {
        let mut b = [0u8; 8];
        if r.read_exact(&mut b).is_ok() {
            heads.push(u64::from_le_bytes(b));
        } else {
            heads.push(u64::MAX);
        }
    }
    let mut out = BufWriter::new(fs::File::create(nodes_path)?);
    let mut done = 0u64;
    let total_entries = count;
    while done < total_entries {
        let mut min_v = u64::MAX;
        let mut min_i = usize::MAX;
        for (i, &h) in heads.iter().enumerate() {
            if h < min_v {
                min_v = h;
                min_i = i;
            }
        }
        if min_i == usize::MAX {
            break;
        }
        let id = min_v >> 1;
        let mut middle = 0u32;
        let mut endpoint = 0u32;
        loop {
            if heads[min_i] & 1 == 1 {
                endpoint += 1;
            } else {
                middle += 1;
            }
            done += 1;
            let mut b = [0u8; 8];
            if readers[min_i].read_exact(&mut b).is_ok() {
                heads[min_i] = u64::from_le_bytes(b);
            } else {
                heads[min_i] = u64::MAX;
            }
            if done >= total_entries || heads[min_i] >> 1 != id {
                break;
            }
        }
        out.write_all(&id.to_le_bytes())?;
        out.write_all(&middle.to_le_bytes())?;
        out.write_all(&endpoint.to_le_bytes())?;
    }
    out.flush()?;
    for cf in chunk_files {
        let _ = fs::remove_file(cf);
    }
    let _ = fs::remove_file(refs_path);
    Ok(())
}

fn fill_coords(in_path: &str, nodes_path: &Path, nodesxy_path: &Path) -> std::io::Result<()> {
    let count = (fs::metadata(nodes_path)?.len() / 16) as usize;
    let mut ids = Vec::with_capacity(count);
    let mut junc = Vec::with_capacity(count);
    {
        let mut f = BufReader::new(fs::File::open(nodes_path)?);
        for _ in 0..count {
            let mut b = [0u8; 16];
            f.read_exact(&mut b)?;
            let id = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
            let mid = u32::from_le_bytes([b[8], b[9], b[10], b[11]]);
            let end = u32::from_le_bytes([b[12], b[13], b[14], b[15]]);
            let segments = 2u64 * mid as u64 + end as u64;
            ids.push(id);
            junc.push(end > 0 || segments > 2);
        }
    }
    let mut lats = vec![0i32; count];
    let mut lons = vec![0i32; count];
    let mut matched = 0u64;
    let mut seen = 0u64;

    let reader = ElementReader::from_path(in_path).map_err(|e| std::io::Error::other(e.to_string()))?;
    reader
        .for_each(|el| {
            let (id, lat, lon) = match el {
                Element::Node(n) => (n.id(), n.decimicro_lat() / 10, n.decimicro_lon() / 10),
                Element::DenseNode(dn) => (dn.id(), dn.decimicro_lat() / 10, dn.decimicro_lon() / 10),
                _ => return,
            };
            if id < 0 {
                return;
            }
            seen += 1;
            if let Ok(i) = ids.binary_search(&(id as u64)) {
                lats[i] = lat;
                lons[i] = lon;
                matched += 1;
            }
        })
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    eprintln!("  total nodes in stream: {seen}");

    let mut out = BufWriter::new(fs::File::create(nodesxy_path)?);
    for i in 0..count {
        out.write_all(&ids[i].to_le_bytes())?;
        out.write_all(&lats[i].to_le_bytes())?;
        out.write_all(&lons[i].to_le_bytes())?;
        out.write_all(&[junc[i] as u8])?;
    }
    out.flush()?;
    eprintln!("  matched {matched} / {count} node coordinates");
    if matched != count as u64 {
        eprintln!("WARNING: {}/{} referenced nodes missing coordinates", count as u64 - matched, count);
    }
    Ok(())
}

fn build_graph(ways_path: &Path, nodesxy_path: &Path, out: &Path, restrictions: &[Restriction]) -> std::io::Result<()> {
    let node_count = (fs::metadata(nodesxy_path)?.len() / 17) as usize;
    let mut ids = Vec::with_capacity(node_count);
    let mut lats = Vec::with_capacity(node_count);
    let mut lons = Vec::with_capacity(node_count);
    let mut junc = Vec::with_capacity(node_count);
    {
        let mut f = BufReader::new(fs::File::open(nodesxy_path)?);
        for _ in 0..node_count {
            let mut b = [0u8; 17];
            f.read_exact(&mut b)?;
            ids.push(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]));
            lats.push(i32::from_le_bytes([b[8], b[9], b[10], b[11]]));
            lons.push(i32::from_le_bytes([b[12], b[13], b[14], b[15]]));
            junc.push(b[16] != 0);
        }
    }

    let mut compact = vec![u32::MAX; node_count];
    let mut compact_count = 0u32;
    for i in 0..node_count {
        let missing = lats[i] == 0 && lons[i] == 0;
        if junc[i] && !missing {
            compact[i] = compact_count;
            compact_count += 1;
        }
    }
    eprintln!("  {compact_count} junction nodes out of {node_count} referenced");

    let mut max_abs_lat = 0.0f64;
    for i in 0..node_count {
        if compact[i] != u32::MAX {
            max_abs_lat = max_abs_lat.max((lats[i] as f64 / 1e6).abs());
        }
    }
    let max_piece_m = piece_limit_m(max_abs_lat);
    let mut new_nodes: Vec<(i32, i32)> = Vec::new();

    let mut names: Vec<Vec<u8>> = vec![Vec::new()];
    let mut name_ids: HashMap<String, u32> = HashMap::new();

    let mut esrc: Vec<u32> = Vec::new();
    let mut etgt: Vec<u32> = Vec::new();
    let mut eweight: Vec<u32> = Vec::new();
    let mut ename: Vec<u32> = Vec::new();
    let mut ecls: Vec<u8> = Vec::new();
    let mut eflags: Vec<u8> = Vec::new();
    let mut egeom: Vec<u32> = Vec::new();
    let mut egeomcnt: Vec<u16> = Vec::new();
    let mut eway: Vec<u64> = Vec::new();
    let mut geometry: Vec<u8> = Vec::new();
    let mut max_eff_x10 = 0u32;

    let ways_file = fs::File::open(ways_path)?;
    let mut ways = BufReader::new(ways_file);
    loop {
        let mut hdr = [0u8; 4];
        if ways.read_exact(&mut hdr).is_err() {
            break;
        }
        let nrefs = u32::from_le_bytes(hdr) as usize;
        let mut wid = [0u8; 8];
        ways.read_exact(&mut wid)?;
        let way_id = u64::from_le_bytes(wid);
        let mut meta = [0u8; 18];
        ways.read_exact(&mut meta)?;
        let class = meta[0];
        let flags = meta[1];
        let fwd_eff = u32::from_le_bytes([meta[2], meta[3], meta[4], meta[5]]) as f64 / 10.0;
        let bwd_eff = u32::from_le_bytes([meta[6], meta[7], meta[8], meta[9]]) as f64 / 10.0;
        let fwd = meta[10] != 0;
        let bwd = meta[11] != 0;
        let ferry_ms = u32::from_le_bytes([meta[12], meta[13], meta[14], meta[15]]);
        let name_len = u16::from_le_bytes([meta[16], meta[17]]) as usize;
        let mut name_buf = vec![0u8; name_len];
        ways.read_exact(&mut name_buf)?;
        let name = String::from_utf8_lossy(&name_buf).into_owned();
        let nidx = intern_name(&mut names, &mut name_ids, &name);

        let mut refs = Vec::with_capacity(nrefs);
        for _ in 0..nrefs {
            let mut b = [0u8; 8];
            ways.read_exact(&mut b)?;
            refs.push(u64::from_le_bytes(b));
        }

        let maxf = (fwd_eff * 10.0).round() as u32;
        if fwd {
            max_eff_x10 = max_eff_x10.max(maxf);
        }
        if bwd {
            max_eff_x10 = max_eff_x10.max((bwd_eff * 10.0).round() as u32);
        }

        let mut run_start = 0usize;
        let mut run_pts: Vec<(f64, f64)> = Vec::new();
        let mut i = 0usize;
        while i < refs.len() {
            let idx = ids.binary_search(&refs[i]).unwrap();
            let missing = lats[idx] == 0 && lons[idx] == 0;
            let is_j = junc[idx] || missing;
            run_pts.push((lats[idx] as f64 / 1e6, lons[idx] as f64 / 1e6));
            if is_j {
                if i > run_start {
                    emit_run(
                        &refs,
                        run_start,
                        i,
                        &ids,
                        &lats,
                        &lons,
                        &compact,
                        class,
                        flags,
                        fwd,
                        bwd,
                        fwd_eff,
                        bwd_eff,
                        ferry_ms,
                        nidx,
                        way_id,
                        max_piece_m,
                        &mut new_nodes,
                        compact_count,
                        &mut esrc,
                        &mut etgt,
                        &mut eweight,
                        &mut ename,
                        &mut ecls,
                        &mut eflags,
                        &mut egeom,
                        &mut egeomcnt,
                        &mut eway,
                        &mut geometry,
                    );
                    run_start = i;
                    run_pts = vec![(lats[idx] as f64 / 1e6, lons[idx] as f64 / 1e6)];
                } else {
                    run_start = i;
                    run_pts = vec![(lats[idx] as f64 / 1e6, lons[idx] as f64 / 1e6)];
                }
            }
            i += 1;
        }
        if run_pts.len() > 1 && run_start < refs.len() - 1 {
            emit_run(
                &refs,
                run_start,
                refs.len() - 1,
                &ids,
                &lats,
                &lons,
                &compact,
                class,
                flags,
                fwd,
                bwd,
                fwd_eff,
                bwd_eff,
                ferry_ms,
                nidx,
                way_id,
                max_piece_m,
                &mut new_nodes,
                compact_count,
                &mut esrc,
                &mut etgt,
                &mut eweight,
                &mut ename,
                &mut ecls,
                &mut eflags,
                &mut egeom,
                &mut egeomcnt,
                &mut eway,
                &mut geometry,
            );
        }
    }

    eprintln!("  {} directed edges, {} names", esrc.len(), names.len());

    let mut jlat: Vec<i32> = vec![0; compact_count as usize];
    let mut jlon: Vec<i32> = vec![0; compact_count as usize];
    for i in 0..node_count {
        let c = compact[i];
        if c != u32::MAX {
            jlat[c as usize] = lats[i];
            jlon[c as usize] = lons[i];
        }
    }
    for &(lat, lon) in &new_nodes {
        jlat.push(lat);
        jlon.push(lon);
    }

    apply_restrictions(
        &mut jlat,
        &mut jlon,
        &ids,
        &compact,
        restrictions,
        &mut esrc,
        &mut etgt,
        &mut eweight,
        &mut ename,
        &mut ecls,
        &mut eflags,
        &mut egeom,
        &mut egeomcnt,
        &mut eway,
    );

    write_output(
        &mut esrc, &mut etgt, &mut eweight, &mut ename, &mut ecls, &mut eflags, &mut egeom,
        &mut egeomcnt, &geometry, &mut names, &jlat, &jlon,
        max_eff_x10, out,
    )
}

#[allow(clippy::too_many_arguments)]
fn apply_restrictions(
    jlat: &mut Vec<i32>,
    jlon: &mut Vec<i32>,
    ids: &[u64],
    compact: &[u32],
    restrictions: &[Restriction],
    esrc: &mut Vec<u32>,
    etgt: &mut Vec<u32>,
    eweight: &mut Vec<u32>,
    ename: &mut Vec<u32>,
    ecls: &mut Vec<u8>,
    eflags: &mut Vec<u8>,
    egeom: &mut Vec<u32>,
    egeomcnt: &mut Vec<u16>,
    eway: &mut Vec<u64>,
) {
    let mut by_via: HashMap<u32, Vec<&Restriction>> = HashMap::new();
    for r in restrictions {
        if let Ok(idx) = ids.binary_search(&r.via_node) {
            let c = compact[idx];
            if c != u32::MAX {
                by_via.entry(c).or_default().push(r);
            }
        }
    }
    if by_via.is_empty() {
        return;
    }
    let initial_edges = esrc.len();
    let mut incoming: HashMap<u32, Vec<usize>> = HashMap::new();
    let mut outgoing: HashMap<u32, Vec<usize>> = HashMap::new();
    for i in 0..initial_edges {
        if by_via.contains_key(&esrc[i]) {
            outgoing.entry(esrc[i]).or_default().push(i);
        }
        if by_via.contains_key(&etgt[i]) {
            incoming.entry(etgt[i]).or_default().push(i);
        }
    }
    let mut copy_of: HashMap<(u32, u64), u32> = HashMap::new();
    for (&v, ins) in &incoming {
        let (lat, lon) = (jlat[v as usize], jlon[v as usize]);
        for &e in ins {
            let w = eway[e];
            let copy = *copy_of.entry((v, w)).or_insert_with(|| {
                let c = jlat.len() as u32;
                jlat.push(lat);
                jlon.push(lon);
                c
            });
            etgt[e] = copy;
        }
    }
    for (v, rs) in &by_via {
        let Some(ins) = incoming.get(v) else {
            continue;
        };
        if ins.is_empty() {
            continue;
        }
        let Some(outs) = outgoing.get(v) else {
            continue;
        };
        let mut ways: Vec<u64> = Vec::new();
        for &e in ins {
            let w = eway[e];
            if !ways.contains(&w) {
                ways.push(w);
            }
        }
        for &w in &ways {
            let copy = copy_of[&(*v, w)];
            let mut banned: Vec<u64> = Vec::new();
            let mut only: Option<Vec<u64>> = None;
            for r in rs {
                if r.from_way == w {
                    match r.kind {
                        RKind::NoTurn => banned.push(r.to_way),
                        RKind::Only => only.get_or_insert_with(Vec::new).push(r.to_way),
                    }
                }
            }
            for &e_out in outs {
                let out_way = eway[e_out];
                if banned.contains(&out_way) {
                    continue;
                }
                if let Some(ref onlys) = only {
                    if !onlys.contains(&out_way) {
                        continue;
                    }
                }
                esrc.push(copy);
                etgt.push(etgt[e_out]);
                eweight.push(eweight[e_out]);
                ename.push(ename[e_out]);
                ecls.push(ecls[e_out]);
                eflags.push(eflags[e_out]);
                egeom.push(egeom[e_out]);
                egeomcnt.push(egeomcnt[e_out]);
                eway.push(eway[e_out]);
            }
        }
        for &e_out in outs {
            esrc[e_out] = u32::MAX;
        }
    }
    let mut keep = 0usize;
    for i in 0..esrc.len() {
        if esrc[i] != u32::MAX {
            esrc[keep] = esrc[i];
            etgt[keep] = etgt[i];
            eweight[keep] = eweight[i];
            ename[keep] = ename[i];
            ecls[keep] = ecls[i];
            eflags[keep] = eflags[i];
            egeom[keep] = egeom[i];
            egeomcnt[keep] = egeomcnt[i];
            eway[keep] = eway[i];
            keep += 1;
        }
    }
    esrc.truncate(keep);
    etgt.truncate(keep);
    eweight.truncate(keep);
    ename.truncate(keep);
    ecls.truncate(keep);
    eflags.truncate(keep);
    egeom.truncate(keep);
    egeomcnt.truncate(keep);
    eway.truncate(keep);
}

fn seg_m(a: (i32, i32), b: (i32, i32)) -> f64 {
    flat_m(
        (a.0 as f64 / 1e6, a.1 as f64 / 1e6),
        (b.0 as f64 / 1e6, b.1 as f64 / 1e6),
    )
}

fn interpolate_pt(a: (i32, i32), b: (i32, i32), t: f64) -> (i32, i32) {
    (
        (a.0 as f64 + (b.0 as f64 - a.0 as f64) * t).round() as i32,
        (a.1 as f64 + (b.1 as f64 - a.1 as f64) * t).round() as i32,
    )
}

fn piece_limit_m(max_abs_lat_deg: f64) -> f64 {
    CELL_SIZE as f64 / 1e6 * 111320.0 * max_abs_lat_deg.to_radians().cos()
}

fn point_at(pts: &[(i32, i32)], cum: &[f64], d: f64) -> (i32, i32) {
    for i in 0..pts.len() - 1 {
        let (c0, c1) = (cum[i], cum[i + 1]);
        if d <= c1 || i == pts.len() - 2 {
            let t = if c1 > c0 { ((d - c0) / (c1 - c0)).clamp(0.0, 1.0) } else { 0.0 };
            return interpolate_pt(pts[i], pts[i + 1], t);
        }
    }
    pts[pts.len() - 1]
}

fn slice_run(pts: &[(i32, i32)], cum: &[f64], d0: f64, d1: f64) -> Vec<(i32, i32)> {
    let mut out = vec![point_at(pts, cum, d0)];
    for i in 1..pts.len() {
        if cum[i] > d0 && cum[i] < d1 {
            out.push(pts[i]);
        }
    }
    out.push(point_at(pts, cum, d1));
    out
}

fn split_run(pts: &[(i32, i32)], max_m: f64) -> Vec<Vec<(i32, i32)>> {
    if pts.len() < 2 {
        return vec![pts.to_vec()];
    }
    let mut cum = vec![0.0f64; pts.len()];
    for i in 1..pts.len() {
        cum[i] = cum[i - 1] + seg_m(pts[i - 1], pts[i]);
    }
    let total = cum[pts.len() - 1];
    if total <= max_m {
        return vec![pts.to_vec()];
    }
    let k = (total / max_m).ceil() as usize;
    (0..k)
        .map(|pi| {
            let d0 = pi as f64 * max_m;
            let d1 = ((pi + 1) as f64 * max_m).min(total);
            slice_run(pts, &cum, d0, d1)
        })
        .collect()
}

fn piece_len(piece: &[(i32, i32)]) -> f64 {
    piece.windows(2).map(|w| seg_m(w[0], w[1])).sum()
}

fn split_weight(total_w: u32, pieces: &[Vec<(i32, i32)>], total_len: f64) -> Vec<u32> {
    let k = pieces.len();
    if k == 1 {
        return vec![total_w];
    }
    let mut ws = Vec::with_capacity(k);
    let mut acc = 0u32;
    for j in 0..k - 1 {
        let w = (total_w as f64 * piece_len(&pieces[j]) / total_len).round() as u32;
        ws.push(w);
        acc += w;
    }
    ws.push(total_w.saturating_sub(acc));
    ws
}

#[allow(clippy::too_many_arguments)]
fn emit_run(
    refs: &[u64],
    a: usize,
    b: usize,
    ids: &[u64],
    lats: &[i32],
    lons: &[i32],
    compact: &[u32],
    class: u8,
    flags: u8,
    fwd: bool,
    bwd: bool,
    fwd_eff: f64,
    bwd_eff: f64,
    ferry_ms: u32,
    nidx: u32,
    way_id: u64,
    max_piece_m: f64,
    new_nodes: &mut Vec<(i32, i32)>,
    base: u32,
    esrc: &mut Vec<u32>,
    etgt: &mut Vec<u32>,
    eweight: &mut Vec<u32>,
    ename: &mut Vec<u32>,
    ecls: &mut Vec<u8>,
    eflags: &mut Vec<u8>,
    egeom: &mut Vec<u32>,
    egeomcnt: &mut Vec<u16>,
    eway: &mut Vec<u64>,
    geometry: &mut Vec<u8>,
) {
    let ia = ids.binary_search(&refs[a]).unwrap();
    let ib = ids.binary_search(&refs[b]).unwrap();
    let sa = compact[ia];
    let sb = compact[ib];
    if sa == u32::MAX || sb == u32::MAX {
        return;
    }
    let pts: Vec<(i32, i32)> = (a..=b)
        .map(|k| {
            let idx = ids.binary_search(&refs[k]).unwrap();
            (lats[idx], lons[idx])
        })
        .collect();
    let len_m = piece_len(&pts);
    let pieces = split_run(&pts, max_piece_m);
    let fwd_w = split_weight(edge_weight(len_m, fwd_eff, ferry_ms), &pieces, len_m);
    let bwd_w = split_weight(edge_weight(len_m, bwd_eff, ferry_ms), &pieces, len_m);
    let mut node_of = Vec::with_capacity(pieces.len() + 1);
    node_of.push(sa);
    for j in 1..pieces.len() {
        let (plat, plon) = pieces[j][0];
        let idx = base + new_nodes.len() as u32;
        new_nodes.push((plat, plon));
        node_of.push(idx);
    }
    node_of.push(sb);

    for (j, piece) in pieces.iter().enumerate() {
        let geom_off = push_geometry(geometry, piece);
        let geom_cnt = (piece.len().saturating_sub(2)) as u16;
        if fwd {
            esrc.push(node_of[j]);
            etgt.push(node_of[j + 1]);
            eweight.push(fwd_w[j]);
            ename.push(nidx);
            ecls.push(class);
            eflags.push(flags);
            egeom.push(geom_off);
            egeomcnt.push(geom_cnt);
            eway.push(way_id);
        }
        if bwd {
            let rev_pts: Vec<(i32, i32)> = piece.iter().rev().copied().collect();
            let rev_off = push_geometry(geometry, &rev_pts);
            esrc.push(node_of[j + 1]);
            etgt.push(node_of[j]);
            eweight.push(bwd_w[j]);
            ename.push(nidx);
            ecls.push(class);
            eflags.push(flags);
            egeom.push(rev_off);
            egeomcnt.push(geom_cnt);
            eway.push(way_id);
        }
    }
}

fn edge_weight(len_m: f64, eff_kmh: f64, ferry_ms: u32) -> u32 {
    if ferry_ms > 0 {
        return ferry_ms;
    }
    (len_m * 3600.0 / eff_kmh).round() as u32
}

#[allow(clippy::too_many_arguments)]
fn write_output(
    esrc: &mut Vec<u32>,
    etgt: &mut Vec<u32>,
    eweight: &mut Vec<u32>,
    ename: &mut Vec<u32>,
    ecls: &mut Vec<u8>,
    eflags: &mut Vec<u8>,
    egeom: &mut Vec<u32>,
    egeomcnt: &mut Vec<u16>,
    geometry: &[u8],
    names: &mut Vec<Vec<u8>>,
    jlat: &[i32],
    jlon: &[i32],
    max_eff_x10: u32,
    out: &Path,
) -> std::io::Result<()> {
    let node_count = jlat.len();
    let min_lat = jlat.iter().copied().min().unwrap_or(0);
    let max_lat = jlat.iter().copied().max().unwrap_or(0);
    let min_lon = jlon.iter().copied().min().unwrap_or(0);
    let max_lon = jlon.iter().copied().max().unwrap_or(0);
    let cell_size = CELL_SIZE;
    let pad = cell_size as i64;
    let min_lat_p = min_lat as i64 - pad;
    let max_lat_p = max_lat as i64 + pad;
    let min_lon_p = min_lon as i64 - pad;
    let max_lon_p = max_lon as i64 + pad;
    let grid_cols = (((max_lon_p - min_lon_p) / cell_size as i64 + 1) as u32).max(1);
    let grid_rows = (((max_lat_p - min_lat_p) / cell_size as i64 + 1) as u32).max(1);
    let grid = Grid {
        cols: grid_cols,
        rows: grid_rows,
        min_lat: min_lat_p as i32,
        min_lon: min_lon_p as i32,
        cell_size,
    };
    let (perm, cell_first) = order_by_cell(&jlat, &jlon, &grid);
    let mut nlat = vec![0i32; node_count as usize];
    let mut nlon = vec![0i32; node_count as usize];
    for i in 0..node_count as usize {
        nlat[perm[i] as usize] = jlat[i];
        nlon[perm[i] as usize] = jlon[i];
    }
    for e in esrc.iter_mut() {
        *e = perm[*e as usize];
    }
    for e in etgt.iter_mut() {
        *e = perm[*e as usize];
    }
    let nn = node_count as usize;
    let mut degree = vec![0u32; nn];
    for &s in esrc.iter() {
        degree[s as usize] += 1;
    }
    let mut first_edge = vec![0u32; nn];
    let mut acc = 0u32;
    for i in 0..nn {
        first_edge[i] = acc;
        acc += degree[i];
    }
    let mut next = first_edge.clone();
    let ne = esrc.len();
    let mut s_src = vec![0u32; ne];
    let mut s_tgt = vec![0u32; ne];
    let mut s_w = vec![0u32; ne];
    let mut s_n = vec![0u32; ne];
    let mut s_c = vec![0u8; ne];
    let mut s_f = vec![0u8; ne];
    let mut s_g = vec![0u32; ne];
    let mut s_gc = vec![0u16; ne];
    for i in 0..ne {
        let slot = next[esrc[i] as usize] as usize;
        next[esrc[i] as usize] += 1;
        s_src[slot] = esrc[i];
        s_tgt[slot] = etgt[i];
        s_w[slot] = eweight[i];
        s_n[slot] = ename[i];
        s_c[slot] = ecls[i];
        s_f[slot] = eflags[i];
        s_g[slot] = egeom[i];
        s_gc[slot] = egeomcnt[i];
    }

    let mut w = Writer::new();
    w.node_lat = nlat;
    w.node_lon = nlon;
    w.first_edge = first_edge;
    w.degree = degree;
    w.edge_source = s_src;
    w.edge_target = s_tgt;
    w.edge_weight_ms = s_w;
    w.edge_name = s_n;
    w.edge_class = s_c;
    w.edge_flags = s_f;
    w.edge_geom = s_g;
    w.edge_geom_count = s_gc;
    w.geometry = geometry.to_vec();
    w.names = names.clone();
    w.cell_first = cell_first;
    w.grid_cols = grid_cols;
    w.grid_rows = grid_rows;
    w.cell_size = cell_size;
    w.min_lat = min_lat_p as i32;
    w.min_lon = min_lon_p as i32;
    w.max_lat = max_lat_p as i32;
    w.max_lon = max_lon_p as i32;
    w.max_speed_x10 = max_eff_x10.max(1);
    w.write(out).map_err(|e| std::io::Error::other(e))?;
    Ok(())
}

fn intern_name(names: &mut Vec<Vec<u8>>, ids: &mut HashMap<String, u32>, name: &str) -> u32 {
    if name.is_empty() {
        return 0;
    }
    *ids.entry(name.to_string()).or_insert_with(|| {
        names.push(name.as_bytes().to_vec());
        (names.len() - 1) as u32
    })
}

#[cfg(test)]
mod oneway_tests {
    use super::*;

    fn ferry_ms(duration: &str) -> Option<u32> {
        classify(1, &Tags::collect([("route", "ferry"), ("motor_vehicle", "yes"), ("duration", duration)].into_iter())).unwrap().ferry_ms
    }

    #[test]
    fn ferry_durations_are_read_like_osrm_and_stored_in_milliseconds() {
        assert_eq!(ferry_ms("01:15"), Some(4_500_000));
        assert_eq!(ferry_ms("1:15:30"), Some(4_530_000));
        assert_eq!(ferry_ms("45"), Some(2_700_000));
        assert_eq!(ferry_ms("PT1H20M"), Some(4_800_000));
        assert_eq!(ferry_ms("PT45M"), Some(2_700_000));
        assert_eq!(ferry_ms("soon"), None);
    }

    fn dirs(tags: &[(&str, &str)]) -> (bool, bool) {
        let c = classify(1, &Tags::collect(tags.iter().copied())).unwrap();
        (c.fwd, c.bwd)
    }

    #[test]
    fn an_unnamed_road_never_borrows_another_road_name() {
        let mut names: Vec<Vec<u8>> = vec![Vec::new()];
        let mut ids = HashMap::new();
        let a = intern_name(&mut names, &mut ids, "Jurgella Lane");
        let none = intern_name(&mut names, &mut ids, "");
        let b = intern_name(&mut names, &mut ids, "Main Street");
        assert_eq!(names[none as usize], b"".to_vec());
        assert_eq!(names[a as usize], b"Jurgella Lane".to_vec());
        assert_eq!(names[b as usize], b"Main Street".to_vec());
        assert_eq!(intern_name(&mut names, &mut ids, "Jurgella Lane"), a);
    }

    #[test]
    fn an_impossible_maxspeed_tag_is_ignored() {
        let plain = classify(1, &Tags::collect([("highway", "primary")].into_iter())).unwrap();
        let typo = classify(1, &Tags::collect([("highway", "primary"), ("maxspeed", "3250")].into_iter())).unwrap();
        let real = classify(1, &Tags::collect([("highway", "primary"), ("maxspeed", "65 mph")].into_iter())).unwrap();
        assert_eq!(typo.fwd_eff, plain.fwd_eff);
        assert!(real.fwd_eff != plain.fwd_eff);
    }

    #[test]
    fn oneway_follows_the_way_node_order_like_osm() {
        assert_eq!(dirs(&[("highway", "residential")]), (true, true));
        assert_eq!(dirs(&[("highway", "residential"), ("oneway", "yes")]), (true, false));
        assert_eq!(dirs(&[("highway", "residential"), ("oneway", "-1")]), (false, true));
        assert_eq!(dirs(&[("highway", "motorway")]), (true, false));
        assert_eq!(dirs(&[("highway", "motorway"), ("oneway", "no")]), (true, true));
        assert_eq!(dirs(&[("highway", "primary"), ("junction", "roundabout")]), (true, false));
    }

    #[test]
    fn restricted_access_roads_stay_routable_like_car_lua() {
        let kept = |tag: &str, val: &str| {
            classify(1, &Tags::collect([("highway", "service"), (tag, val)].iter().copied())).is_some()
        };
        assert!(kept("access", "customers"));
        assert!(kept("access", "destination"));
        assert!(kept("access", "delivery"));
        assert!(kept("access", "permit"));
        assert!(kept("access", "residents"));
        assert!(!kept("access", "private"));
        assert!(!kept("access", "no"));
        assert!(!kept("motor_vehicle", "agricultural"));
        assert!(!kept("motorcar", "psv"));
    }

    #[test]
    fn restricted_access_sets_the_restricted_flag() {
        let flags = |tags: &[(&str, &str)]| {
            classify(1, &Tags::collect(tags.iter().copied())).unwrap().flags
        };
        let restricted = |val: &str| {
            flags(&[("highway", "service"), ("access", val)]) & alexandria_roads::F_RESTRICTED != 0
        };
        assert!(restricted("destination"));
        assert!(restricted("customers"));
        assert!(restricted("delivery"));
        assert!(restricted("permit"));
        assert!(restricted("residents"));
        assert!(restricted("unknown"));
        assert!(flags(&[("highway", "service"), ("motorcar", "destination")])
            & alexandria_roads::F_RESTRICTED != 0);
        assert!(flags(&[("highway", "service"), ("motor_vehicle", "customers")])
            & alexandria_roads::F_RESTRICTED != 0);
        assert!(flags(&[("highway", "service"), ("vehicle", "delivery")])
            & alexandria_roads::F_RESTRICTED != 0);
        assert!(flags(&[("highway", "residential")]) & alexandria_roads::F_RESTRICTED == 0);
        assert!(flags(&[("highway", "residential"), ("access", "yes")])
            & alexandria_roads::F_RESTRICTED == 0);
    }
}

#[cfg(test)]
mod restriction_tests {
    use super::*;

    fn reach(esrc: &[u32], etgt: &[u32], start: usize) -> Vec<bool> {
        let mut seen = vec![false; 64];
        let mut stack = vec![start as u32];
        while let Some(u) = stack.pop() {
            if seen[u as usize] {
                continue;
            }
            seen[u as usize] = true;
            for i in 0..esrc.len() {
                if esrc[i] == u {
                    stack.push(etgt[i]);
                }
            }
        }
        seen
    }

    fn split(restrictions: &[Restriction]) -> (Vec<u32>, Vec<u32>) {
        let ids: Vec<u64> = (0..8).collect();
        let compact: Vec<u32> = (0..8).collect();
        let mut jlat: Vec<i32> = vec![0; 8];
        let mut jlon: Vec<i32> = vec![0; 8];
        let mut esrc: Vec<u32> = vec![];
        let mut etgt: Vec<u32> = vec![];
        let mut eweight: Vec<u32> = vec![];
        let mut ename: Vec<u32> = vec![];
        let mut ecls: Vec<u8> = vec![];
        let mut eflags: Vec<u8> = vec![];
        let mut egeom: Vec<u32> = vec![];
        let mut egeomcnt: Vec<u16> = vec![];
        let mut eway: Vec<u64> = vec![];
        for &(s, t, w) in &[
            (0, 1, 100), (1, 0, 100), (2, 0, 200), (0, 2, 200),
            (1, 3, 300), (3, 1, 300), (4, 0, 400), (0, 4, 400),
            (0, 5, 500), (5, 0, 500), (6, 1, 600), (1, 7, 700),
        ] {
            esrc.push(s);
            etgt.push(t);
            eweight.push(1);
            ename.push(0);
            ecls.push(0);
            eflags.push(0);
            egeom.push(0);
            egeomcnt.push(0);
            eway.push(w);
        }
        apply_restrictions(
            &mut jlat, &mut jlon, &ids, &compact, restrictions,
            &mut esrc, &mut etgt, &mut eweight, &mut ename, &mut ecls, &mut eflags,
            &mut egeom, &mut egeomcnt, &mut eway,
        );
        (esrc, etgt)
    }

    #[test]
    fn adjacent_via_nodes_keep_through_traffic() {
        let r = |via, from, to, kind| Restriction {
            via_node: via,
            from_way: from,
            to_way: to,
            kind,
        };
        let restrictions = vec![
            r(0, 100, 400, RKind::NoTurn),
            r(0, 100, 500, RKind::NoTurn),
            r(0, 200, 400, RKind::NoTurn),
            r(0, 200, 500, RKind::NoTurn),
            r(0, 500, 200, RKind::NoTurn),
            r(0, 500, 100, RKind::NoTurn),
            r(0, 400, 200, RKind::NoTurn),
            r(0, 400, 100, RKind::NoTurn),
            r(1, 600, 300, RKind::NoTurn),
            r(1, 300, 700, RKind::NoTurn),
        ];
        let (esrc, etgt) = split(&restrictions);
        let seen = reach(&esrc, &etgt, 2);
        assert!(seen[3], "southbound through both intersections must reach north");
        let seen = reach(&esrc, &etgt, 3);
        assert!(seen[2], "northbound through both intersections must reach south");
    }

    #[test]
    fn banned_turn_stays_banned() {
        let ids: Vec<u64> = (0..4).collect();
        let compact: Vec<u32> = (0..4).collect();
        let mut jlat: Vec<i32> = vec![0, 0, 0, 0];
        let mut jlon: Vec<i32> = vec![0, 0, 0, 0];
        let mut esrc: Vec<u32> = vec![];
        let mut etgt: Vec<u32> = vec![];
        let mut eweight: Vec<u32> = vec![];
        let mut ename: Vec<u32> = vec![];
        let mut ecls: Vec<u8> = vec![];
        let mut eflags: Vec<u8> = vec![];
        let mut egeom: Vec<u32> = vec![];
        let mut egeomcnt: Vec<u16> = vec![];
        let mut eway: Vec<u64> = vec![];
        for &(s, t, w) in &[(1, 0, 100), (0, 1, 100), (2, 0, 200), (0, 2, 200), (0, 3, 300), (3, 0, 300)] {
            esrc.push(s);
            etgt.push(t);
            eweight.push(1);
            ename.push(0);
            ecls.push(0);
            eflags.push(0);
            egeom.push(0);
            egeomcnt.push(0);
            eway.push(w);
        }
        let r = vec![Restriction { via_node: 0, from_way: 100, to_way: 200, kind: RKind::NoTurn }];
        apply_restrictions(
            &mut jlat, &mut jlon, &ids, &compact, &r,
            &mut esrc, &mut etgt, &mut eweight, &mut ename, &mut ecls, &mut eflags,
            &mut egeom, &mut egeomcnt, &mut eway,
        );
        let mut from1: Vec<u32> = vec![];
        for i in 0..esrc.len() {
            if esrc[i] == 1 {
                from1.push(etgt[i]);
            }
        }
        assert!(!from1.is_empty());
        let mut heads: Vec<u32> = vec![];
        for &c in &from1 {
            for i in 0..esrc.len() {
                if esrc[i] == c {
                    heads.push(etgt[i]);
                }
            }
        }
        assert!(heads.contains(&3), "way A may turn onto C");
        assert!(!heads.contains(&2), "way A may not turn onto B");
    }
}

#[cfg(test)]
mod split_tests {
    use super::*;
    use alexandria_core::{write_hashes, Entry, VerifiedFile, CHUNK};
    use alexandria_roads::{C_MOTORWAY_LINK, C_PRIMARY, Roads};
    use sha2::{Digest, Sha256};

    fn pt(lat: f64, lon: f64) -> (i32, i32) {
        ((lat * 1e6).round() as i32, (lon * 1e6).round() as i32)
    }

    struct WaySpec {
        id: u64,
        name: String,
        class: u8,
        flags: u8,
        fwd: bool,
        bwd: bool,
        fwd_eff: f64,
        bwd_eff: f64,
        refs: Vec<u64>,
    }

    fn write_ways(path: &Path, ways: &[WaySpec]) {
        let mut w = BufWriter::new(fs::File::create(path).unwrap());
        for way in ways {
            w.write_all(&(way.refs.len() as u32).to_le_bytes()).unwrap();
            w.write_all(&way.id.to_le_bytes()).unwrap();
            w.write_all(&[way.class, way.flags]).unwrap();
            w.write_all(&((way.fwd_eff * 10.0).round() as u32).to_le_bytes()).unwrap();
            w.write_all(&((way.bwd_eff * 10.0).round() as u32).to_le_bytes()).unwrap();
            w.write_all(&[way.fwd as u8, way.bwd as u8]).unwrap();
            w.write_all(&0u32.to_le_bytes()).unwrap();
            w.write_all(&(way.name.len() as u16).to_le_bytes()).unwrap();
            w.write_all(way.name.as_bytes()).unwrap();
            for &r in &way.refs {
                w.write_all(&r.to_le_bytes()).unwrap();
            }
        }
        w.flush().unwrap();
    }

    fn write_nodes(path: &Path, nodes: &[(u64, f64, f64, bool)]) {
        let mut w = BufWriter::new(fs::File::create(path).unwrap());
        for &(id, lat, lon, junc) in nodes {
            w.write_all(&id.to_le_bytes()).unwrap();
            w.write_all(&((lat * 1e6).round() as i32).to_le_bytes()).unwrap();
            w.write_all(&((lon * 1e6).round() as i32).to_le_bytes()).unwrap();
            w.write_all(&[junc as u8]).unwrap();
        }
        w.flush().unwrap();
    }

    fn build_roads(ways: &[WaySpec], nodes: &[(u64, f64, f64, bool)]) -> Roads {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "alex-cli-split-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        let ways_path = dir.join("ways.bin");
        let nodes_path = dir.join("nodesxy.bin");
        let out = dir.join("out.roads");
        write_ways(&ways_path, ways);
        write_nodes(&nodes_path, nodes);
        build_graph(&ways_path, &nodes_path, &out, &[]).unwrap();
        let size = fs::metadata(&out).unwrap().len();
        let hashes = write_hashes(&out, CHUNK).unwrap();
        let raw = fs::read(&hashes).unwrap();
        let mut h = [0u8; 32];
        h.copy_from_slice(&Sha256::digest(&raw));
        let entry = Entry {
            hashes_sha256: h,
            size,
            path: "roads".into(),
        };
        let vf = VerifiedFile::open(&out, &entry, CHUNK).unwrap();
        let r = Roads::open(vf).unwrap();
        fs::remove_dir_all(&dir).ok();
        r
    }

    fn long_highway() -> (Vec<WaySpec>, Vec<(u64, f64, f64, bool)>) {
        let nodes = vec![
            (1u64, 46.0, -102.60, true),
            (2u64, 46.0, -102.58, false),
            (3u64, 46.0, -102.56, false),
            (4u64, 46.0, -102.54, false),
            (5u64, 46.0, -102.40, true),
            (6u64, 46.005, -102.398, true),
        ];
        let ways = vec![
            WaySpec {
                id: 100,
                name: "Highway".into(),
                class: C_PRIMARY,
                flags: 0,
                fwd: true,
                bwd: false,
                fwd_eff: 65.0,
                bwd_eff: 65.0,
                refs: vec![1, 2, 3, 4, 5],
            },
            WaySpec {
                id: 200,
                name: "Exit 42".into(),
                class: C_MOTORWAY_LINK,
                flags: 0,
                fwd: true,
                bwd: false,
                fwd_eff: 30.0,
                bwd_eff: 30.0,
                refs: vec![5, 6],
            },
        ];
        (ways, nodes)
    }

    fn long_highway_only() -> (Vec<WaySpec>, Vec<(u64, f64, f64, bool)>) {
        let nodes = vec![
            (1u64, 46.0, -102.60, true),
            (2u64, 46.0, -102.58, false),
            (3u64, 46.0, -102.56, false),
            (4u64, 46.0, -102.54, false),
            (5u64, 46.0, -102.40, true),
        ];
        let ways = vec![WaySpec {
            id: 100,
            name: "Highway".into(),
            class: C_PRIMARY,
            flags: 0,
            fwd: true,
            bwd: false,
            fwd_eff: 65.0,
            bwd_eff: 65.0,
            refs: vec![1, 2, 3, 4, 5],
        }];
        (ways, nodes)
    }

    fn restricted_junction_roads() -> Roads {
        let nodes = vec![
            (1u64, 46.0, -102.62, true),
            (2u64, 46.0, -102.60, true),
            (3u64, 46.0, -102.56, true),
            (4u64, 45.99, -102.60, true),
            (5u64, 46.0, -102.58, true),
            (6u64, 46.001, -102.58, true),
        ];
        let way = |id: u64, name: &str, refs: Vec<u64>| WaySpec {
            id,
            name: name.into(),
            class: C_PRIMARY,
            flags: 0,
            fwd: true,
            bwd: true,
            fwd_eff: 50.0,
            bwd_eff: 50.0,
            refs,
        };
        let ways = vec![way(400, "Main West", vec![1, 2]), way(401, "Main East", vec![2, 5, 3]), way(300, "Side St", vec![4, 2]), way(500, "Lane", vec![5, 6])];
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "alex-cli-uturn-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        let restrictions = vec![Restriction { via_node: 2, from_way: 300, to_way: 401, kind: RKind::Only }];
        let ways_path = dir.join("ways.bin");
        let nodes_path = dir.join("nodesxy.bin");
        let out = dir.join("out.roads");
        write_ways(&ways_path, &ways);
        write_nodes(&nodes_path, &nodes);
        build_graph(&ways_path, &nodes_path, &out, &restrictions).unwrap();
        let size = fs::metadata(&out).unwrap().len();
        let hashes = write_hashes(&out, CHUNK).unwrap();
        let raw = fs::read(&hashes).unwrap();
        let mut h = [0u8; 32];
        h.copy_from_slice(&Sha256::digest(&raw));
        let entry = Entry { hashes_sha256: h, size, path: "roads".into() };
        let vf = VerifiedFile::open(&out, &entry, CHUNK).unwrap();
        let r = Roads::open(vf).unwrap();
        fs::remove_dir_all(&dir).ok();
        r
    }

    #[test]
    fn a_start_beside_a_restricted_junction_drives_straight_to_it() {
        let mut r = restricted_junction_roads();
        let route = r.route((45.995, -102.60), (46.0, -102.61)).unwrap();
        assert!(
            route.distance_m < 5000.0,
            "the start is 556 m south of the junction on a two-way street, so the route must head north at once (about 4.6 km in all, turning around at the Lane dead end), not drive south to the end and back; got {} m",
            route.distance_m
        );
    }

    #[test]
    fn a_uturn_dodging_a_restriction_goes_to_a_dead_end_instead() {
        let mut r = restricted_junction_roads();
        let route = r.route((45.995, -102.60), (46.0, -102.61)).unwrap();
        assert!(
            (route.distance_m - 4641.0).abs() < 60.0,
            "the only legal U-turn is at the Lane dead end: Side St 556 m, Main East 1545 m, Lane 111 m, back 111 m, Main East 1545 m, Main West 773 m; got {} m",
            route.distance_m
        );
        for s in &route.steps {
            if s.text.contains("U-turn") {
                assert!(
                    (s.at[0] - -102.58).abs() < 1e-6 && (s.at[1] - 46.001).abs() < 1e-6,
                    "U-turn at {:?} is not at the Lane dead end",
                    s.at
                );
            }
        }
        assert!(
            route.steps.iter().any(|s| s.text.contains("U-turn")),
            "expected the turnaround at the Lane dead end, got {:?}",
            route.steps.iter().map(|s| s.text.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn split_run_never_exceeds_the_piece_limit() {
        let limit = piece_limit_m(46.0);
        let pts: Vec<(i32, i32)> = (0..11)
            .map(|i| pt(46.0, -102.60 + i as f64 * 0.02))
            .collect();
        let pieces = split_run(&pts, limit);
        assert!(pieces.len() > 1);
        for piece in &pieces {
            assert!(piece.len() >= 2);
            assert!(piece_len(piece) <= limit + 1.0, "piece {} m over limit", piece_len(piece));
        }
    }

    #[test]
    fn split_run_leaves_a_short_run_whole() {
        let limit = piece_limit_m(46.0);
        let pts = vec![pt(46.0, -102.60), pt(46.0, -102.59)];
        let pieces = split_run(&pts, limit);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0], pts);
    }

    #[test]
    fn split_run_interpolates_a_long_straight_segment() {
        let limit = piece_limit_m(46.0);
        let a = pt(46.0, -102.60);
        let b = pt(46.0, -102.40);
        let pieces = split_run(&[a, b], limit);
        assert!(pieces.len() > 1);
        let first = pieces[0][0];
        let last = pieces[pieces.len() - 1][pieces[pieces.len() - 1].len() - 1];
        assert_eq!(first, a);
        assert_eq!(last, b);
        for piece in &pieces {
            assert!(piece_len(piece) <= limit + 1.0);
        }
        let total: f64 = pieces.iter().map(|p| piece_len(p)).sum();
        assert!((total - seg_m(a, b)).abs() < 2.0);
    }

    #[test]
    fn split_weight_sums_to_the_total() {
        let limit = piece_limit_m(46.0);
        let pts: Vec<(i32, i32)> = (0..11)
            .map(|i| pt(46.0, -102.60 + i as f64 * 0.02))
            .collect();
        let pieces = split_run(&pts, limit);
        let total = piece_len(&pts);
        let ws = split_weight(1000, &pieces, total);
        assert_eq!(ws.len(), pieces.len());
        assert_eq!(ws.iter().sum::<u32>(), 1000);
    }

    #[test]
    fn route_from_just_before_an_exit_is_short() {
        let (ways, nodes) = long_highway();
        let mut r = build_roads(&ways, &nodes);
        let route = r.route((46.0, -102.45), (46.0025, -102.399)).unwrap();
        assert!(route.distance_m < 9000.0, "got {} m", route.distance_m);
    }

    #[test]
    fn destination_on_a_ramp_off_a_long_oneway_is_reachable() {
        let (ways, nodes) = long_highway();
        let mut r = build_roads(&ways, &nodes);
        let route = r.route((46.0, -102.59), (46.0025, -102.399)).unwrap();
        assert!(route.distance_m > 14000.0 && route.distance_m < 16000.0, "got {} m", route.distance_m);
    }

    #[test]
    fn splitting_keeps_a_long_road_route_intact() {
        let (ways, nodes) = long_highway_only();
        let mut r = build_roads(&ways, &nodes);
        let route = r.route((46.0, -102.595), (46.0, -102.405)).unwrap();
        assert!(route.distance_m > 14000.0, "got {} m", route.distance_m);
        let texts: Vec<&str> = route.steps.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, vec!["Head out on Highway", "Arrive at destination"]);
    }
}
