use flate2::read::GzDecoder;
use std::io::Read;

pub const MAGIC: &[u8; 7] = b"PMTiles";

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    BadMagic,
    BadVersion(u8),
    Truncated,
    Decompress(String),
    NotFound,
    VectorTiles,
    Source(String),
    TooLarge,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io: {e}"),
            Error::BadMagic => write!(f, "not a PMTiles archive"),
            Error::BadVersion(v) => write!(f, "unsupported PMTiles version {v}"),
            Error::Truncated => write!(f, "truncated PMTiles"),
            Error::Decompress(e) => write!(f, "decompress: {e}"),
            Error::NotFound => write!(f, "tile not found"),
            Error::VectorTiles => write!(
                f,
                "this archive holds vector tiles, which this build cannot draw"
            ),
            Error::Source(e) => write!(f, "source: {e}"),
            Error::TooLarge => write!(f, "tile or directory exceeds the size cap"),
        }
    }
}

impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub trait Source {
    fn size(&self) -> u64;
    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>>;
}

impl Source for alexandria_core::VerifiedFile {
    fn size(&self) -> u64 {
        alexandria_core::VerifiedFile::size(self)
    }
    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        alexandria_core::VerifiedFile::read_at(self, offset, len)
            .map_err(|e| Error::Source(e.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileType {
    Mvt,
    Png,
    Jpeg,
    Webp,
    Other(u8),
}

#[derive(Debug, Clone)]
pub struct Header {
    pub root_offset: u64,
    pub root_length: u64,
    pub leaf_offset: u64,
    pub leaf_length: u64,
    pub data_offset: u64,
    pub data_length: u64,
    pub internal_compression: u8,
    pub tile_compression: u8,
    pub tile_type: u8,
    pub min_zoom: u8,
    pub max_zoom: u8,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    tile_id: u64,
    run: u64,
    offset: u64,
    length: u64,
}

pub struct Pmtiles<S: Source> {
    src: S,
    pub header: Header,
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

fn u64le(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

fn varint(b: &[u8], pos: &mut usize) -> Result<u64> {
    let mut out = 0u64;
    let mut shift = 0u32;
    loop {
        if *pos >= b.len() || shift > 63 {
            return Err(Error::Truncated);
        }
        let byte = b[*pos];
        *pos += 1;
        out |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(out);
        }
        shift += 7;
    }
}

fn zxy_to_tile_id(z: u8, x: u32, y: u32) -> Option<u64> {
    if z > 26 {
        return None;
    }
    let n = 1u64 << z;
    if (x as u64) >= n || (y as u64) >= n {
        return None;
    }
    let mut xx = x as u64;
    let mut yy = y as u64;
    let mut d = 0u64;
    let mut s = n / 2;
    while s > 0 {
        let rx = if xx & s != 0 { 1u64 } else { 0 };
        let ry = if yy & s != 0 { 1u64 } else { 0 };
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
    (((1u64 << (2 * z as u32)) - 1) / 3).checked_add(d)
}

impl<S: Source> Pmtiles<S> {
    pub fn open(mut src: S) -> Result<Pmtiles<S>> {
        if src.size() < 127 {
            return Err(Error::Truncated);
        }
        let h = src.read_at(0, 127)?;
        if &h[0..7] != MAGIC {
            return Err(Error::BadMagic);
        }
        if h[7] != 3 {
            return Err(Error::BadVersion(h[7]));
        }
        let header = Header {
            root_offset: u64le(&h, 8),
            root_length: u64le(&h, 16),
            leaf_offset: u64le(&h, 40),
            leaf_length: u64le(&h, 48),
            data_offset: u64le(&h, 56),
            data_length: u64le(&h, 64),
            internal_compression: h[97],
            tile_compression: h[98],
            tile_type: h[99],
            min_zoom: h[100],
            max_zoom: h[101],
        };
        let root_end = header
            .root_offset
            .checked_add(header.root_length)
            .ok_or(Error::Truncated)?;
        let leaf_end = header
            .leaf_offset
            .checked_add(header.leaf_length)
            .ok_or(Error::Truncated)?;
        let data_end = header
            .data_offset
            .checked_add(header.data_length)
            .ok_or(Error::Truncated)?;
        if root_end > src.size()
            || data_end > src.size()
            || (header.leaf_length > 0 && leaf_end > src.size())
        {
            return Err(Error::Truncated);
        }
        let min_lon = u64le(&h, 102) as u32 as i32 as f64 / 1e7;
        let min_lat = u64le(&h, 106) as u32 as i32 as f64 / 1e7;
        let max_lon = u64le(&h, 110) as u32 as i32 as f64 / 1e7;
        let max_lat = u64le(&h, 114) as u32 as i32 as f64 / 1e7;
        Ok(Pmtiles {
            src,
            header,
            min_lon,
            min_lat,
            max_lon,
            max_lat,
        })
    }

    pub fn tile_type(&self) -> TileType {
        match self.header.tile_type {
            1 => TileType::Mvt,
            2 => TileType::Png,
            3 => TileType::Jpeg,
            4 => TileType::Webp,
            other => TileType::Other(other),
        }
    }

    fn decode(&self, raw: Vec<u8>) -> Result<Vec<u8>> {
        match self.header.internal_compression {
            1 => Ok(raw),
            2 => {
                let mut out = Vec::new();
                GzDecoder::new(&raw[..])
                    .take((16 << 20) + 1)
                    .read_to_end(&mut out)
                    .map_err(|e| Error::Decompress(e.to_string()))?;
                if out.len() > 16 << 20 {
                    return Err(Error::TooLarge);
                }
                Ok(out)
            }
            4 => {
                let mut dec = ruzstd::decoding::StreamingDecoder::new(&raw[..])
                    .map_err(|e| Error::Decompress(format!("{e:?}")))?;
                let mut out = Vec::new();
                dec.by_ref()
                    .take((16 << 20) + 1)
                    .read_to_end(&mut out)
                    .map_err(|e| Error::Decompress(e.to_string()))?;
                if out.len() > 16 << 20 {
                    return Err(Error::TooLarge);
                }
                Ok(out)
            }
            other => Err(Error::Decompress(format!("unknown codec {other}"))),
        }
    }

    fn directory(&mut self, offset: u64, length: u64) -> Result<Vec<Entry>> {
        if length > (16 << 20) {
            return Err(Error::TooLarge);
        }
        let raw = self.src.read_at(offset, length as usize)?;
        let plain = self.decode(raw)?;
        let mut pos = 0usize;
        let count = varint(&plain, &mut pos)?;
        if count > 1_000_000 {
            return Err(Error::TooLarge);
        }
        let n = count as usize;
        let mut ids = Vec::with_capacity(n);
        let mut run = 0u64;
        for _ in 0..n {
            run = run
                .checked_add(varint(&plain, &mut pos)?)
                .ok_or(Error::Truncated)?;
            ids.push(run);
        }
        let mut runs = Vec::with_capacity(n);
        for _ in 0..n {
            runs.push(varint(&plain, &mut pos)?);
        }
        let mut lengths = Vec::with_capacity(n);
        for _ in 0..n {
            lengths.push(varint(&plain, &mut pos)?);
        }
        let mut offsets = Vec::with_capacity(n);
        for _ in 0..n {
            offsets.push(varint(&plain, &mut pos)?);
        }
        let mut out = Vec::with_capacity(n);
        let mut prev_off = 0u64;
        let mut prev_len = 0u64;
        for i in 0..n {
            let raw = offsets[i];
            let off = if raw == 0 {
                if i == 0 {
                    0
                } else {
                    prev_off.checked_add(prev_len).ok_or(Error::Truncated)?
                }
            } else {
                raw.checked_sub(1).ok_or(Error::Truncated)?
            };
            prev_off = off;
            prev_len = lengths[i];
            out.push(Entry {
                tile_id: ids[i],
                run: runs[i],
                length: lengths[i],
                offset: off,
            });
        }
        Ok(out)
    }

    pub fn tile(&mut self, z: u8, x: u32, y: u32) -> Result<Vec<u8>> {
        if z < self.header.min_zoom || z > self.header.max_zoom || z > 31 {
            return Err(Error::NotFound);
        }
        let n = 1u64 << z;
        if x as u64 >= n || y as u64 >= n {
            return Err(Error::NotFound);
        }
        let id = zxy_to_tile_id(z, x, y).ok_or(Error::NotFound)?;
        let mut dir = self.directory(self.header.root_offset, self.header.root_length)?;
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 8 {
                return Err(Error::Truncated);
            }
            match dir.binary_search_by(|e| e.tile_id.cmp(&id)) {
                Ok(i) => {
                    let e = dir[i];
                    if e.run > 0 {
                        if e.length > (16 << 20) {
                            return Err(Error::TooLarge);
                        }
                        let start = self
                            .header
                            .data_offset
                            .checked_add(e.offset)
                            .ok_or(Error::Truncated)?;
                        if e.offset.checked_add(e.length).ok_or(Error::Truncated)?
                            > self.header.data_length
                        {
                            return Err(Error::Truncated);
                        }
                        let raw = self.src.read_at(start, e.length as usize)?;
                        return self.decode_tile(raw);
                    }
                    let start = self
                        .header
                        .leaf_offset
                        .checked_add(e.offset)
                        .ok_or(Error::Truncated)?;
                    if e.offset.checked_add(e.length).ok_or(Error::Truncated)?
                        > self.header.leaf_length
                    {
                        return Err(Error::Truncated);
                    }
                    dir = self.directory(start, e.length)?;
                }
                Err(0) => return Err(Error::NotFound),
                Err(i) => {
                    let e = dir[i - 1];
                    if e.run > 0 && id < e.tile_id.saturating_add(e.run) {
                        if e.length > (16 << 20) {
                            return Err(Error::TooLarge);
                        }
                        let start = self
                            .header
                            .data_offset
                            .checked_add(e.offset)
                            .ok_or(Error::Truncated)?;
                        if e.offset.checked_add(e.length).ok_or(Error::Truncated)?
                            > self.header.data_length
                        {
                            return Err(Error::Truncated);
                        }
                        let raw = self.src.read_at(start, e.length as usize)?;
                        return self.decode_tile(raw);
                    }
                    if e.run == 0 {
                        let start = self
                            .header
                            .leaf_offset
                            .checked_add(e.offset)
                            .ok_or(Error::Truncated)?;
                        if e.offset.checked_add(e.length).ok_or(Error::Truncated)?
                            > self.header.leaf_length
                        {
                            return Err(Error::Truncated);
                        }
                        dir = self.directory(start, e.length)?;
                    } else {
                        return Err(Error::NotFound);
                    }
                }
            }
        }
    }

    fn decode_tile(&self, raw: Vec<u8>) -> Result<Vec<u8>> {
        match self.header.tile_compression {
            1 => Ok(raw),
            2 => {
                let mut out = Vec::new();
                GzDecoder::new(&raw[..])
                    .take((16 << 20) + 1)
                    .read_to_end(&mut out)
                    .map_err(|e| Error::Decompress(e.to_string()))?;
                if out.len() > 16 << 20 {
                    return Err(Error::TooLarge);
                }
                Ok(out)
            }
            4 => {
                let mut dec = ruzstd::decoding::StreamingDecoder::new(&raw[..])
                    .map_err(|e| Error::Decompress(format!("{e:?}")))?;
                let mut out = Vec::new();
                dec.by_ref()
                    .take((16 << 20) + 1)
                    .read_to_end(&mut out)
                    .map_err(|e| Error::Decompress(e.to_string()))?;
                if out.len() > 16 << 20 {
                    return Err(Error::TooLarge);
                }
                Ok(out)
            }
            other => Err(Error::Decompress(format!("unknown tile codec {other}"))),
        }
    }
}
