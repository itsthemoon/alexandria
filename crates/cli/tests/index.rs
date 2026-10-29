use std::path::PathBuf;
use std::process::Command;

const MAGIC: u32 = 0x044D_495A;

fn u16b(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}
fn u32b(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}
fn u64b(v: u64) -> [u8; 8] {
    v.to_le_bytes()
}

fn dirent(mime: u16, cluster: u32, blob: u32, url: &str, title: &str) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&u16b(mime));
    v.push(0);
    v.push(b'C');
    v.extend_from_slice(&u32b(1));
    v.extend_from_slice(&u32b(cluster));
    if mime != 0xFFFF {
        v.extend_from_slice(&u32b(blob));
    }
    v.extend_from_slice(url.as_bytes());
    v.push(0);
    v.extend_from_slice(title.as_bytes());
    v.push(0);
    v
}

fn cluster(blobs: &[Vec<u8>]) -> Vec<u8> {
    let n = blobs.len();
    let mut plain = Vec::new();
    let mut at = (4 * (n + 1)) as u32;
    for b in blobs {
        plain.extend_from_slice(&u32b(at));
        at += b.len() as u32;
    }
    plain.extend_from_slice(&u32b(at));
    for b in blobs {
        plain.extend_from_slice(b);
    }
    let mut out = vec![0u8];
    out.extend_from_slice(&plain);
    out
}

fn zim_with(articles: usize) -> Vec<u8> {
    let body = |i: usize| {
        format!(
            "<p>Article {i} covers a subject in enough words that it is kept by the indexer, \
             with detail about the same subject repeated for length.</p>"
        )
        .into_bytes()
    };
    let mut dirents = Vec::new();
    let mut blobs = Vec::new();
    for i in 0..articles {
        dirents.push(dirent(
            0,
            0,
            i as u32,
            &format!("a{i:05}"),
            &format!("Article {i:05}"),
        ));
        blobs.push(body(i));
    }

    let mimes = b"text/html\0\0".to_vec();
    let mut out = vec![0u8; 80];
    let mime_pos = out.len() as u64;
    out.extend_from_slice(&mimes);
    let mut dirent_pos = Vec::new();
    for d in &dirents {
        dirent_pos.push(out.len() as u64);
        out.extend_from_slice(d);
    }
    let cl_pos = out.len() as u64;
    out.extend_from_slice(&cluster(&blobs));
    let url_pos = out.len() as u64;
    for p in &dirent_pos {
        out.extend_from_slice(&u64b(*p));
    }
    let title_pos = out.len() as u64;
    for i in 0..articles {
        out.extend_from_slice(&u32b(i as u32));
    }
    let cluster_pos = out.len() as u64;
    out.extend_from_slice(&u64b(cl_pos));
    let checksum_pos = out.len() as u64;
    out.extend_from_slice(&[0u8; 16]);

    out[0..4].copy_from_slice(&u32b(MAGIC));
    out[4..6].copy_from_slice(&u16b(6));
    out[6..8].copy_from_slice(&u16b(1));
    out[24..28].copy_from_slice(&u32b(articles as u32));
    out[28..32].copy_from_slice(&u32b(1));
    out[32..40].copy_from_slice(&u64b(url_pos));
    out[40..48].copy_from_slice(&u64b(title_pos));
    out[48..56].copy_from_slice(&u64b(cluster_pos));
    out[56..64].copy_from_slice(&u64b(mime_pos));
    out[64..68].copy_from_slice(&u32b(0));
    out[72..80].copy_from_slice(&u64b(checksum_pos));
    out
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("alex-index-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn indexed(stdout: &str) -> usize {
    let line = stdout
        .lines()
        .find(|l| l.starts_with("indexed "))
        .expect("no summary line");
    line.trim_start_matches("indexed ")
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

fn run(zim: &PathBuf, out: &PathBuf, stop: Option<usize>) -> (bool, String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_alex"));
    c.arg("index").arg(zim).arg(out);
    c.env_remove("ALEX_INDEX_STOP_AFTER");
    if let Some(n) = stop {
        c.env("ALEX_INDEX_STOP_AFTER", n.to_string());
    }
    let o = c.output().unwrap();
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).to_string(),
        String::from_utf8_lossy(&o.stderr).to_string(),
    )
}

#[test]
fn an_interrupted_index_finishes_on_the_next_run_without_duplicates() {
    let d = dir("resume");
    let zim = d.join("pack.zim");
    std::fs::write(&zim, zim_with(40)).unwrap();

    let clean = d.join("clean.search.db");
    let (ok, out, _) = run(&zim, &clean, None);
    assert!(ok, "clean run failed");
    let all = indexed(&out);
    assert_eq!(all, 40);

    let part = d.join("part.search.db");
    let (ok, _, _) = run(&zim, &part, Some(5));
    assert!(!ok, "the interrupted run should fail");
    assert!(part.is_file(), "the partial database must survive the failure");

    let (ok, out, err) = run(&zim, &part, None);
    assert!(ok, "the resumed run failed");
    assert!(
        err.contains("resuming after 5"),
        "the second run must continue the partial index, not start over: {err}"
    );
    assert_eq!(
        indexed(&out),
        all,
        "the resumed index must hold every article exactly once"
    );

    let (ok, out, err) = run(&zim, &part, None);
    assert!(ok);
    assert_eq!(indexed(&out), all, "a finished index is rebuilt, not extended");
    assert!(!err.contains("resuming"), "a finished index starts over: {err}");
    std::fs::remove_dir_all(&d).ok();
}
