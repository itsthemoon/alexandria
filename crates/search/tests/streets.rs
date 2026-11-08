use alexandria_core::{write_hashes, Entry, VerifiedFile};
use alexandria_search::streets::Streets;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const CH: u64 = 4096;

fn sample() -> Vec<u8> {
    let body = "alexandria-streets 1\n\
        lexington avenue\tLexington Avenue\t40.680\t-73.950\n\
        lexington avenue\tLexington Avenue\t40.730\t-73.930\n\
        chemin des revoires\tChemin des Révoires\t43.730\t7.410\n\
        rue des hauts de monte carlo\tRue des Hauts de Monte Carlo\t43.740\t7.420\n\
        broadway\tBroadway\t40.760\t-73.980\n\
        bad line without tabs\n\
        another\tAnother\tnot-a-number\t1\n";
    body.as_bytes().to_vec()
}

#[test]
fn finds_a_street_and_keeps_one_point_per_name() {
    let s = Streets::parse(&sample()).unwrap();
    assert_eq!(s.len(), 5);
    let hits = s.find("lexington", None, 10);
    assert_eq!(hits.len(), 1, "one name, one result");
    assert!(hits[0].0.eq_ignore_ascii_case("Lexington Avenue"));
}

#[test]
fn the_nearest_match_comes_first() {
    let s = Streets::parse(&sample()).unwrap();
    let near_downtown = (40.735, -73.935);
    let lexington = s.find("lexington", Some(near_downtown), 10);
    assert_eq!(lexington[0].1, 40.730, "the closer end of the avenue wins");

    let near_south = (40.675, -73.955);
    let lexington = s.find("lexington", Some(near_south), 10);
    assert_eq!(lexington[0].1, 40.680);
}

#[test]
fn accents_do_not_have_to_be_typed() {
    let s = Streets::parse(&sample()).unwrap();
    let hits = s.find("revoires", None, 5);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0.as_str(), "Chemin des Révoires");
    assert!(s.find("RÉVOIRES", None, 5).len() == 1);
}

#[test]
fn an_exact_name_beats_a_longer_one_and_limits_hold() {
    let s = Streets::parse(&sample()).unwrap();
    let hits = s.find("broadway", None, 1);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0.as_str(), "Broadway");
    assert!(s.find("b", None, 10).is_empty(), "one letter is too broad");
    assert!(s.find("zzzz", None, 10).is_empty());
}

#[test]
fn a_file_without_the_header_is_not_used() {
    assert!(Streets::parse(b"lexington avenue\tLexington Avenue\t40.6\t-73.9\n").is_none());
    assert!(Streets::parse(b"").is_none());
}

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("alex-streets-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn entry_for(path: &Path) -> Entry {
    let size = std::fs::metadata(path).unwrap().len();
    let hashes = write_hashes(path, CH).unwrap();
    let raw = std::fs::read(hashes).unwrap();
    use sha2::{Digest, Sha256};
    let mut h = [0u8; 32];
    h.copy_from_slice(&Sha256::digest(&raw));
    Entry {
        hashes_sha256: h,
        size,
        path: path.file_name().unwrap().to_str().unwrap().to_string(),
    }
}

fn write_indexed(path: &Path, version: &str, rows: &[(&str, &str, f32, f32, Option<&str>)]) {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(format!("{}\n", " ".repeat(63)).as_bytes()).unwrap();
    for (key, name, lat, lon, place) in rows {
        match place {
            Some(p) => writeln!(f, "{key}\t{name}\t{lat}\t{lon}\t{p}").unwrap(),
            None => writeln!(f, "{key}\t{name}\t{lat}\t{lon}").unwrap(),
        }
    }
    let index_at = f.stream_position().unwrap();
    writeln!(f, "{}\t64", rows[0].0).unwrap();
    let head = format!("alexandria-streets {version} {index_at} {}", rows.len());
    f.seek(SeekFrom::Start(0)).unwrap();
    f.write_all(format!("{:63}\n", head).as_bytes()).unwrap();
}

fn open_signed(dir: &Path) -> Streets {
    let path = dir.join("streets.txt");
    let entry = entry_for(&path);
    let vf = VerifiedFile::open_in(dir, &entry, CH).unwrap();
    Streets::open_verified(vf, CH).unwrap()
}

