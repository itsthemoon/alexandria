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

fn dirent(cluster: u32, blob: u32, url: &str, title: &str) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&u16b(0));
    v.push(0);
    v.push(b'C');
    v.extend_from_slice(&u32b(1));
    v.extend_from_slice(&u32b(cluster));
    v.extend_from_slice(&u32b(blob));
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
        dirents.push(dirent(0, i as u32, &format!("a{i:05}"), &format!("Article {i:05}")));
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
    let d = std::env::temp_dir().join(format!("alex-browse-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Mem(Vec<u8>);

impl alexandria_zim::Source for Mem {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&mut self, offset: u64, len: usize) -> alexandria_zim::Result<Vec<u8>> {
        let s = offset as usize;
        let e = s + len;
        if e > self.0.len() {
            return Err(alexandria_zim::Error::Truncated);
        }
        Ok(self.0[s..e].to_vec())
    }
}

#[test]
fn the_index_finds_a_late_dirent_and_the_zim_title_wins_over_a_tampered_index_title() {
    let d = dir("tamper-title");
    let zim_path = d.join("pack.zim");
    let zim = zim_with(50);
    std::fs::write(&zim_path, &zim).unwrap();

    let out = d.join("pack.search.db");
    let mut c = Command::new(env!("CARGO_BIN_EXE_alex"));
    let o = c.arg("index").arg(&zim_path).arg(&out).output().unwrap();
    assert!(o.status.success(), "index failed: {}", String::from_utf8_lossy(&o.stderr));

    let conn = rusqlite::Connection::open(&out).unwrap();
    conn.execute(
        "UPDATE docs SET title = 'Tampered Title' WHERE url = 'a00049'",
        [],
    )
    .unwrap();

    let (total, _, hits) = alexandria_search::browse(&conn, "article 00049", false, 0, 10).unwrap();
    assert_eq!(total, 1, "the filter must find the article through the index");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].url, "a00049");
    assert_eq!(hits[0].title, "Tampered Title", "the index title is tampered");
    drop(conn);

    let mut z = alexandria_zim::Zim::open(Mem(zim)).unwrap();
    let entry = z.lookup_url(&hits[0].url).expect("the url must resolve in the ZIM");
    assert_eq!(entry.title, "Article 00049", "the shown title must come from the ZIM");

    std::fs::remove_dir_all(&d).ok();
}
