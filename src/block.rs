//! Encoding and decoding for one BGZF block.

use anyhow::{bail, Context, Result};
use crc32fast::Hasher;
use flate2::{read::DeflateDecoder, write::DeflateEncoder, Compression};
use std::io::{Read, Write};

/// Conservative maximum used by bgzip and guaranteed to fit even when stored.
pub const MAX_UNCOMPRESSED_BLOCK: usize = 65_280;
pub const BGZF_EOF: [u8; 28] = [
    31, 139, 8, 4, 0, 0, 0, 0, 0, 255, 6, 0, 66, 67, 2, 0, 27, 0,
    3, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

pub fn compress_block(input: &[u8], level: u32) -> Result<Vec<u8>> {
    if input.len() > MAX_UNCOMPRESSED_BLOCK { bail!("input is larger than one BGZF block"); }
    let mut deflater = DeflateEncoder::new(Vec::new(), Compression::new(level));
    deflater.write_all(input)?;
    let payload = deflater.finish()?;
    let total = 18 + payload.len() + 8;
    if total > 65_536 { bail!("compressed BGZF block exceeds 64 KiB"); }

    let mut crc = Hasher::new();
    crc.update(input);
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&[31, 139, 8, 4, 0, 0, 0, 0, 0, 255, 6, 0, b'B', b'C', 2, 0]);
    out.extend_from_slice(&((total - 1) as u16).to_le_bytes());
    out.extend_from_slice(&payload);
    out.extend_from_slice(&crc.finalize().to_le_bytes());
    out.extend_from_slice(&(input.len() as u32).to_le_bytes());
    Ok(out)
}

/// Validate a concatenated BGZF stream and return its decompressed bytes.
pub fn decompress_bgzf(mut data: &[u8]) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    while !data.is_empty() {
        if data.len() < 18 || data[0..4] != [31, 139, 8, 4] { bail!("not a BGZF block"); }
        let xlen = u16::from_le_bytes([data[10], data[11]]) as usize;
        if xlen < 6 || data.len() < 12 + xlen + 8 { bail!("truncated BGZF header"); }
        let mut cursor = 12;
        let mut block_size = None;
        while cursor < 12 + xlen {
            if cursor + 4 > 12 + xlen { bail!("malformed BGZF extra field"); }
            let slen = u16::from_le_bytes([data[cursor + 2], data[cursor + 3]]) as usize;
            if cursor + 4 + slen > 12 + xlen { bail!("malformed BGZF subfield"); }
            if &data[cursor..cursor + 2] == b"BC" && slen == 2 {
                block_size = Some(u16::from_le_bytes([data[cursor + 4], data[cursor + 5]]) as usize + 1);
            }
            cursor += 4 + slen;
        }
        let size = block_size.context("missing BGZF BC subfield")?;
        if size > data.len() || size < 26 { bail!("truncated or invalid BGZF block size"); }
        let payload_start = 12 + xlen;
        let payload_end = size - 8;
        let mut decoded = Vec::new();
        DeflateDecoder::new(&data[payload_start..payload_end]).read_to_end(&mut decoded)
            .context("invalid deflate payload")?;
        let expected_crc = u32::from_le_bytes(data[payload_end..payload_end + 4].try_into()?);
        let expected_size = u32::from_le_bytes(data[payload_end + 4..size].try_into()?);
        let mut crc = Hasher::new(); crc.update(&decoded);
        if crc.finalize() != expected_crc { bail!("BGZF CRC32 mismatch"); }
        if decoded.len() as u32 != expected_size { bail!("BGZF ISIZE mismatch"); }
        result.extend_from_slice(&decoded);
        data = &data[size..];
    }
    Ok(result)
}
