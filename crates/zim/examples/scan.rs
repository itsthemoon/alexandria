use std::io::{Read, Seek, SeekFrom};
use std::time::Instant;

struct F(std::fs::File, u64);
impl alexandria_zim::Source for F {
    fn size(&self) -> u64 {
        self.1
    }
    fn read_at(&mut self, offset: u64, len: usize) -> alexandria_zim::Result<Vec<u8>> {
        self.0
            .seek(SeekFrom::Start(offset))
            .map_err(|e| alexandria_zim::Error::Source(e.to_string()))?;
        let mut b = vec![0u8; len];
        self.0
            .read_exact(&mut b)
            .map_err(|e| alexandria_zim::Error::Source(e.to_string()))?;
        Ok(b)
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("path");
    let f = std::fs::File::open(&path).unwrap();
    let size = f.metadata().unwrap().len();
    let t = Instant::now();
    let mut z = alexandria_zim::Zim::open(F(f, size)).unwrap();
    eprintln!("open {:?}", t.elapsed());
    let t = Instant::now();
    let e = z.scan_entries().unwrap();
    eprintln!("scan {} entries {:?}", e.len(), t.elapsed());
    println!(
        "{}",
        e.iter()
            .take(5)
            .map(|x| x.title.clone())
            .collect::<Vec<_>>()
            .join(" | ")
    );
}