#[test]
fn an_indexed_file_answers_without_loading_it() {
    let d = tmpdir("v3");
    let path = d.join("streets.txt");
    let rows = [
        ("bedford avenue", "Bedford Avenue", 40.678, -73.953, None),
        ("bedford place", "Bedford Place", 40.679, -73.954, None),
        ("bedford street", "Bedford Street", 40.728, -74.004, None),
        ("broadway", "Broadway", 40.760, -73.980, None),
        ("lexington avenue", "Lexington Avenue", 40.730, -73.930, None),
        ("lexington avenue", "Lexington Avenue", 40.680, -73.950, None),
    ];
    write_indexed(&path, "3", &rows);

    let s = open_signed(&d);
    assert_eq!(s.len(), 6);
    let near = (40.6791, -73.9541);
    let hits = s.find("bedford", Some(near), 2);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].0.as_str(), "Bedford Place", "nearest first");
    let near_street = (40.728, -74.004);
    assert_eq!(
        s.find("bedford", Some(near_street), 1)[0].0.as_str(),
        "Bedford Street",
        "standing on it puts it first"
    );
    let lexington = s.find("lexington", Some(near), 5);
    assert_eq!(lexington.len(), 1, "one result per name");
    assert_eq!(lexington[0].1, 40.680, "the end nearest the view");
    assert!(s.find("zzz", None, 5).is_empty());
    assert_eq!(s.find("broadway", None, 5)[0].0.as_str(), "Broadway");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn a_v4_file_reads_and_carries_the_place_label() {
    let d = tmpdir("v4");
    let path = d.join("streets.txt");
    let rows = [
        ("avenue des champs-elysees", "Avenue des Champs-Élysées", 48.87, 2.31, Some("Paris, FR")),
        ("court street", "Court Street", 40.68, -73.99, Some("New York, US")),
    ];
    write_indexed(&path, "4", &rows);

    let s = open_signed(&d);
    assert_eq!(s.len(), 2);
    let hits = s.find("avenue des champs", None, 5);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0.as_str(), "Avenue des Champs-Élysées");
    assert_eq!(hits[0].3.as_deref(), Some("Paris, FR"));
    assert_eq!(s.find("court", None, 5)[0].3.as_deref(), Some("New York, US"));
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn a_v3_line_without_a_place_still_reads() {
    let d = tmpdir("v3no");
    let path = d.join("streets.txt");
    let rows = [
        ("broadway", "Broadway", 40.760, -73.980, None),
    ];
    write_indexed(&path, "3", &rows);
    let s = open_signed(&d);
    let hits = s.find("broadway", None, 5);
    assert_eq!(hits[0].3, None);
    std::fs::remove_dir_all(&d).ok();
}

const CAP: u64 = 64 * 1024 * 1024;

fn write_big_v1(path: &Path) {
    use std::io::Write;
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(b"alexandria-streets 1\n").unwrap();
    let row = b"lexington avenue\tLexington Avenue\t40.7\t-73.9\n";
    let mut total = 21u64;
    while total < CAP {
        f.write_all(row).unwrap();
        total += row.len() as u64;
    }
}

fn write_big_indexed_huge_tail(path: &Path) {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(format!("{}\n", " ".repeat(63)).as_bytes()).unwrap();
    let row = b"lexington avenue\tLexington Avenue\t40.7\t-73.9\n";
    let mut total = 64u64;
    while total < CAP {
        f.write_all(row).unwrap();
        total += row.len() as u64;
    }
    f.seek(SeekFrom::Start(0)).unwrap();
    f.write_all(format!("{:63}\n", "alexandria-streets 3 0 1").as_bytes())
        .unwrap();
}

#[test]
fn a_non_indexed_file_over_64_mib_is_refused() {
    let d = tmpdir("bigv1");
    let path = d.join("streets.txt");
    write_big_v1(&path);
    let entry = entry_for(&path);
    let vf = VerifiedFile::open_in(&d, &entry, CH).unwrap();
    assert!(Streets::open_verified(vf, CH).is_none());
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn an_index_with_a_huge_tail_is_refused() {
    let d = tmpdir("bigtail");
    let path = d.join("streets.txt");
    write_big_indexed_huge_tail(&path);
    let entry = entry_for(&path);
    let vf = VerifiedFile::open_in(&d, &entry, CH).unwrap();
    assert!(Streets::open_verified(vf, CH).is_none());
    std::fs::remove_dir_all(&d).ok();
}

fn write_no_newline_block(path: &Path) {
    use std::io::{Seek, SeekFrom, Write};
    let mut body = Vec::new();
    body.extend_from_slice(b"ab\tFirst Street\t40.7\t7.4\n");
    body.extend(std::iter::repeat(b'x').take(1_500_000));
    body.extend_from_slice(b"\n");
    body.extend_from_slice(b"ab\tSecond Street\t40.8\t7.5\n");
    let index_at = 64 + body.len() as u64;
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(format!("{}\n", " ".repeat(63)).as_bytes()).unwrap();
    f.write_all(&body).unwrap();
    f.write_all(b"ab\t64\n").unwrap();
    let head = format!("alexandria-streets 3 {index_at} 2");
    f.seek(SeekFrom::Start(0)).unwrap();
    f.write_all(format!("{:63}\n", head).as_bytes()).unwrap();
}

#[test]
fn a_block_without_a_newline_stops_after_a_megabyte() {
    let d = tmpdir("nonewline");
    let path = d.join("streets.txt");
    write_no_newline_block(&path);
    let s = open_signed(&d);
    let hits = s.find("ab", None, 5);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].0.as_str(), "First Street");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn a_name_whose_multibyte_char_is_split_at_byte_64_still_opens() {
    let d = tmpdir("mb");
    let path = d.join("streets.txt");
    let mut b = b"alexandria-streets 1\n".to_vec();
    b.extend_from_slice(b"ab\t");
    while b.len() < 63 {
        b.push(b'R');
    }
    assert_eq!(b.len(), 63);
    b.extend_from_slice("é".as_bytes());
    assert_eq!(b.len(), 65);
    b.extend_from_slice(b"\t40.7\t7.4\n");
    std::fs::write(&path, &b).unwrap();
    let entry = entry_for(&path);
    let vf = VerifiedFile::open_in(&d, &entry, CH).unwrap();
    let s = Streets::open_verified(vf, CH).expect("split multibyte name must still open");
    assert_eq!(s.find("ab", None, 5).len(), 1);
    std::fs::remove_dir_all(&d).ok();
}
