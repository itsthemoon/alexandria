use alexandria_core::{write_hashes, Entry, VerifiedFile};
use alexandria_roads::{order_by_cell, Grid, Roads, Writer, C_RESIDENTIAL};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

const CH: u64 = 4096;

struct Seg {
    a: (f64, f64),
    b: (f64, f64),
    oneway: bool,
    name: &'static str,
    flags: u8,
}

fn deg(v: f64) -> i32 {
    (v * 1e6).round() as i32
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

fn build(segs: &[Seg]) -> Writer {
    let mut nodes: Vec<(i32, i32)> = Vec::new();
    let mut ids: HashMap<(i32, i32), u32> = HashMap::new();
    let mut node = |c: (f64, f64)| -> u32 {
        let k = (deg(c.0), deg(c.1));
        *ids.entry(k).or_insert_with(|| {
            nodes.push(k);
            (nodes.len() - 1) as u32
        })
    };
    let mut names: Vec<Vec<u8>> = Vec::new();
    let mut name_ids: HashMap<&'static str, u32> = HashMap::new();
    let mut intern = |n: &'static str| -> u32 {
        *name_ids.entry(n).or_insert_with(|| {
            names.push(n.as_bytes().to_vec());
            (names.len() - 1) as u32
        })
    };

    let mut esrc = Vec::new();
    let mut etgt = Vec::new();
    let mut eweight = Vec::new();
    let mut ename = Vec::new();
    let mut ecls = Vec::new();
    let mut eflags = Vec::new();
    for s in segs {
        let a = node(s.a);
        let b = node(s.b);
        let w = (hav_m(s.a, s.b) * 144.0).round() as u32;
        let n = intern(s.name);
        let fl = s.flags;
        let mut push = |a: u32, b: u32| {
            esrc.push(a);
            etgt.push(b);
            eweight.push(w);
            ename.push(n);
            ecls.push(C_RESIDENTIAL);
            eflags.push(fl);
        };
        push(a, b);
        if !s.oneway {
            push(b, a);
        }
    }

    let min_lat = nodes.iter().map(|n| n.0).min().unwrap();
    let max_lat = nodes.iter().map(|n| n.0).max().unwrap();
    let min_lon = nodes.iter().map(|n| n.1).min().unwrap();
    let max_lon = nodes.iter().map(|n| n.1).max().unwrap();
    let cell_size = 10000u32;
    let grid_cols = (((max_lon - min_lon) as u32) / cell_size + 1).max(1);
    let grid_rows = (((max_lat - min_lat) as u32) / cell_size + 1).max(1);
    let grid = Grid {
        cols: grid_cols,
        rows: grid_rows,
        min_lat,
        min_lon,
        cell_size,
    };
    let lat: Vec<i32> = nodes.iter().map(|n| n.0).collect();
    let lon: Vec<i32> = nodes.iter().map(|n| n.1).collect();
    let (perm, cell_first) = order_by_cell(&lat, &lon, &grid);

    let nn = nodes.len();
    let mut nlat = vec![0i32; nn];
    let mut nlon = vec![0i32; nn];
    for i in 0..nn {
        nlat[perm[i] as usize] = lat[i];
        nlon[perm[i] as usize] = lon[i];
    }
    for e in esrc.iter_mut() {
        *e = perm[*e as usize];
    }
    for e in etgt.iter_mut() {
        *e = perm[*e as usize];
    }
    let mut degree = vec![0u32; nn];
    for &s in &esrc {
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
    for i in 0..ne {
        let slot = next[esrc[i] as usize] as usize;
        next[esrc[i] as usize] += 1;
        s_src[slot] = esrc[i];
        s_tgt[slot] = etgt[i];
        s_w[slot] = eweight[i];
        s_n[slot] = ename[i];
        s_c[slot] = ecls[i];
        s_f[slot] = eflags[i];
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
    w.edge_geom = vec![0u32; ne];
    w.edge_geom_count = vec![0u16; ne];
    w.names = names;
    w.cell_first = cell_first;
    w.grid_cols = grid_cols;
    w.grid_rows = grid_rows;
    w.cell_size = cell_size;
    w.min_lat = min_lat;
    w.min_lon = min_lon;
    w.max_lat = max_lat;
    w.max_lon = max_lon;
    w.max_speed_x10 = 250;
    w
}

fn write_roads(dir: &Path, w: &Writer) -> Entry {
    let path = dir.join("graph.roads");
    w.write(&path).unwrap();
    let size = fs::metadata(&path).unwrap().len();
    let hashes = write_hashes(&path, CH).unwrap();
    let raw = fs::read(&hashes).unwrap();
    let mut h = [0u8; 32];
    h.copy_from_slice(&Sha256::digest(&raw));
    Entry {
        hashes_sha256: h,
        size,
        path: "graph.roads".into(),
    }
}

fn open(dir: &Path, entry: &Entry) -> Roads {
    let vf = VerifiedFile::open_in(dir, entry, CH).unwrap();
    Roads::open(vf).unwrap()
}

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("alex-roads-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn routes_across_a_square_and_back() {
    let segs = vec![
        Seg { a: (40.7000, -73.9000), b: (40.7010, -73.9000), oneway: false, name: "1st Ave", flags: 0 },
        Seg { a: (40.7000, -73.9000), b: (40.7000, -73.8990), oneway: false, name: "A St", flags: 0 },
        Seg { a: (40.7010, -73.9000), b: (40.7010, -73.8990), oneway: false, name: "B St", flags: 0 },
        Seg { a: (40.7000, -73.8990), b: (40.7010, -73.8990), oneway: false, name: "2nd Ave", flags: 0 },
    ];
    let d = tmpdir("square");
    let w = build(&segs);
    let entry = write_roads(&d, &w);
    let mut r = open(&d, &entry);
    let route = r.route((40.7000, -73.9000), (40.7010, -73.8990)).unwrap();
    assert!(route.distance_m > 0.0);
    assert!(!route.line.is_empty());
    assert!(!route.steps.is_empty());
    assert_eq!(route.steps.last().unwrap().text, "Arrive at destination");
    fs::remove_dir_all(&d).ok();
}

#[test]
fn oneway_blocks_the_direct_direction() {
    let segs = vec![
        Seg { a: (40.7000, -73.9000), b: (40.7010, -73.9000), oneway: true, name: "1st Ave", flags: 0 },
        Seg { a: (40.7000, -73.9000), b: (40.7000, -73.8990), oneway: false, name: "A St", flags: 0 },
        Seg { a: (40.7010, -73.9000), b: (40.7010, -73.8990), oneway: false, name: "B St", flags: 0 },
        Seg { a: (40.7000, -73.8990), b: (40.7010, -73.8990), oneway: false, name: "2nd Ave", flags: 0 },
    ];
    let d = tmpdir("oneway");
    let w = build(&segs);
    let entry = write_roads(&d, &w);
    let mut r = open(&d, &entry);
    let route = r.route((40.7010, -73.9000), (40.7000, -73.9000)).unwrap();
    assert!(route.distance_m > 200.0, "going the long way around a oneway: {}", route.distance_m);
    fs::remove_dir_all(&d).ok();
}

#[test]
fn a_trip_longer_than_the_limit_is_refused_plainly() {
    let segs = vec![
        Seg { a: (40.7000, -73.9000), b: (40.7010, -73.9000), oneway: false, name: "1st Ave", flags: 0 },
        Seg { a: (43.7000, -73.9000), b: (43.7010, -73.9000), oneway: false, name: "North St", flags: 0 },
    ];
    let d = tmpdir("toofar");
    let w = build(&segs);
    let entry = write_roads(&d, &w);
    let mut r = open(&d, &entry);
    let err = r.route((40.7000, -73.9000), (43.7000, -73.9000)).unwrap_err();
    assert!(err.contains("250 km"), "got: {err}");
    fs::remove_dir_all(&d).ok();
}

#[test]
fn unreachable_destination_is_an_error() {
    let segs = vec![
        Seg { a: (40.7000, -73.9000), b: (40.7010, -73.9000), oneway: false, name: "1st Ave", flags: 0 },
        Seg { a: (40.7200, -73.9200), b: (40.7210, -73.9200), oneway: false, name: "Island St", flags: 0 },
    ];
    let d = tmpdir("unreachable");
    let w = build(&segs);
    let entry = write_roads(&d, &w);
    let mut r = open(&d, &entry);
    let err = r.route((40.7000, -73.9000), (40.7200, -73.9200)).unwrap_err();
    assert!(err.contains("No route"), "got: {err}");
    fs::remove_dir_all(&d).ok();
}

#[test]
fn snapping_finds_a_nearby_edge() {
    let segs = vec![
        Seg { a: (40.7000, -73.9000), b: (40.7010, -73.9000), oneway: false, name: "1st Ave", flags: 0 },
    ];
    let d = tmpdir("snap");
    let w = build(&segs);
    let entry = write_roads(&d, &w);
    let mut r = open(&d, &entry);
    let route = r.route((40.7002, -73.9000), (40.7008, -73.9000)).unwrap();
    assert!(route.distance_m > 0.0);
    fs::remove_dir_all(&d).ok();
}

fn build_graph_with_flags(edges: &[((f64, f64), (f64, f64), u8, &'static str)]) -> Writer {
    let mut nodes: Vec<(i32, i32)> = Vec::new();
    let mut ids: HashMap<(i32, i32), u32> = HashMap::new();
    let mut node = |c: (f64, f64)| -> u32 {
        let k = (deg(c.0), deg(c.1));
        *ids.entry(k).or_insert_with(|| {
            nodes.push(k);
            (nodes.len() - 1) as u32
        })
    };
    let mut names: Vec<Vec<u8>> = Vec::new();
    let mut name_ids: HashMap<&'static str, u32> = HashMap::new();
    let mut esrc = Vec::new();
    let mut etgt = Vec::new();
    let mut eweight = Vec::new();
    let mut ename = Vec::new();
    let mut ecls = Vec::new();
    let mut eflags = Vec::new();
    for &(a, b, fl, nm) in edges {
        let na = node(a);
        let nb = node(b);
        let w = (hav_m(a, b) * 144.0).round() as u32;
        let n = *name_ids.entry(nm).or_insert_with(|| {
            names.push(nm.as_bytes().to_vec());
            (names.len() - 1) as u32
        });
        esrc.push(na);
        etgt.push(nb);
        eweight.push(w);
        ename.push(n);
        ecls.push(C_RESIDENTIAL);
        eflags.push(fl);
    }
    let min_lat = nodes.iter().map(|n| n.0).min().unwrap();
    let max_lat = nodes.iter().map(|n| n.0).max().unwrap();
    let min_lon = nodes.iter().map(|n| n.1).min().unwrap();
    let max_lon = nodes.iter().map(|n| n.1).max().unwrap();
    let cell_size = 10000u32;
    let grid_cols = (((max_lon - min_lon) as u32) / cell_size + 1).max(1);
    let grid_rows = (((max_lat - min_lat) as u32) / cell_size + 1).max(1);
    let grid = Grid { cols: grid_cols, rows: grid_rows, min_lat, min_lon, cell_size };
    let lat: Vec<i32> = nodes.iter().map(|n| n.0).collect();
    let lon: Vec<i32> = nodes.iter().map(|n| n.1).collect();
    let (perm, cell_first) = order_by_cell(&lat, &lon, &grid);
    let nn = nodes.len();
    let mut nlat = vec![0i32; nn];
    let mut nlon = vec![0i32; nn];
    for i in 0..nn {
        nlat[perm[i] as usize] = lat[i];
        nlon[perm[i] as usize] = lon[i];
    }
    for e in esrc.iter_mut() {
        *e = perm[*e as usize];
    }
    for e in etgt.iter_mut() {
        *e = perm[*e as usize];
    }
    let mut degree = vec![0u32; nn];
    for &s in &esrc {
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
    for i in 0..ne {
        let slot = next[esrc[i] as usize] as usize;
        next[esrc[i] as usize] += 1;
        s_src[slot] = esrc[i];
        s_tgt[slot] = etgt[i];
        s_w[slot] = eweight[i];
        s_n[slot] = ename[i];
        s_c[slot] = ecls[i];
        s_f[slot] = eflags[i];
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
    w.edge_geom = vec![0u32; ne];
    w.edge_geom_count = vec![0u16; ne];
    w.names = names;
    w.cell_first = cell_first;
    w.grid_cols = grid_cols;
    w.grid_rows = grid_rows;
    w.cell_size = cell_size;
    w.min_lat = min_lat;
    w.min_lon = min_lon;
    w.max_lat = max_lat;
    w.max_lon = max_lon;
    w.max_speed_x10 = 250;
    w
}

#[test]
fn roundabout_exit_is_numbered() {
    use alexandria_roads::F_ROUNDABOUT;
    let a = (40.7000, -73.9000);
    let b = (40.7000, -73.8990);
    let c = (40.7010, -73.8990);
    let d = (40.7010, -73.9000);
    let e = (40.6990, -73.9000);
    let f = (40.7015, -73.8990);
    let g = (40.7020, -73.8990);
    let h = (40.7030, -73.8990);
    let edges = vec![
        (a, b, F_ROUNDABOUT, "Roundabout"),
        (b, c, F_ROUNDABOUT, "Roundabout"),
        (c, d, F_ROUNDABOUT, "Roundabout"),
        (d, a, F_ROUNDABOUT, "Roundabout"),
        (e, a, 0, "Entry Road"),
        (a, e, 0, "Entry Road"),
        (b, f, 0, "First Exit"),
        (f, b, 0, "First Exit"),
        (c, g, 0, "Main Street"),
        (g, c, 0, "Main Street"),
        (g, h, 0, "Main Street"),
        (h, g, 0, "Main Street"),
    ];
    let d2 = tmpdir("roundabout");
    let w = build_graph_with_flags(&edges);
    let entry = write_roads(&d2, &w);
    let mut r = open(&d2, &entry);
    let route = r.route(e, h).unwrap();
    let texts: Vec<&str> = route.steps.iter().map(|s| s.text.as_str()).collect();
    assert!(
        texts.iter().any(|t| t.contains("roundabout") && t.contains("2nd exit")),
        "expected a numbered roundabout exit, got {texts:?}"
    );
    fs::remove_dir_all(&d2).ok();
}

#[test]
fn lazy_routes_match_eager_behaviour_on_square() {
    let segs = vec![
        Seg { a: (40.7000, -73.9000), b: (40.7010, -73.9000), oneway: false, name: "1st Ave", flags: 0 },
        Seg { a: (40.7000, -73.9000), b: (40.7000, -73.8990), oneway: false, name: "A St", flags: 0 },
        Seg { a: (40.7010, -73.9000), b: (40.7010, -73.8990), oneway: false, name: "B St", flags: 0 },
        Seg { a: (40.7000, -73.8990), b: (40.7010, -73.8990), oneway: false, name: "2nd Ave", flags: 0 },
    ];
    let d = tmpdir("square-eager");
    let w = build(&segs);
    let entry = write_roads(&d, &w);

    let mut a = open(&d, &entry);
    let ra = a.route((40.7000, -73.9000), (40.7010, -73.8990)).unwrap();
    let mut b = open(&d, &entry);
    let rb = b.route((40.7000, -73.9000), (40.7010, -73.8990)).unwrap();

    assert_eq!(ra.distance_m, rb.distance_m);
    assert_eq!(ra.duration_s, rb.duration_s);
    assert_eq!(ra.line, rb.line);
    let at: Vec<&[f64; 2]> = ra.steps.iter().map(|s| &s.at).collect();
    let bt: Vec<&[f64; 2]> = rb.steps.iter().map(|s| &s.at).collect();
    let atext: Vec<&str> = ra.steps.iter().map(|s| s.text.as_str()).collect();
    let btext: Vec<&str> = rb.steps.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(atext, btext);
    assert_eq!(at, bt);
    assert_eq!(ra.steps.len(), rb.steps.len());
    assert!(ra.distance_m > 0.0);
    assert_eq!(ra.steps.last().unwrap().text, "Arrive at destination");
    fs::remove_dir_all(&d).ok();
}

#[test]
fn restricted_street_is_not_a_through_shortcut() {
    use alexandria_roads::F_RESTRICTED;
    let segs = vec![
        Seg { a: (40.7000, -73.9000), b: (40.7010, -73.9000), oneway: false, name: "Main Rd", flags: 0 },
        Seg { a: (40.7010, -73.9000), b: (40.7020, -73.9000), oneway: false, name: "Main Rd", flags: 0 },
        Seg { a: (40.7020, -73.9000), b: (40.7020, -73.8980), oneway: false, name: "Main Rd", flags: 0 },
        Seg { a: (40.7010, -73.9000), b: (40.7020, -73.8980), oneway: false, name: "Cut Through", flags: F_RESTRICTED },
    ];
    let d = tmpdir("shortcut");
    let w = build(&segs);
    let entry = write_roads(&d, &w);
    let mut r = open(&d, &entry);
    let route = r.route((40.7002, -73.9000), (40.7020, -73.8982)).unwrap();
    assert!(route.distance_m > 350.0, "must detour around the restricted cut, got {} m", route.distance_m);
    fs::remove_dir_all(&d).ok();
}

#[test]
fn destination_on_restricted_street_is_reachable() {
    use alexandria_roads::F_RESTRICTED;
    let segs = vec![
        Seg { a: (40.7000, -73.9000), b: (40.7010, -73.9000), oneway: false, name: "Main St", flags: 0 },
        Seg { a: (40.7010, -73.9000), b: (40.7020, -73.9000), oneway: false, name: "Service Rd", flags: F_RESTRICTED },
    ];
    let d = tmpdir("restricted-dest");
    let w = build(&segs);
    let entry = write_roads(&d, &w);
    let mut r = open(&d, &entry);
    let route = r.route((40.7000, -73.9000), (40.7020, -73.9000)).unwrap();
    assert!(route.distance_m > 150.0, "got {} m", route.distance_m);
    fs::remove_dir_all(&d).ok();
}

#[test]
fn start_on_restricted_street_leaves_it() {
    use alexandria_roads::F_RESTRICTED;
    let segs = vec![
        Seg { a: (40.7020, -73.9000), b: (40.7010, -73.9000), oneway: false, name: "Service Rd", flags: F_RESTRICTED },
        Seg { a: (40.7010, -73.9000), b: (40.7010, -73.8990), oneway: false, name: "Main St", flags: 0 },
    ];
    let d = tmpdir("start-restricted");
    let w = build(&segs);
    let entry = write_roads(&d, &w);
    let mut r = open(&d, &entry);
    let route = r.route((40.7020, -73.9000), (40.7010, -73.8990)).unwrap();
    assert!(route.distance_m > 120.0, "got {} m", route.distance_m);
    fs::remove_dir_all(&d).ok();
}

fn snake_segs(out: &mut Vec<Seg>, lat: f64, lon0: f64, lon1: f64, n: usize, name: &'static str) {
    let step = (lon1 - lon0) / n as f64;
    for i in 0..n {
        out.push(Seg {
            a: (lat, lon0 + i as f64 * step),
            b: (lat, lon0 + (i + 1) as f64 * step),
            oneway: false,
            name,
            flags: 0,
        });
    }
}

#[test]
fn snap_skips_a_nearby_island_for_the_connected_road() {
    let mut segs = Vec::new();
    snake_segs(&mut segs, 40.7000, -73.8990, -73.6890, 2100, "Main Rd");
    segs.push(Seg { a: (40.7005, -73.8900), b: (40.7005, -73.89001), oneway: false, name: "Island St", flags: 0 });
    segs.push(Seg { a: (40.7005, -73.89001), b: (40.7005, -73.89002), oneway: false, name: "Island St", flags: 0 });
    let d = tmpdir("snap-island");
    let w = build(&segs);
    let entry = write_roads(&d, &w);
    let mut r = open(&d, &entry);
    let route = r.route((40.7000, -73.8990), (40.7005, -73.8900)).unwrap();
    assert!(route.distance_m > 500.0, "destination must snap to the connected road, got {} m", route.distance_m);
    fs::remove_dir_all(&d).ok();
}

#[test]
fn snap_looks_past_every_edge_of_a_many_edge_island() {
    let mut segs = Vec::new();
    snake_segs(&mut segs, 40.7000, -73.8990, -73.6890, 2100, "Main Rd");
    for i in 0..12 {
        let a = (40.7009, -73.8900 + i as f64 * 0.00002);
        let b = (40.7009, -73.8900 + (i + 1) as f64 * 0.00002);
        segs.push(Seg { a, b, oneway: false, name: "Gated Ln", flags: 0 });
    }
    let d = tmpdir("snap-big-island");
    let w = build(&segs);
    let entry = write_roads(&d, &w);
    let mut r = open(&d, &entry);
    let route = r.route((40.7000, -73.8990), (40.7009, -73.8900)).unwrap();
    assert!(route.distance_m > 500.0, "destination must snap to the connected road, got {} m", route.distance_m);
    fs::remove_dir_all(&d).ok();
}

#[test]
fn snap_skips_a_destination_you_can_leave_but_never_enter() {
    let mut segs = Vec::new();
    snake_segs(&mut segs, 40.7000, -73.8990, -73.6890, 2100, "Main Rd");
    let join = -73.8990 + 90.0 * ((-73.6890 - -73.8990) / 2100.0);
    segs.push(Seg { a: (40.7009, join), b: (40.7000, join), oneway: true, name: "Exit Only", flags: 0 });
    for i in 0..3 {
        let a = (40.7009, join + i as f64 * 0.0002);
        let b = (40.7009, join + (i + 1) as f64 * 0.0002);
        segs.push(Seg { a, b, oneway: false, name: "Gated Ln", flags: 0 });
    }
    let d = tmpdir("snap-exit-only");
    let w = build(&segs);
    let entry = write_roads(&d, &w);
    let mut r = open(&d, &entry);
    let route = r.route((40.7000, -73.8990), (40.7009, join + 0.0005)).unwrap();
    assert!(route.distance_m > 500.0, "destination must snap to a road you can drive onto, got {} m", route.distance_m);
    let back = r.route((40.7009, join + 0.0005), (40.7000, -73.8990)).unwrap();
    assert!(back.distance_m > 500.0, "leaving the enclave must still work, got {} m", back.distance_m);
    fs::remove_dir_all(&d).ok();
}

#[test]
fn a_start_on_a_junction_can_take_any_road_out_of_it() {
    let mut segs = Vec::new();
    for i in 0..100 {
        let a = (40.7000, -73.9000 + i as f64 * 0.001);
        let b = (40.7000, -73.9000 + (i + 1) as f64 * 0.001);
        segs.push(Seg { a, b, oneway: true, name: "Expressway", flags: 0 });
    }
    let x = (40.7000, -73.9000 + 50.0 * 0.001);
    segs.push(Seg { a: x, b: (40.7010, x.1), oneway: true, name: "Exit", flags: 0 });
    for i in 0..10 {
        let a = (40.7010, x.1 - i as f64 * 0.001);
        let b = (40.7010, x.1 - (i + 1) as f64 * 0.001);
        segs.push(Seg { a, b, oneway: false, name: "Frontage Rd", flags: 0 });
    }
    let d = tmpdir("junction-start");
    let w = build(&segs);
    let entry = write_roads(&d, &w);
    let mut r = open(&d, &entry);
    let to = (40.7010, x.1 - 0.010);
    let route = r.route(x, to).unwrap();
    assert!(route.distance_m < 1500.0, "start on the exit junction must be able to take the exit, got {} m", route.distance_m);
    let back = r.route((40.7000, -73.9000), x).unwrap();
    assert!((back.distance_m - 4200.0).abs() < 100.0, "end on the junction, got {} m", back.distance_m);
    fs::remove_dir_all(&d).ok();
}

#[test]
fn a_point_in_the_middle_of_a_road_snaps_to_that_spot() {
    let segs = vec![
        Seg { a: (40.7000, -73.9000), b: (40.7100, -73.9000), oneway: false, name: "Long Rd", flags: 0 },
        Seg { a: (40.7100, -73.9000), b: (40.7100, -73.8900), oneway: false, name: "Cross St", flags: 0 },
    ];
    let d = tmpdir("mid-road");
    let w = build(&segs);
    let entry = write_roads(&d, &w);
    let mut r = open(&d, &entry);
    let route = r.route((40.7050, -73.9000), (40.7100, -73.8950)).unwrap();
    let want = 556.0 + 422.0;
    assert!((route.distance_m - want).abs() < 20.0, "expected about {want} m from the middle of Long Rd, got {} m", route.distance_m);
    let first = route.line[0];
    assert!((first[1] - 40.7050).abs() < 1e-4, "route must start where the point is, got {:?}", first);
    fs::remove_dir_all(&d).ok();
}

#[test]
fn two_points_on_the_same_road_drive_straight_between_them() {
    let segs = vec![
        Seg { a: (40.7000, -73.9000), b: (40.7100, -73.9000), oneway: false, name: "Long Rd", flags: 0 },
        Seg { a: (40.7100, -73.9000), b: (40.7100, -73.8900), oneway: false, name: "Cross St", flags: 0 },
        Seg { a: (40.7000, -73.9000), b: (40.7000, -73.8900), oneway: false, name: "Low St", flags: 0 },
    ];
    let d = tmpdir("same-road");
    let w = build(&segs);
    let entry = write_roads(&d, &w);
    let mut r = open(&d, &entry);
    for (a, b) in [((40.7020, -73.9000), (40.7080, -73.9000)), ((40.7080, -73.9000), (40.7020, -73.9000))] {
        let route = r.route(a, b).unwrap();
        assert!((route.distance_m - 667.0).abs() < 15.0, "{a:?} -> {b:?} should be about 667 m, got {} m", route.distance_m);
    }
    fs::remove_dir_all(&d).ok();
}

#[test]
fn island_to_island_unreachable_is_a_fast_error() {
    let segs = vec![
        Seg { a: (40.7000, -73.9000), b: (40.7000, -73.8990), oneway: false, name: "Island A", flags: 0 },
        Seg { a: (40.9000, -73.7000), b: (40.9000, -73.6990), oneway: false, name: "Island B", flags: 0 },
    ];
    let d = tmpdir("island-island");
    let w = build(&segs);
    let entry = write_roads(&d, &w);
    let mut r = open(&d, &entry);
    let t0 = std::time::Instant::now();
    let err = r.route((40.7000, -73.9000), (40.9000, -73.6990)).unwrap_err();
    let elapsed = t0.elapsed();
    assert!(err.contains("No route"), "got: {err}");
    assert!(elapsed.as_secs() < 5, "unreachable route took too long: {:?}", elapsed);
    fs::remove_dir_all(&d).ok();
}

#[test]
fn routes_within_an_isolated_island() {
    let segs = vec![
        Seg { a: (40.7000, -73.9000), b: (40.7010, -73.9000), oneway: false, name: "Harbor Rd", flags: 0 },
        Seg { a: (40.7010, -73.9000), b: (40.7010, -73.8990), oneway: false, name: "Dock St", flags: 0 },
        Seg { a: (40.7010, -73.8990), b: (40.7000, -73.8990), oneway: false, name: "Shore Rd", flags: 0 },
    ];
    let d = tmpdir("island-route");
    let w = build(&segs);
    let entry = write_roads(&d, &w);
    let mut r = open(&d, &entry);
    let route = r.route((40.7000, -73.9000), (40.7000, -73.8990)).unwrap();
    assert!(route.distance_m > 150.0, "route within the island must work, got {} m", route.distance_m);
    fs::remove_dir_all(&d).ok();
}

#[test]
fn snap_checks_a_neighbouring_cell_before_accepting_a_near_edge() {
    let segs = vec![
        Seg { a: (40.6900, -73.9000), b: (40.7100, -73.9000), oneway: false, name: "Short Rd", flags: 0 },
        Seg { a: (40.7000, -73.8996), b: (40.7000, -73.8900), oneway: false, name: "Long Rd", flags: 0 },
        Seg { a: (40.7000, -73.8900), b: (40.7100, -73.8900), oneway: false, name: "Long Rd", flags: 0 },
        Seg { a: (40.7100, -73.8900), b: (40.7100, -73.9000), oneway: false, name: "Long Rd", flags: 0 },
    ];
    let d = tmpdir("snap-radius");
    let w = build(&segs);
    let entry = write_roads(&d, &w);
    let mut r = open(&d, &entry);
    let route = r.route((40.7000, -73.9000), (40.7100, -73.9000)).unwrap();
    assert!(route.distance_m < 2000.0, "must snap to the nearer road, got {} m", route.distance_m);
    fs::remove_dir_all(&d).ok();
}
