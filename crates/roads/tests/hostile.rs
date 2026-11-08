use alexandria_core::{write_hashes, Entry, VerifiedFile};
use alexandria_roads::Roads;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::Path;

const CH: u64 = 4096;

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("alex-roads-hostile-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&p).unwrap();
    p
}

fn entry_for(dir: &Path, bytes: &[u8]) -> Entry {
    let path = dir.join("graph.roads");
    fs::write(&path, bytes).unwrap();
    let size = bytes.len() as u64;
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

fn open_bytes(bytes: &[u8], tag: &str) -> Result<Roads, String> {
    let d = tmpdir(tag);
    let e = entry_for(&d, bytes);
    let vf = VerifiedFile::open_in(&d, &e, CH).unwrap();
    let r = Roads::open(vf);
    fs::remove_dir_all(&d).ok();
    r
}

fn valid_header(bytes: &mut Vec<u8>) {
    bytes.extend_from_slice(b"ALXROAD1");
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&0i32.to_le_bytes());
    bytes.extend_from_slice(&0i32.to_le_bytes());
    bytes.extend_from_slice(&1i32.to_le_bytes());
    bytes.extend_from_slice(&1i32.to_le_bytes());
    bytes.extend_from_slice(&10000u32.to_le_bytes());
    bytes.extend_from_slice(&100u32.to_le_bytes());
}

#[test]
fn rejects_short_files() {
    assert!(open_bytes(b"ALX", "short").is_err());
    assert!(open_bytes(&[], "empty").is_err());
}

#[test]
fn rejects_bad_magic() {
    let mut b = Vec::new();
    b.extend_from_slice(b"NOTROADS");
    b.resize(64, 0);
    assert!(open_bytes(&b, "magic").is_err());
}

#[test]
fn rejects_bad_version() {
    let mut b = Vec::new();
    valid_header(&mut b);
    b[8] = 9;
    b.resize(64, 0);
    assert!(open_bytes(&b, "version").is_err());
}

#[test]
fn rejects_out_of_range_edge_target_without_panicking() {
    let mut b = Vec::new();
    valid_header(&mut b);
    b[12..16].copy_from_slice(&1u32.to_le_bytes());
    b[16..20].copy_from_slice(&1u32.to_le_bytes());
    b[20..24].copy_from_slice(&0u32.to_le_bytes());
    b.resize(64, 0);
    let mut f = fs::File::create(std::env::temp_dir().join("unused")).unwrap();
    f.write_all(b"x").unwrap();
    let bytes = b.clone();
    assert!(open_bytes(&bytes, "edge-target").is_err());
}

#[test]
fn rejects_node_degree_overflow() {
    let mut b = Vec::new();
    valid_header(&mut b);
    b[12..16].copy_from_slice(&1u32.to_le_bytes());
    b[16..20].copy_from_slice(&0u32.to_le_bytes());
    b.resize(64, 0);
    let bytes = b.clone();
    assert!(open_bytes(&bytes, "degree").is_err());
}

fn graph_with_one_edge(target: u32, degree: u32) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"ALXROAD1");
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(&2u32.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&40_000_000i32.to_le_bytes());
    b.extend_from_slice(&(-74_000_000i32).to_le_bytes());
    b.extend_from_slice(&41_000_000i32.to_le_bytes());
    b.extend_from_slice(&(-73_000_000i32).to_le_bytes());
    b.extend_from_slice(&20_000u32.to_le_bytes());
    b.extend_from_slice(&250u32.to_le_bytes());
    b.extend_from_slice(&40_700_000i32.to_le_bytes());
    b.extend_from_slice(&(-73_900_000i32).to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&degree.to_le_bytes());
    b.extend_from_slice(&40_701_000i32.to_le_bytes());
    b.extend_from_slice(&(-73_899_000i32).to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&target.to_le_bytes());
    b.extend_from_slice(&1000u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.push(0);
    b.push(0);
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&2u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b
}

#[test]
fn rejects_out_of_range_edge_target_at_route_time() {
    let d = tmpdir("route-edge-target");
    let bytes = graph_with_one_edge(999, 1);
    let e = entry_for(&d, &bytes);
    let vf = VerifiedFile::open_in(&d, &e, CH).unwrap();
    let mut r = Roads::open(vf).unwrap();
    let err = r.route((40.7, -73.9), (40.701, -73.899)).unwrap_err();
    assert!(err.contains("out of range"), "got: {err}");
    fs::remove_dir_all(&d).ok();
}

#[test]
fn rejects_out_of_range_node_degree_at_route_time() {
    let d = tmpdir("route-node-degree");
    let bytes = graph_with_one_edge(1, 5);
    let e = entry_for(&d, &bytes);
    let vf = VerifiedFile::open_in(&d, &e, CH).unwrap();
    let mut r = Roads::open(vf).unwrap();
    let err = r.route((40.7, -73.9), (40.701, -73.899)).unwrap_err();
    assert!(err.contains("out of range") || err.contains("edge range"), "got: {err}");
    fs::remove_dir_all(&d).ok();
}
