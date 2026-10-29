use alexandria_zim::{Error, Source, Zim};

struct Mem(Vec<u8>);

impl Source for Mem {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&mut self, offset: u64, len: usize) -> alexandria_zim::Result<Vec<u8>> {
        let s = offset as usize;
        let e = s + len;
        if e > self.0.len() {
            return Err(Error::Truncated);
        }
        Ok(self.0[s..e].to_vec())
    }
}

fn u16b(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}
fn u32b(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}
fn u64b(v: u64) -> [u8; 8] {
    v.to_le_bytes()
}

struct Dirent {
    mime: u16,
    ns: u8,
    cluster: u32,
    blob: u32,
    url: &'static str,
    title: &'static str,
}

fn dirent(d: &Dirent) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&u16b(d.mime));
    v.push(0);
    v.push(d.ns);
    v.extend_from_slice(&u32b(1));
    v.extend_from_slice(&u32b(d.cluster));
    if d.mime != 0xFFFF {
        v.extend_from_slice(&u32b(d.blob));
    }
    v.extend_from_slice(d.url.as_bytes());
    v.push(0);
    v.extend_from_slice(d.title.as_bytes());
    v.push(0);
    v
}

fn cluster(blobs: &[&[u8]], compression: u8, zstd: bool) -> Vec<u8> {
    let n = blobs.len();
    let first = 4 * (n + 1);
    let mut plain = Vec::new();
    let mut at = first as u32;
    for b in blobs {
        plain.extend_from_slice(&u32b(at));
        at += b.len() as u32;
    }
    plain.extend_from_slice(&u32b(at));
    for b in blobs {
        plain.extend_from_slice(b);
    }
    let payload = if zstd {
        ruzstd::encoding::compress_to_vec(&plain[..], ruzstd::encoding::CompressionLevel::Fastest)
    } else {
        plain
    };
    let mut out = vec![compression];
    out.extend_from_slice(&payload);
    out
}

fn build(zstd: bool, compression: u8) -> Vec<u8> {
    build_titled(zstd, compression, "Beta")
}

fn build_titled(zstd: bool, compression: u8, beta_title: &'static str) -> Vec<u8> {
    let mimes = b"text/html\0\0".to_vec();

    let d0 = dirent(&Dirent {
        mime: 0,
        ns: b'C',
        cluster: 0,
        blob: 0,
        url: "a",
        title: "Alpha",
    });
    let d1 = dirent(&Dirent {
        mime: 0,
        ns: b'C',
        cluster: 0,
        blob: 1,
        url: "b",
        title: beta_title,
    });
    let d2 = dirent(&Dirent {
        mime: 0xFFFF,
        ns: b'C',
        cluster: 0,
        blob: 0,
        url: "c",
        title: "See A",
    });

    let cl = cluster(
        &[b"hello", b"world"],
        if zstd { 5 } else { compression },
        zstd,
    );

    let mut out = vec![0u8; 80];
    let mime_pos = out.len() as u64;
    out.extend_from_slice(&mimes);

    let d0_pos = out.len() as u64;
    out.extend_from_slice(&d0);
    let d1_pos = out.len() as u64;
    out.extend_from_slice(&d1);
    let d2_pos = out.len() as u64;
    out.extend_from_slice(&d2);

    let cl_pos = out.len() as u64;
    out.extend_from_slice(&cl);

    let url_pos = out.len() as u64;
    for p in [d0_pos, d1_pos, d2_pos] {
        out.extend_from_slice(&u64b(p));
    }
    let title_pos = out.len() as u64;
    for i in [0u32, 1, 2] {
        out.extend_from_slice(&u32b(i));
    }
    let cluster_pos = out.len() as u64;
    out.extend_from_slice(&u64b(cl_pos));

    let checksum_pos = out.len() as u64;
    out.extend_from_slice(&[0u8; 16]);

    out[0..4].copy_from_slice(&u32b(alexandria_zim::MAGIC));
    out[4..6].copy_from_slice(&u16b(6));
    out[6..8].copy_from_slice(&u16b(1));
    out[24..28].copy_from_slice(&u32b(3));
    out[28..32].copy_from_slice(&u32b(1));
    out[32..40].copy_from_slice(&u64b(url_pos));
    out[40..48].copy_from_slice(&u64b(title_pos));
    out[48..56].copy_from_slice(&u64b(cluster_pos));
    out[56..64].copy_from_slice(&u64b(mime_pos));
    out[64..68].copy_from_slice(&u32b(0));
    out[72..80].copy_from_slice(&u64b(checksum_pos));
    out
}

