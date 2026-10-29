use alexandria_core::{write_hashes, Entry, VerifiedFile};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

fn tempdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("alex-hard-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn entry_for(path: &std::path::Path) -> Entry {
    let hashes = write_hashes(path, 4096).unwrap();
    let raw = fs::read(&hashes).unwrap();
    let mut h = [0u8; 32];
    h.copy_from_slice(&Sha256::digest(&raw));
    Entry {
        hashes_sha256: h,
        size: fs::metadata(path).unwrap().len(),
        path: "f".into(),
    }
}

#[test]
fn refuses_symlinked_files() {
    let d = tempdir("symlink-file");
    let real = d.join("real.bin");
    fs::write(&real, b"hello world").unwrap();
    let e = entry_for(&real);
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let link = d.join("link.bin");
        symlink(&real, &link).unwrap();
        assert!(VerifiedFile::open(&link, &e, 4096).is_err());
    }
    assert!(VerifiedFile::open(&real, &e, 4096).is_ok());
}

#[test]
fn refuses_symlinked_or_oversized_hash_lists() {
    let d = tempdir("symlink-hashes");
    let real = d.join("f.bin");
    fs::write(&real, b"hello world").unwrap();
    let e = entry_for(&real);
    let hashes = d.join("f.bin.hashes");

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let saved = d.join("saved.hashes");
        fs::rename(&hashes, &saved).unwrap();
        symlink(&saved, &hashes).unwrap();
        assert!(VerifiedFile::open(&real, &e, 4096).is_err());
        fs::remove_file(&hashes).unwrap();
        fs::rename(&saved, &hashes).unwrap();
    }

    let mut extra = fs::read(&hashes).unwrap();
    extra.extend_from_slice(&[0u8; 32]);
    fs::write(&hashes, extra).unwrap();
    assert!(VerifiedFile::open(&real, &e, 4096).is_err());
}
