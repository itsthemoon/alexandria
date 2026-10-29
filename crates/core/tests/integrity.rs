use alexandria_core::{
    hash_file, hex, load_signing_key, parse_verifying_key, sign_bytes, verify_bytes, write_hashes,
    Entry, Error, Manifest, VerifiedFile,
};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const CH: u64 = 4096;

fn tmpdir(tag: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("alex-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&p).unwrap();
    p
}

fn write_file(path: &Path, bytes: &[u8]) {
    let mut f = File::create(path).unwrap();
    f.write_all(bytes).unwrap();
    f.sync_all().unwrap();
}

fn entry_for(path: &Path, chunk: u64) -> Entry {
    let size = fs::metadata(path).unwrap().len();
    let hashes = write_hashes(path, chunk).unwrap();
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

fn fixed_key() -> (ed25519_dalek::SigningKey, String) {
    let seed = [7u8; 32];
    let k = load_signing_key(&hex(&seed)).unwrap();
    let pubhex = hex(k.verifying_key().as_bytes());
    (k, pubhex)
}

#[test]
fn manifest_roundtrip_and_signature() {
    let dir = tmpdir("sig");
    let file = dir.join("data.bin");
    write_file(&file, &vec![9u8; 10_000]);
    let e = entry_for(&file, CH);
    let m = Manifest {
        release: "v1".into(),
        date: "2026-01-01".into(),
        chunk: CH,
        entries: vec![e],
    };
    let text = m.to_text();
    let parsed = Manifest::parse(&text).unwrap();
    assert_eq!(parsed, m);
    let (key, pubhex) = fixed_key();
    let raw = text.as_bytes();
    let sig = sign_bytes(&key, raw);
    verify_bytes(&parse_verifying_key(&pubhex).unwrap(), raw, &sig).unwrap();
    let mut bad = sig;
    bad[0] ^= 1;
    assert!(matches!(
        verify_bytes(&parse_verifying_key(&pubhex).unwrap(), raw, &bad),
        Err(Error::BadSignature)
    ));
}

#[test]
fn rejects_unsafe_manifest_entries() {
    let base = "alexandria-manifest 1\nrelease v1 2026-01-01\nchunk 4096\n";
    let h = "00".repeat(32);
    for bad in [
        format!("{h} 1 ../etc/passwd"),
        format!("{h} 1 /abs/path"),
        format!("{h} 1 dir/./x"),
        format!("{h} 1 A.BIN"),
        format!("{h} 1 dup\n{h} 1 dup"),
    ] {
        assert!(
            Manifest::parse(&format!("{base}{bad}\n")).is_err(),
            "accepted: {bad}"
        );
    }
}

#[test]
fn truncated_and_oversized_files_fail() {
    let dir = tmpdir("size");
    let f = dir.join("a.bin");
    write_file(&f, &vec![1u8; 5000]);
    let e = entry_for(&f, CH);

    fs::write(&f, vec![1u8; 4999]).unwrap();
    assert!(matches!(
        VerifiedFile::open(&f, &e, CH),
        Err(Error::SizeMismatch { .. })
    ));

    fs::write(&f, vec![1u8; 5001]).unwrap();
    assert!(matches!(
        VerifiedFile::open(&f, &e, CH),
        Err(Error::SizeMismatch { .. })
    ));
}

#[test]
fn tampered_chunk_is_refused() {
    let dir = tmpdir("chunk");
    let f = dir.join("a.bin");
    let mut data = vec![0u8; 10_000];
    for (i, b) in data.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    write_file(&f, &data);
    let e = entry_for(&f, CH);
    let mut r = VerifiedFile::open(&f, &e, CH).unwrap();

    let mut raw = OpenOptions::new().read(true).write(true).open(&f).unwrap();
    raw.seek(SeekFrom::Start(CH + 5)).unwrap();
    raw.write_all(&[0xEE]).unwrap();
    raw.sync_all().unwrap();

    let err = r.read_at(CH, 16).unwrap_err();
    assert!(matches!(err, Error::ChunkMismatch { index: 1 }));
    assert_eq!(r.read_at(0, 16).unwrap(), &data[..16]);
}

#[test]
fn swapped_chunks_are_refused() {
    let dir = tmpdir("swap");
    let f = dir.join("a.bin");
    let data: Vec<u8> = (0..10_000).map(|i| (i % 251) as u8).collect();
    write_file(&f, &data);
    let e = entry_for(&f, CH);
    let mut whole = fs::read(&f).unwrap();
    let (a, b) = whole.split_at_mut(CH as usize);
    let tmp = a.to_vec();
    a.copy_from_slice(&b[..CH as usize]);
    b[..CH as usize].copy_from_slice(&tmp);
    fs::write(&f, &whole).unwrap();

    let mut r = VerifiedFile::open(&f, &e, CH).unwrap();
    assert!(matches!(
        r.read_at(0, 16),
        Err(Error::ChunkMismatch { index: 0 })
    ));
}

#[test]
fn tampered_hash_list_is_refused() {
    let dir = tmpdir("hashes");
    let f = dir.join("a.bin");
    write_file(&f, &vec![2u8; 5000]);
    let e = entry_for(&f, CH);
    let hp = PathBuf::from(format!("{}.hashes", f.display()));
    let mut hb = fs::read(&hp).unwrap();
    hb[0] ^= 1;
    fs::write(&hp, hb).unwrap();
    assert!(matches!(
        VerifiedFile::open(&f, &e, CH),
        Err(Error::HashesMismatch { .. })
    ));
}

#[test]
fn partial_last_chunk_reads() {
    let dir = tmpdir("last");
    let f = dir.join("a.bin");
    let data: Vec<u8> = (0..5000).map(|i| (i % 251) as u8).collect();
    write_file(&f, &data);
    let e = entry_for(&f, CH);
    let mut r = VerifiedFile::open(&f, &e, CH).unwrap();
    assert_eq!(r.read_at(4090, 10).unwrap(), &data[4090..4100]);
    assert_eq!(r.read_at(4990, 10).unwrap(), &data[4990..5000]);
    assert!(matches!(r.read_at(4995, 10), Err(Error::OutOfBounds)));
}

#[test]
fn chunk_from_another_file_is_refused() {
    let dir = tmpdir("cross");
    let a = dir.join("a.bin");
    let b = dir.join("b.bin");
    write_file(&a, &vec![3u8; 5000]);
    write_file(&b, &vec![4u8; 5000]);
    let ea = entry_for(&a, CH);
    let _eb = entry_for(&b, CH);
    let b_bytes = fs::read(&b).unwrap();
    let mut a_bytes = fs::read(&a).unwrap();
    a_bytes[..CH as usize].copy_from_slice(&b_bytes[..CH as usize]);
    fs::write(&a, &a_bytes).unwrap();
    let mut r = VerifiedFile::open(&a, &ea, CH).unwrap();
    assert!(r.read_at(0, 8).is_err());
}

#[test]
fn concurrent_writer_never_yields_wrong_bytes() {
    let dir = tmpdir("race");
    let f = dir.join("a.bin");
    let original: Vec<u8> = (0..64_000).map(|i| (i % 251) as u8).collect();
    write_file(&f, &original);
    let e = entry_for(&f, CH);
    let mut r = VerifiedFile::open(&f, &e, CH).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let path = f.clone();
    let writer = thread::spawn(move || {
        let mut w = OpenOptions::new().write(true).open(path).unwrap();
        let mut i = 0u64;
        while !flag.load(Ordering::Relaxed) {
            let off = (i * 37) % 64_000;
            w.seek(SeekFrom::Start(off)).unwrap();
            w.write_all(&[0xA5]).unwrap();
            i += 1;
        }
    });

    let mut ok = 0;
    let mut err = 0;
    for i in 0..2000u64 {
        let off = (i * 131) % 60_000;
        match r.read_at(off, 64) {
            Ok(buf) => {
                assert_eq!(
                    buf,
                    &original[off as usize..off as usize + 64],
                    "returned bytes that did not match the signed hash"
                );
                ok += 1;
            }
            Err(_) => err += 1,
        }
    }
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();
    assert!(ok + err == 2000);
}

#[test]
fn full_verify_detects_bit_rot() {
    let dir = tmpdir("rot");
    let f = dir.join("a.bin");
    write_file(&f, &vec![5u8; 9000]);
    let e = entry_for(&f, CH);
    let mut r = VerifiedFile::open(&f, &e, CH).unwrap();
    r.verify_all().unwrap();

    let mut raw = OpenOptions::new().read(true).write(true).open(&f).unwrap();
    raw.seek(SeekFrom::Start(8000)).unwrap();
    raw.write_all(&[0]).unwrap();
    raw.sync_all().unwrap();
    assert!(r.verify_all().is_err());
}

#[test]
fn hash_file_matches_manual_chunks() {
    let dir = tmpdir("hash");
    let f = dir.join("a.bin");
    let data = vec![42u8; 4096 * 2 + 7];
    write_file(&f, &data);
    let hs = hash_file(&f, 4096).unwrap();
    assert_eq!(hs.len(), 3 * 32);
    use sha2::{Digest, Sha256};
    let mut expect = Vec::new();
    for c in data.chunks(4096) {
        expect.extend_from_slice(&Sha256::digest(c));
    }
    assert_eq!(hs, expect);
}

#[test]
fn open_is_read_only() {
    let dir = tmpdir("ro");
    let f = dir.join("a.bin");
    write_file(&f, &vec![1u8; 4096]);
    let e = entry_for(&f, CH);
    let r = VerifiedFile::open(&f, &e, CH).unwrap();
    drop(r);
    let mut w = OpenOptions::new().write(true).open(&f).unwrap();
    w.write_all(&[0]).unwrap();
}
