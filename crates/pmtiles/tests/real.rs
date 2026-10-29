use alexandria_pmtiles::{Pmtiles, Source};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

struct FileSource {
    file: File,
    len: u64,
}

impl Source for FileSource {
    fn size(&self) -> u64 {
        self.len
    }
    fn read_at(&mut self, offset: u64, len: usize) -> alexandria_pmtiles::Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(alexandria_pmtiles::Error::Io)?;
        self.file
            .read_exact(&mut buf)
            .map_err(alexandria_pmtiles::Error::Io)?;
        Ok(buf)
    }
}

#[test]
fn reads_a_real_third_party_archive() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/stamen_z3.pmtiles");
    let file = File::open(path).unwrap();
    let len = file.metadata().unwrap().len();
    let mut p = Pmtiles::open(FileSource { file, len }).unwrap();
    assert_eq!(p.header.max_zoom, 3);
    for (z, x, y) in [(0u8, 0u32, 0u32), (2, 1, 1), (3, 4, 3)] {
        let tile = p.tile(z, x, y).unwrap();
        assert!(
            tile.starts_with(b"\x89PNG\r\n\x1a\n"),
            "tile {z}/{x}/{y} is not a PNG"
        );
        assert!(tile.len() > 1000);
    }
}
