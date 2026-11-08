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

fn build_with(mimes: &[&str], items: &[(u16, u8, &'static str, &'static [u8])], zstd: bool) -> Vec<u8> {
    let mut mime_list = Vec::new();
    for m in mimes {
        mime_list.extend_from_slice(m.as_bytes());
        mime_list.push(0);
    }
    mime_list.push(0);
    let blobs: Vec<&[u8]> = items.iter().map(|i| i.3).collect();
    let cl = cluster(&blobs, if zstd { 5 } else { 1 }, zstd);
    let mut out = vec![0u8; 80];
    let mime_pos = out.len() as u64;
    out.extend_from_slice(&mime_list);
    let mut pos = Vec::new();
    for (n, (mime, ns, url, _)) in items.iter().enumerate() {
        pos.push(out.len() as u64);
        out.extend_from_slice(&dirent(&Dirent { mime: *mime, ns: *ns, cluster: 0, blob: n as u32, url, title: url }));
    }
    let cl_pos = out.len() as u64;
    out.extend_from_slice(&cl);
    let url_pos = out.len() as u64;
    for p in &pos {
        out.extend_from_slice(&u64b(*p));
    }
    let title_pos = out.len() as u64;
    for i in 0..items.len() as u32 {
        out.extend_from_slice(&u32b(i));
    }
    let cluster_pos = out.len() as u64;
    out.extend_from_slice(&u64b(cl_pos));
    let checksum_pos = out.len() as u64;
    out.extend_from_slice(&[0u8; 16]);
    out[0..4].copy_from_slice(&u32b(alexandria_zim::MAGIC));
    out[4..6].copy_from_slice(&u16b(6));
    out[6..8].copy_from_slice(&u16b(1));
    out[24..28].copy_from_slice(&u32b(items.len() as u32));
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
fn image_sources_become_data_uris_from_the_pack() {
    let zim = build_with(
        &["text/html", "image/webp", "image/png"],
        &[
            (1, b'C', "_assets_/cat.webp", b"ABC"),
            (0, b'C', "page", b"<p>hi</p>"),
            (2, b'I', "dog.png", b"PNGDATA!"),
        ],
        false,
    );
    let mut z = Zim::open(Mem(zim)).unwrap();
    assert_eq!(z.image_data_uri("./_assets_/cat.webp", 100).as_deref(), Some("data:image/webp;base64,QUJD"));
    assert_eq!(z.image_data_uri("../I/dog.png", 100).as_deref(), Some("data:image/png;base64,UE5HREFUQSE="));
    assert_eq!(z.image_data_uri("./_assets_/cat.webp", 2), None);
    assert_eq!(z.image_data_uri("page", 100), None);
    assert_eq!(z.image_data_uri("missing.png", 100), None);
    assert_eq!(z.image_data_uri("https://example.com/cat.webp", 100), None);
}

#[test]
fn oversized_images_in_a_compressed_cluster_are_rejected_before_their_data() {
    let big: &'static [u8] = Box::leak(vec![b'x'; 100_000].into_boxed_slice());
    let zim = build_with(
        &["text/html", "image/webp"],
        &[
            (0, b'C', "page", b"<p>hi</p>"),
            (1, b'C', "_assets_/big.webp", big),
        ],
        true,
    );
    let mut z = Zim::open(Mem(zim)).unwrap();
    assert_eq!(z.image_data_uri("./_assets_/big.webp", 100), None);
    assert!(z
        .image_data_uri("./_assets_/big.webp", big.len() + 1)
        .unwrap()
        .starts_with("data:image/webp;base64,"));
}

struct TestItem {
    mime: u16,
    ns: u8,
    url: &'static str,
    title: &'static str,
    blob: &'static [u8],
}

fn build_sorted(mimes: &[&str], items: &[TestItem]) -> Vec<u8> {
    let mut mime_list = Vec::new();
    for m in mimes {
        mime_list.extend_from_slice(m.as_bytes());
        mime_list.push(0);
    }
    mime_list.push(0);

    let n = items.len();
    let mut url_order: Vec<usize> = (0..n).collect();
    url_order.sort_by_key(|&i| (items[i].ns, items[i].url));
    let mut title_order: Vec<usize> = (0..n).collect();
    title_order.sort_by_key(|&i| (items[i].ns, items[i].title));

    let mut dirents: Vec<Vec<u8>> = Vec::new();
    for &i in &url_order {
        let it = &items[i];
        let mut v = Vec::new();
        v.extend_from_slice(&u16b(it.mime));
        v.push(0);
        v.push(it.ns);
        v.extend_from_slice(&u32b(1));
        v.extend_from_slice(&u32b(0));
        if it.mime != 0xFFFF {
            v.extend_from_slice(&u32b(i as u32));
        }
        v.extend_from_slice(it.url.as_bytes());
        v.push(0);
        v.extend_from_slice(it.title.as_bytes());
        v.push(0);
        dirents.push(v);
    }

    let blobs: Vec<&[u8]> = items.iter().map(|i| i.blob).collect();
    let cl = cluster(&blobs, 1, false);

    let mut out = vec![0u8; 80];
    let mime_pos = out.len() as u64;
    out.extend_from_slice(&mime_list);
    let mut dirent_offsets = Vec::new();
    for d in &dirents {
        dirent_offsets.push(out.len() as u64);
        out.extend_from_slice(d);
    }
    let cl_pos = out.len() as u64;
    out.extend_from_slice(&cl);
    let url_pos = out.len() as u64;
    for off in &dirent_offsets {
        out.extend_from_slice(&u64b(*off));
    }
    let title_pos = out.len() as u64;
    for &i in &title_order {
        let url_index = url_order.iter().position(|&x| x == i).unwrap() as u32;
        out.extend_from_slice(&u32b(url_index));
    }
    let cluster_pos = out.len() as u64;
    out.extend_from_slice(&u64b(cl_pos));
    let checksum_pos = out.len() as u64;
    out.extend_from_slice(&[0u8; 16]);

    out[0..4].copy_from_slice(&u32b(alexandria_zim::MAGIC));
    out[4..6].copy_from_slice(&u16b(6));
    out[6..8].copy_from_slice(&u16b(1));
    out[24..28].copy_from_slice(&u32b(n as u32));
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
fn scan_entries_limit_reads_only_the_requested_count() {
    let mut z = Zim::open(Mem(build(false, 0))).unwrap();
    assert_eq!(z.scan_entries_limit(2).unwrap().len(), 2);
    assert_eq!(z.scan_entries_limit(100).unwrap().len(), 3);
    assert!(z.scan_entries_up_to(2).is_err());
}

#[test]
fn title_entry_and_browse_follow_the_title_pointer_list() {
    let zim = build_sorted(
        &["text/html", "image/webp"],
        &[
            TestItem { mime: 0, ns: b'C', url: "a", title: "Zebra", blob: b"Z" },
            TestItem { mime: 0, ns: b'C', url: "b", title: "Alpha", blob: b"A" },
            TestItem { mime: 1, ns: b'I', url: "img", title: "img", blob: b"I" },
            TestItem { mime: 0xFFFF, ns: b'C', url: "r", title: "Redirect", blob: b"" },
        ],
    );
    let mut z = Zim::open(Mem(zim)).unwrap();

    assert_eq!(z.entry(0).unwrap().title, "Zebra");
    assert_eq!(z.title_entry(0).unwrap().title, "Alpha");
    assert_eq!(z.title_entry(1).unwrap().title, "Redirect");
    assert_eq!(z.title_entry(2).unwrap().title, "Zebra");
    assert_eq!(z.title_entry(3).unwrap().title, "img");
    assert!(z.title_entry(4).is_err());

    assert_eq!(z.article_count_bound().unwrap(), 3);

    let (total, page) = z.browse("", false, 0, 10).unwrap();
    assert_eq!(total, 3);
    let titles: Vec<&str> = page.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, vec!["Alpha", "Zebra"]);

    let (_, page) = z.browse("", true, 0, 10).unwrap();
    let titles: Vec<&str> = page.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, vec!["Zebra", "Alpha"]);

    let (total, page) = z.browse("ze", false, 0, 10).unwrap();
    assert_eq!(total, 1);
    let titles: Vec<&str> = page.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, vec!["Zebra"]);
}

