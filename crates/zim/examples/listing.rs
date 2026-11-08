use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = std::path::Path::new(&args[1]);
    let pack = &args[2];
    let pub_hex = std::fs::read_to_string(&args[3]).unwrap();

    let root = alexandria_core::parse_verifying_key(pub_hex.trim()).unwrap();
    let signed = alexandria_core::open_signed(dir, &root).unwrap();
    let m = signed.manifest;
    let entry = m.entries.iter().find(|e| e.path == *pack).unwrap();

    let t = Instant::now();
    let vf = alexandria_core::VerifiedFile::open_in(dir, entry, m.chunk).unwrap();
    let mut z = alexandria_zim::Zim::open(vf).unwrap();
    eprintln!(
        "open {:?} (titlePtrPos {:#x})",
        t.elapsed(),
        z.header.title_ptr_pos
    );

    let t = Instant::now();
    let (total, first) = z.browse("", false, 0, 10).unwrap();
    let page1 = t.elapsed();

    let t = Instant::now();
    let (_, mid) = z.browse("", false, 1_000_000, 10).unwrap();
    let page1m = t.elapsed();

    let t = Instant::now();
    let (_, last) = z.browse("", false, total.saturating_sub(10), 10).unwrap();
    let pagelast = t.elapsed();

    println!("total articles: {total}");
    println!("first 10:");
    for e in &first {
        println!("  {}", e.title);
    }
    println!("offset 1,000,000:");
    for e in &mid {
        println!("  {}", e.title);
    }
    println!("last 10:");
    for e in &last {
        println!("  {}", e.title);
    }
    eprintln!(
        "page 1 {:?} | page @1,000,000 {:?} | last page {:?}",
        page1, page1m, pagelast
    );
}
