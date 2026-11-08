use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_alex"))
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("alex-manifest-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn a_drive_that_scatters_metadata_files_still_manifests() {
    let d = temp("appledouble");
    fs::write(d.join("pack.zim"), b"content").unwrap();
    fs::write(d.join("._pack.zim"), b"resource fork").unwrap();
    fs::write(d.join(".DS_Store"), b"finder").unwrap();

    let out = Command::new(bin())
        .arg("manifest")
        .arg(&d)
        .arg("v0.2.0")
        .arg("2026-11-04")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "manifest failed: {stderr}");

    let text = fs::read_to_string(d.join("MANIFEST")).unwrap();
    assert!(text.contains("pack.zim"), "the real file is listed: {text}");
    assert!(!text.contains("._pack.zim"), "resource fork skipped: {text}");
    assert!(!text.contains(".DS_Store"), "finder file skipped: {text}");
    fs::remove_dir_all(&d).ok();
}

#[test]
fn hashes_from_a_pipe_match_the_hashes_file() {
    let d = temp("stream");
    let bytes: Vec<u8> = (0..(3 << 20) + 17).map(|i| (i % 253) as u8).collect();
    let file = d.join("pack.zim");
    fs::write(&file, &bytes).unwrap();
    assert!(Command::new(bin()).arg("hash").arg(&file).status().unwrap().success());
    let mut child = Command::new(bin())
        .arg("hash-stream")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(&bytes).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, fs::read(d.join("pack.zim.hashes")).unwrap());
    assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), bytes.len().to_string());
}
