use alexandria_core::{hex, load_signing_key, open_signed, sign_bytes, sign_keys, Error};
use ed25519_dalek::SigningKey;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const MANIFEST: &str = "alexandria-manifest 1\nrelease v1 2026-10-27\nchunk 1048576\n";

fn tmpdir(tag: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("alex-keys-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&p).unwrap();
    p
}

fn key(b: u8) -> SigningKey {
    load_signing_key(&hex(&[b; 32])).unwrap()
}

fn keys_text(keys: &[&SigningKey]) -> String {
    let mut s = String::from("alexandria-keys 1\n");
    for k in keys {
        s.push_str(&hex(k.verifying_key().as_bytes()));
        s.push('\n');
    }
    s
}

fn library(tag: &str, signer: &SigningKey, keys: Option<(&str, &SigningKey)>) -> PathBuf {
    let dir = tmpdir(tag);
    fs::write(dir.join("MANIFEST"), MANIFEST).unwrap();
    fs::write(dir.join("MANIFEST.sig"), sign_bytes(signer, MANIFEST.as_bytes())).unwrap();
    if let Some((text, ks)) = keys {
        fs::write(dir.join("KEYS"), text).unwrap();
        fs::write(dir.join("KEYS.sig"), sign_keys(ks, text.as_bytes())).unwrap();
    }
    dir
}

#[test]
fn root_may_sign_the_manifest_directly() {
    let root = key(1);
    let dir = library("root", &root, None);
    let s = open_signed(&dir, &root.verifying_key()).unwrap();
    assert_eq!(s.manifest.release, "v1");
    assert_eq!(s.raw, MANIFEST.as_bytes());
}

#[test]
fn a_release_key_listed_in_root_signed_keys_is_trusted() {
    let (root, release) = (key(1), key(2));
    let dir = library("listed", &release, Some((&keys_text(&[&release]), &root)));
    let s = open_signed(&dir, &root.verifying_key()).unwrap();
    assert_eq!(s.manifest.date, "2026-10-27");
}

#[test]
fn any_listed_key_works_not_only_the_first() {
    let (root, a, b) = (key(1), key(2), key(3));
    let dir = library("second", &b, Some((&keys_text(&[&a, &b]), &root)));
    assert!(open_signed(&dir, &root.verifying_key()).is_ok());
}

#[test]
fn a_release_key_without_keys_is_refused() {
    let (root, release) = (key(1), key(2));
    let dir = library("nokeys", &release, None);
    assert!(matches!(
        open_signed(&dir, &root.verifying_key()),
        Err(Error::BadSignature)
    ));
}

#[test]
fn an_unlisted_key_is_refused() {
    let (root, listed, other) = (key(1), key(2), key(3));
    let dir = library("unlisted", &other, Some((&keys_text(&[&listed]), &root)));
    assert!(matches!(
        open_signed(&dir, &root.verifying_key()),
        Err(Error::BadSignature)
    ));
}

#[test]
fn keys_signed_by_anyone_but_root_are_refused() {
    let (root, release) = (key(1), key(2));
    let dir = library("selfsigned", &release, Some((&keys_text(&[&release]), &release)));
    assert!(matches!(
        open_signed(&dir, &root.verifying_key()),
        Err(Error::BadSignature)
    ));
}

#[test]
fn a_key_added_to_keys_after_signing_is_refused() {
    let (root, listed, attacker) = (key(1), key(2), key(9));
    let dir = library("tampered", &attacker, Some((&keys_text(&[&listed]), &root)));
    fs::write(dir.join("KEYS"), keys_text(&[&listed, &attacker])).unwrap();
    assert!(matches!(
        open_signed(&dir, &root.verifying_key()),
        Err(Error::BadSignature)
    ));
}

#[test]
fn a_keys_signature_cannot_be_replayed_as_a_manifest_signature() {
    let root = key(1);
    let dir = tmpdir("replay");
    fs::write(dir.join("MANIFEST"), MANIFEST).unwrap();
    fs::write(dir.join("MANIFEST.sig"), sign_keys(&root, MANIFEST.as_bytes())).unwrap();
    assert!(matches!(
        open_signed(&dir, &root.verifying_key()),
        Err(Error::BadSignature)
    ));
}

#[test]
fn a_root_signed_but_malformed_keys_file_is_refused() {
    let (root, release) = (key(1), key(2));
    let bad = format!("not-keys\n{}\n", hex(release.verifying_key().as_bytes()));
    let dir = library("malformed", &release, Some((&bad, &root)));
    assert!(open_signed(&dir, &root.verifying_key()).is_err());
}

#[test]
fn keys_without_its_signature_is_refused() {
    let (root, release) = (key(1), key(2));
    let dir = library("nosig", &release, Some((&keys_text(&[&release]), &root)));
    fs::remove_file(dir.join("KEYS.sig")).unwrap();
    assert!(open_signed(&dir, &root.verifying_key()).is_err());
}

#[test]
fn a_tampered_manifest_is_refused() {
    let (root, release) = (key(1), key(2));
    let dir = library("manifest", &release, Some((&keys_text(&[&release]), &root)));
    fs::write(dir.join("MANIFEST"), MANIFEST.replace("v1", "v2")).unwrap();
    assert!(matches!(
        open_signed(&dir, &root.verifying_key()),
        Err(Error::BadSignature)
    ));
}

#[cfg(unix)]
#[test]
fn symlinked_trust_files_are_refused_even_when_their_content_is_valid() {
    use std::os::unix::fs::symlink;
    let root = key(1);
    for name in ["MANIFEST", "MANIFEST.sig"] {
        let good = library(&format!("good-{name}"), &root, None);
        let dir = library(&format!("link-{name}"), &root, None);
        fs::remove_file(dir.join(name)).unwrap();
        symlink(good.join(name), dir.join(name)).unwrap();
        assert!(open_signed(&dir, &root.verifying_key()).is_err(), "{name} via symlink accepted");
    }
}