fn build_listing(with_listing: bool) -> Vec<u8> {
    let mimes = ["text/html", "image/webp"];
    let items = [
        TestItem { mime: 0, ns: b'C', url: "a", title: "Zebra", blob: b"Z" },
        TestItem { mime: 0, ns: b'C', url: "b", title: "Alpha", blob: b"A" },
        TestItem { mime: 1, ns: b'I', url: "img", title: "img", blob: b"I" },
        TestItem { mime: 0xFFFF, ns: b'C', url: "r", title: "Redirect", blob: b"" },
    ];
    let n = items.len();

    let mut url_order: Vec<usize> = (0..n).collect();
    url_order.sort_by_key(|&i| (items[i].ns, items[i].url));

    let mut article_title: Vec<usize> = (0..n)
        .filter(|&i| items[i].ns == b'C' && items[i].mime != 0xFFFF)
        .collect();
    article_title.sort_by_key(|&i| (items[i].ns, items[i].title));
    let listing_blob: Vec<u8> = article_title
        .iter()
        .flat_map(|&i| {
            let url_index = url_order.iter().position(|&x| x == i).unwrap() as u32;
            u32b(url_index).to_vec()
        })
        .collect();

    let mut mime_list = Vec::new();
    for m in mimes {
        mime_list.extend_from_slice(m.as_bytes());
        mime_list.push(0);
    }
    mime_list.push(0);

    let mut dirents: Vec<Vec<u8>> = Vec::new();
    for &i in &url_order {
        let it = &items[i];
        let mut v = Vec::new();
        v.extend_from_slice(&u16b(it.mime));
        v.push(0);
        v.push(it.ns);
        v.extend_from_slice(&u32b(1));
        v.extend_from_slice(&u32b(0));
        if it.mime != 0xFFFF {
            v.extend_from_slice(&u32b(i as u32));
        }
        v.extend_from_slice(it.url.as_bytes());
        v.push(0);
        v.extend_from_slice(it.title.as_bytes());
        v.push(0);
        dirents.push(v);
    }
    if with_listing {
        let mut v = Vec::new();
        v.extend_from_slice(&u16b(0));
        v.push(0);
        v.push(b'X');
        v.extend_from_slice(&u32b(1));
        v.extend_from_slice(&u32b(0));
        v.extend_from_slice(&u32b(n as u32));
        v.extend_from_slice(b"listing/titleOrdered/v1");
        v.push(0);
        v.extend_from_slice(b"listing/titleOrdered/v1");
        v.push(0);
        dirents.push(v);
    }

    let mut blobs: Vec<&[u8]> = items.iter().map(|i| i.blob).collect();
    if with_listing {
        blobs.push(&listing_blob);
    }
    let cl = cluster(&blobs, 1, false);

    let mut out = vec![0u8; 80];
    let mime_pos = out.len() as u64;
    out.extend_from_slice(&mime_list);
    let mut dirent_offsets = Vec::new();
    for d in &dirents {
        dirent_offsets.push(out.len() as u64);
        out.extend_from_slice(d);
    }
    let cl_pos = out.len() as u64;
    out.extend_from_slice(&cl);
    let url_pos = out.len() as u64;
    for off in &dirent_offsets {
        out.extend_from_slice(&u64b(*off));
    }
    let cluster_pos = out.len() as u64;
    out.extend_from_slice(&u64b(cl_pos));
    let checksum_pos = out.len() as u64;
    out.extend_from_slice(&[0u8; 16]);

    out[0..4].copy_from_slice(&u32b(alexandria_zim::MAGIC));
    out[4..6].copy_from_slice(&u16b(6));
    out[6..8].copy_from_slice(&u16b(1));
    let total = if with_listing { n + 1 } else { n };
    out[24..28].copy_from_slice(&u32b(total as u32));
    out[28..32].copy_from_slice(&u32b(1));
    out[32..40].copy_from_slice(&u64b(url_pos));
    out[40..48].copy_from_slice(&u64b(u64::MAX));
    out[48..56].copy_from_slice(&u64b(cluster_pos));
    out[56..64].copy_from_slice(&u64b(mime_pos));
    out[64..68].copy_from_slice(&u32b(0));
    out[72..80].copy_from_slice(&u64b(checksum_pos));
    out
}

