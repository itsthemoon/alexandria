use alexandria_pmtiles::{Error, Pmtiles, Source};

struct Mem(Vec<u8>);

impl Source for Mem {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&mut self, offset: u64, len: usize) -> alexandria_pmtiles::Result<Vec<u8>> {
        let s = offset as usize;
        let e = s + len;
        if e > self.0.len() {
            return Err(Error::Truncated);
        }
        Ok(self.0[s..e].to_vec())
    }
}

fn varint(mut v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            return out;
        }
    }
}

fn zxy_to_id(z: u8, x: u32, y: u32) -> u64 {
    let n = 1u64 << z;
    let (mut xx, mut yy) = (x as u64, y as u64);
    let mut d = 0u64;
    let mut s = n / 2;
    while s > 0 {
        let rx = (xx & s > 0) as u64;
        let ry = (yy & s > 0) as u64;
        d += s * s * ((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                xx = n - 1 - xx;
                yy = n - 1 - yy;
            }
            std::mem::swap(&mut xx, &mut yy);
        }
        s /= 2;
    }
    (4u64.pow(z as u32) - 1) / 3 + d
}

struct Tile {
    z: u8,
    x: u32,
    y: u32,
    body: Vec<u8>,
}

fn tiles() -> Vec<Tile> {
    let mut out = Vec::new();
    for z in 0..3u8 {
        let n = 1u32 << z;
        for y in 0..n {
            for x in 0..n {
                out.push(Tile {
                    z,
                    x,
                    y,
                    body: format!("tile-{z}-{x}-{y}").into_bytes(),
                });
            }
        }
    }
    out.sort_by_key(|t| zxy_to_id(t.z, t.x, t.y));
    out
}

fn build(tile_type: u8) -> Vec<u8> {
    let tiles = tiles();
    let mut root = Vec::new();
    root.extend(varint(tiles.len() as u64));
    let mut prev = 0u64;
    for t in &tiles {
        let id = zxy_to_id(t.z, t.x, t.y);
        root.extend(varint(id - prev));
        prev = id;
    }
    for _ in &tiles {
        root.extend(varint(1));
    }
    for t in &tiles {
        root.extend(varint(t.body.len() as u64));
    }
    let mut off = 0u64;
    for t in &tiles {
        root.extend(varint(off + 1));
        off += t.body.len() as u64;
    }

    let root_off = 127u64;
    let data_off = root_off + root.len() as u64;
    let mut out = vec![0u8; 127];
    out[0..7].copy_from_slice(b"PMTiles");
    out[7] = 3;
    out[8..16].copy_from_slice(&root_off.to_le_bytes());
    out[16..24].copy_from_slice(&(root.len() as u64).to_le_bytes());
    out[56..64].copy_from_slice(&data_off.to_le_bytes());
    out[64..72].copy_from_slice(&off.to_le_bytes());
    out[72..80].copy_from_slice(&(tiles.len() as u64).to_le_bytes());
    out[96] = 1;
    out[97] = 1;
    out[98] = 1;
    out[99] = tile_type;
    out[100] = 0;
    out[101] = 2;
    out[102..106].copy_from_slice(&(-180_0000000i32).to_le_bytes());
    out[106..110].copy_from_slice(&(-85_0000000i32).to_le_bytes());
    out[110..114].copy_from_slice(&(180_0000000i32).to_le_bytes());
    out[114..118].copy_from_slice(&(85_0000000i32).to_le_bytes());
    out.extend_from_slice(&root);
    for t in &tiles {
        out.extend_from_slice(&t.body);
    }
    out
}

#[test]
fn reads_tiles_at_every_zoom() {
    let mut p = Pmtiles::open(Mem(build(2))).unwrap();
    assert_eq!(p.tile(0, 0, 0).unwrap(), b"tile-0-0-0");
    assert_eq!(p.tile(1, 0, 1).unwrap(), b"tile-1-0-1");
    assert_eq!(p.tile(1, 1, 0).unwrap(), b"tile-1-1-0");
    assert_eq!(p.tile(2, 3, 2).unwrap(), b"tile-2-3-2");
    assert_eq!(p.tile(2, 0, 3).unwrap(), b"tile-2-0-3");
}

#[test]
fn missing_tiles_are_not_found() {
    let mut p = Pmtiles::open(Mem(build(2))).unwrap();
    assert!(matches!(p.tile(3, 0, 0), Err(Error::NotFound)));
    assert!(matches!(p.tile(15, 0, 0), Err(Error::NotFound)));
    assert!(matches!(p.tile(2, 4, 0), Err(Error::NotFound)));
}

#[test]
fn serves_vector_tiles() {
    let mut p = Pmtiles::open(Mem(build(1))).unwrap();
    assert_eq!(p.tile(0, 0, 0).unwrap(), b"tile-0-0-0");
}

#[test]
fn rejects_bad_magic() {
    let mut b = build(2);
    b[0] = b'X';
    assert!(matches!(Pmtiles::open(Mem(b)), Err(Error::BadMagic)));
}
