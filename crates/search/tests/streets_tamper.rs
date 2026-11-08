use alexandria_core::{write_hashes, Entry, VerifiedFile};
use alexandria_search::streets::Streets;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CH: u64 = 4096;

fn tmpdir(tag: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("alex-streets-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&p).unwrap();
    p
}

fn write_indexed(path: &Path) {
    use std::io::{Seek, SeekFrom, Write};
    let rows: [(&str, &str, &str, &str); 3] = [
        ("bedford avenue", "Bedford Avenue", "40.678", "-73.953"),
        ("broadway", "Broadway", "40.760", "-73.980"),
        ("tampered", "Tampered Street", "40.123", "-73.456"),
    ];
    let mut f = fs::File::create(path).unwrap();
    f.write_all(format!("{}\n", " ".repeat(63)).as_bytes()).unwrap();
    for (key, name, lat, lon) in rows {
        writeln!(f, "{key}\t{name}\t{lat}\t{lon}").unwrap();
    }
    let index_at = f.stream_position().unwrap();
    writeln!(f, "tampered\t{}", 0).unwrap();
    let head = format!("alexandria-streets 3 {index_at} {}", rows.len());
    f.seek(SeekFrom::Start(0)).unwrap();
    f.write_all(format!("{:63}\n", head).as_bytes()).unwrap();
}

fn entry_for(path: &Path) -> Entry {
    let size = fs::metadata(path).unwrap().len();
    let hashes = write_hashes(path, CH).unwrap();
    let raw = fs::read(hashes).unwrap();
    use sha2::{Digest, Sha256};
    let mut h = [0u8; 32];
    h.copy_from_slice(&Sha256::digest(&raw));
    Entry {
        hashes_sha256: h,
        size,
        path: path.file_name().unwrap().to_str().unwrap().to_string(),
    }
}

#[test]
fn a_tampered_street_file_serves_nothing() {
    let dir = tmpdir("tamper");
    let path = dir.join("streets.txt");
    write_indexed(&path);
    let entry = entry_for(&path);

    let mut raw = fs::read(&path).unwrap();
    let i = raw.windows(6).position(|w| w == b"40.123").unwrap();
    raw[i + 3] = b'9';
    fs::write(&path, &raw).unwrap();

    let hits = VerifiedFile::open_in(&dir, &entry, CH)
        .ok()
        .and_then(|vf| Streets::open_verified(vf, CH))
        .map(|s| s.find("tampered", None, 5))
        .unwrap_or_default();
    assert!(
        !hits.iter().any(|(n, _, _, _)| n == "Tampered Street"),
        "served the tampered name: {hits:?}"
    );
    fs::remove_dir_all(&dir).ok();
}