#[test]
fn title_listing_v1_lists_articles_when_no_title_pointer_list() {
    let mut z = Zim::open(Mem(build_listing(true))).unwrap();
    assert_eq!(z.header.title_ptr_pos, u64::MAX);
    assert_eq!(z.title_entry(0).unwrap().title, "Alpha");
    assert_eq!(z.title_entry(1).unwrap().title, "Zebra");
    assert!(z.title_entry(2).is_err());
    assert_eq!(z.article_count_bound().unwrap(), 2);
    let (total, page) = z.browse("", false, 0, 10).unwrap();
    assert_eq!(total, 2);
    let titles: Vec<&str> = page.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, vec!["Alpha", "Zebra"]);
}

#[test]
fn no_title_order_is_an_error_not_garbage() {
    let mut z = Zim::open(Mem(build_listing(false))).unwrap();
    assert_eq!(z.header.title_ptr_pos, u64::MAX);
    assert!(matches!(z.title_entry(0), Err(Error::NoTitleOrder)));
    assert!(matches!(z.article_count_bound(), Err(Error::NoTitleOrder)));
    assert!(matches!(z.browse("", false, 0, 10), Err(Error::NoTitleOrder)));
}

#[test]
fn many_unclosed_scripts_are_dropped_without_rescanning() {
    let mut s = String::from("<p>lead</p>");
    for _ in 0..40_000 {
        s.push_str("<script>var x = 1;");
    }
    let start = std::time::Instant::now();
    let out = alexandria_zim::article_html(&s);
    let elapsed = start.elapsed();
    assert!(out.contains("lead"));
    assert!(!out.contains("var x"));
    assert!(
        elapsed.as_millis() < 5000,
        "unclosed scripts took {elapsed:?}"
    );
}

#[test]
fn a_doubled_angle_bracket_does_not_smuggle_a_tag() {
    for tag in ["script", "style", "iframe", "template"] {
        let html = format!("<p>lead</p><<{tag}>alert(1)</{tag}><p>tail</p>");
        let page = alexandria_zim::article_html(&html);
        let out = page.split("<body>").nth(1).unwrap();
        assert!(!out.contains(&format!("<{tag}")), "{tag}: {out}");
        assert!(out.contains("lead") && out.contains("tail"), "{out}");
    }
}

#[test]
fn a_void_tag_without_a_slash_does_not_swallow_the_rest_of_the_article() {
    let html = r#"<body><table class="infobox"><tr><td>Coordinates<meta typeof="mw:Extension/indicator" id="mwDA"></td></tr></table><p>The library opened in 2002.</p><link rel="x" href="y"><input type="hidden"><source src="a"><p>Second paragraph.</p></body>"#;
    let out = alexandria_zim::article_html(html);
    assert!(out.contains("The library opened in 2002."), "{out}");
    assert!(out.contains("Second paragraph."), "{out}");
    assert!(!out.contains("<meta typeof"), "{out}");
}
