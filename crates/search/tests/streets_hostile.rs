use alexandria_core::{write_hashes, Entry, VerifiedFile};
use alexandria_search::streets::Streets;
use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const CH: u64 = 4096;

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("alex-streets-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&p).unwrap();
    p
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

fn open_signed(dir: &Path) -> Option<Streets> {
    let path = dir.join("streets.txt");
    let entry = entry_for(&path);
    let vf = VerifiedFile::open_in(dir, &entry, CH).unwrap();
    Streets::open_verified(vf, CH)
}

#[test]
fn blocks_out_of_order_are_refused() {
    let d = tmpdir("unsorted");
    let path = d.join("streets.txt");
    let mut f = fs::File::create(&path).unwrap();
    f.write_all(format!("{}\n", " ".repeat(63)).as_bytes()).unwrap();
    f.write_all(b"b\tBeta Street\t40.8\t-73.8\n").unwrap();
    let offset_b = f.stream_position().unwrap();
    f.write_all(b"a\tAlpha Street\t40.7\t-73.9\n").unwrap();
    let index_at = f.stream_position().unwrap();
    f.write_all(format!("z\t64\nb\t{offset_b}\n").as_bytes()).unwrap();
    let head = format!("alexandria-streets 3 {index_at} 2");
    f.seek(SeekFrom::Start(0)).unwrap();
    f.write_all(format!("{:63}\n", head).as_bytes()).unwrap();

    assert!(open_signed(&d).is_none(), "unsorted block keys must be refused");
    fs::remove_dir_all(&d).ok();
}

#[test]
fn a_block_offset_far_before_its_key_scans_a_bounded_span() {
    let d = tmpdir("far-offset");
    let path = d.join("streets.txt");
    let mut f = fs::File::create(&path).unwrap();
    f.write_all(format!("{}\n", " ".repeat(63)).as_bytes()).unwrap();
    let row = b"aa\tAlpha Street\t40.7\t-73.9\n";
    let mut total = 64u64;
    while total < 5 * 1024 * 1024 {
        f.write_all(row).unwrap();
        total += row.len() as u64;
    }
    f.write_all(b"be\tBeta Street\t40.8\t-73.8\n").unwrap();
    let index_at = f.stream_position().unwrap();
    f.write_all(b"aa\t64\nbe\t100\n").unwrap();
    let head = format!("alexandria-streets 3 {index_at} 2");
    f.seek(SeekFrom::Start(0)).unwrap();
    f.write_all(format!("{:63}\n", head).as_bytes()).unwrap();

    let s = open_signed(&d).expect("sorted blocks with increasing offsets must open");
    let hits = s.find("be", None, 5);
    assert!(
        hits.is_empty(),
        "the scan must stop before the mislocated key, not scan to EOF: {hits:?}"
    );
    fs::remove_dir_all(&d).ok();
}
