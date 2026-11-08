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

#[test]
#[cfg(unix)]
fn rejects_a_symlink_component_inside_the_library() {
    use std::os::unix::fs::symlink;
    let d = tempdir("symlink-component");
    let target = d.join("target.bin");
    fs::write(&target, b"hello world").unwrap();
    let e = entry_for(&target);
    let link = d.join("f");
    symlink(&target, &link).unwrap();
    assert!(VerifiedFile::open_in(&d, &e, 4096).is_err());
}

#[test]
#[cfg(unix)]
fn rejects_a_symlinked_directory_component() {
    use std::os::unix::fs::symlink;
    let d = tempdir("symlink-dir");
    let realdir = d.join("real");
    fs::create_dir(&realdir).unwrap();
    let target = realdir.join("f");
    fs::write(&target, b"hello world").unwrap();
    let e = {
        let hashes = write_hashes(&target, 4096).unwrap();
        let raw = fs::read(&hashes).unwrap();
        let mut h = [0u8; 32];
        h.copy_from_slice(&Sha256::digest(&raw));
        Entry {
            hashes_sha256: h,
            size: fs::metadata(&target).unwrap().len(),
            path: "sub/f".into(),
        }
    };
    symlink(&realdir, d.join("sub")).unwrap();
    assert!(VerifiedFile::open_in(&d, &e, 4096).is_err());
}
