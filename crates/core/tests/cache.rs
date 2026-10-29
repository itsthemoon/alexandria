use alexandria_core::{cached_copy, hash_file, write_hashes, Entry};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CH: u64 = 4096;

fn tmpdir(tag: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("alex-cache-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&p).unwrap();
    p
}

fn write(path: &Path, bytes: &[u8]) {
    let mut f = File::create(path).unwrap();
    f.write_all(bytes).unwrap();
    f.sync_all().unwrap();
}

fn entry_for(path: &Path) -> (Entry, PathBuf) {
    let size = fs::metadata(path).unwrap().len();
    let hashes = write_hashes(path, CH).unwrap();
    let raw = fs::read(&hashes).unwrap();
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&hash_file(&hashes, CH).unwrap());
    let e = Entry {
        hashes_sha256: digest,
        size,
        path: path.file_name().unwrap().to_string_lossy().to_string(),
    };
    (e, hashes)
}

#[test]
fn the_copy_is_written_once_and_reused() {
    let root = tmpdir("reuse");
    let cache = root.join("cache");
    let src = root.join("big.bin");
    let bytes: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
    write(&src, &bytes);
    let (e, _h) = entry_for(&src);

    let a = cached_copy(&root, &e, CH, &cache).unwrap();
    assert_eq!(fs::read(&a).unwrap(), bytes, "copy is the real bytes");
    let first = fs::metadata(&a).unwrap().modified().unwrap();

    std::thread::sleep(std::time::Duration::from_millis(20));
    let b = cached_copy(&root, &e, CH, &cache).unwrap();
    assert_eq!(a, b);
    assert_eq!(fs::metadata(&b).unwrap().modified().unwrap(), first);
}

#[test]
fn a_truncated_copy_is_replaced_not_trusted() {
    let root = tmpdir("short");
    let cache = root.join("cache");
    let src = root.join("m.bin");
    write(&src, &vec![7u8; 9000]);
    let (e, _h) = entry_for(&src);

    let p = cached_copy(&root, &e, CH, &cache).unwrap();
    fs::write(&p, b"short").unwrap();
    let q = cached_copy(&root, &e, CH, &cache).unwrap();
    assert_eq!(q, p);
    assert_eq!(fs::metadata(&q).unwrap().len(), 9000);
}

#[test]
fn a_same_size_tampered_copy_is_replaced_not_trusted() {
    let root = tmpdir("tamper");
    let cache = root.join("cache");
    let src = root.join("m.bin");
    let bytes: Vec<u8> = (0..12_000u32).map(|i| (i % 241) as u8).collect();
    write(&src, &bytes);
    let (e, _h) = entry_for(&src);

    let p = cached_copy(&root, &e, CH, &cache).unwrap();
    let mut bad = fs::read(&p).unwrap();
    bad[5000] ^= 0xff;
    fs::write(&p, &bad).unwrap();
    assert_eq!(fs::metadata(&p).unwrap().len(), bytes.len() as u64);

    let q = cached_copy(&root, &e, CH, &cache).unwrap();
    assert!(fs::read(&q).unwrap() == bytes, "tampered copy is rebuilt from the verified library");
}

#[test]
fn copies_of_other_entries_are_swept_and_partials_are_ignored() {
    let root = tmpdir("sweep");
    let cache = root.join("cache");
    let src = root.join("m.bin");
    write(&src, &vec![3u8; 5000]);
    let (e, _h) = entry_for(&src);
    fs::create_dir_all(&cache).unwrap();
    let stale = cache.join("old-model-0000.bin");
    let partial = cache.join("something.bin.part");
    fs::write(&stale, b"junk").unwrap();
    fs::write(&partial, b"junk").unwrap();
    let keep = cache.join("notes.txt");
    fs::write(&keep, b"keep me").unwrap();

    let p = cached_copy(&root, &e, CH, &cache).unwrap();
    assert!(!stale.exists(), "old copy removed");
    assert!(!partial.exists(), "partial write removed");
    assert!(keep.exists(), "unrelated files are left alone");
    assert_eq!(fs::metadata(&p).unwrap().len(), 5000);
}