#[test]
fn reads_main_page_and_articles() {
    let z = Zim::open(Mem(build(false, 0))).unwrap();
    assert_eq!(z.header.article_count, 3);
    let mut z = z;
    let main = z.main_page().unwrap();
    assert_eq!(main.url, "a");
    assert_eq!(main.title, "Alpha");
    assert_eq!(z.read_article(0).unwrap(), b"hello");
    assert_eq!(z.read_article(1).unwrap(), b"world");
}

#[test]
fn finds_by_url() {
    let mut z = Zim::open(Mem(build(false, 0))).unwrap();
    let e = z.find('C', "b").unwrap();
    assert_eq!(e.title, "Beta");
    assert_eq!(z.read_article(1).unwrap(), b"world");
    assert!(matches!(z.find('C', "zzz"), Err(Error::NotFound)));
}

#[test]
fn empty_title_means_url() {
    let mut z = Zim::open(Mem(build_titled(false, 0, ""))).unwrap();
    assert_eq!(z.find('C', "b").unwrap().title, "b");
    assert_eq!(z.entry(1).unwrap().title, "b");
    let scanned = z.scan_entries().unwrap();
    assert_eq!(scanned[1].title, "b");
    let hits = alexandria_zim::search(&mut z, "b", 5).unwrap_or_default();
    let _ = hits;
}

#[test]
fn follows_redirects() {
    let mut z = Zim::open(Mem(build(false, 0))).unwrap();
    let e = z.find('C', "c").unwrap();
    assert!(e.is_redirect());
    let final_entry = z.resolve(2).unwrap();
    assert_eq!(final_entry.mime, 0);
    assert_eq!(z.read_article(2).unwrap(), b"hello");
}

#[test]
fn reads_zstd_clusters() {
    let mut z = Zim::open(Mem(build(true, 4))).unwrap();
    assert_eq!(z.read_article(1).unwrap(), b"world");
}

#[test]
fn rejects_bad_magic_and_versions() {
    let mut b = build(false, 0);
    b[0] ^= 1;
    assert!(matches!(Zim::open(Mem(b)), Err(Error::BadMagic(_))));

    let mut b = build(false, 0);
    b[4] = 9;
    assert!(matches!(Zim::open(Mem(b)), Err(Error::BadVersion(9))));
}

#[test]
fn rejects_unsupported_compression() {
    let mut z = Zim::open(Mem(build(false, 3))).unwrap();
    assert!(matches!(z.read_article(0), Err(Error::BadCluster(3))));
}

#[test]
fn rejects_truncated_file() {
    let mut b = build(false, 0);
    b.truncate(60);
    assert!(matches!(Zim::open(Mem(b)), Err(Error::Truncated)));
}

#[test]
fn rejects_redirect_loop() {
    let mut b = build(false, 0);
    let d0 = 91usize;
    b[d0..d0 + 2].copy_from_slice(&u16b(0xFFFF));
    b[d0 + 8..d0 + 12].copy_from_slice(&u32b(0));
    let mut z = Zim::open(Mem(b)).unwrap();
    assert!(matches!(z.read_article(0), Err(Error::RedirectLoop)));
}

#[test]
fn reads_only_through_a_verified_source() {
    use alexandria_core::{write_hashes, Entry, VerifiedFile};
    use std::fs;

    let dir = std::env::temp_dir().join(format!("zim-verified-{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("test.zim");
    fs::write(&path, build(false, 0)).unwrap();
    let hashes = write_hashes(&path, 4096).unwrap();
    let raw = fs::read(&hashes).unwrap();
    use sha2::{Digest, Sha256};
    let mut h = [0u8; 32];
    h.copy_from_slice(&Sha256::digest(&raw));
    let entry = Entry {
        hashes_sha256: h,
        size: fs::metadata(&path).unwrap().len(),
        path: "test.zim".into(),
    };
    let vf = VerifiedFile::open(&path, &entry, 4096).unwrap();
    let mut z = Zim::open(vf).unwrap();
    assert_eq!(z.read_article(0).unwrap(), b"hello");

    use std::io::{Seek, SeekFrom, Write};
    let mut f = fs::OpenOptions::new().write(true).open(&path).unwrap();
    f.seek(SeekFrom::Start(10)).unwrap();
    f.write_all(&[0xFF]).unwrap();
    f.sync_all().unwrap();
    drop(f);
    let vf = VerifiedFile::open(&path, &entry, 4096).unwrap();
    assert!(Zim::open(vf).is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn scans_every_entry_in_order() {
    let mut z = Zim::open(Mem(build(false, 0))).unwrap();
    let scanned = z.scan_entries().unwrap();
    assert_eq!(scanned.len(), z.header.article_count as usize);
    for i in 0..z.header.article_count {
        let one = z.entry(i).unwrap();
        assert_eq!(scanned[i as usize].url, one.url);
        assert_eq!(scanned[i as usize].title, one.title);
    }
}
