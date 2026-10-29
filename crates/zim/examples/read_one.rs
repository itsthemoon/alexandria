use std::io::{Read, Seek, SeekFrom};
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
    let needle = std::env::args().nth(2).expect("title");
    let f = std::fs::File::open(&path).unwrap();
    let size = f.metadata().unwrap().len();
    let mut z = alexandria_zim::Zim::open(F(f, size)).unwrap();
    let entries = z.scan_entries().unwrap();
    let (idx, _) = entries
        .iter()
        .enumerate()
        .find(|(_, e)| e.title.to_lowercase().contains(&needle.to_lowercase()))
        .expect("no title");
    let html = z.read_article(idx as u32).unwrap();
    println!(
        "{}",
        alexandria_zim::html_to_text(&String::from_utf8_lossy(&html))
    );
}
