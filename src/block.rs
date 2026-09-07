//! Encoding and decoding for one BGZF block.

use anyhow::{bail, Context, Result};
use crc32fast::Hasher;
use libdeflater::{Compressor, CompressionLvl, Decompressor};

/// Conservative maximum used by bgzip and guaranteed to fit even when stored.
pub const MAX_UNCOMPRESSED_BLOCK: usize = 65_280;
pub const BGZF_EOF: [u8; 28] = [
    31, 139, 8, 4, 0, 0, 0, 0, 0, 255, 6, 0, 66, 67, 2, 0, 27, 0,
    3, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

pub fn compress_block(input: &[u8], level: u32) -> Result<Vec<u8>> {
    if input.len() > MAX_UNCOMPRESSED_BLOCK { bail!("input is larger than one BGZF block"); }
    // Level 0 is stored (no compression): bypass the deflate encoder entirely.
    // A single raw stored sub-block holds up to 65535 bytes, so any BGZF chunk
    // fits. Output remains valid deflate accepted by bgzip/inflate.
    if level == 0 {
        return Ok(compress_block_stored(input));
    }
    // libdeflate levels are 1-12; map tarabg/bgzip levels 1:1 so the same
    // flag means the same effort class in both tools.
    if level < 1 || level > 9 {
        bail!("invalid compression level {level}");
    }
    let payload = deflate_compress_fresh(input, level)?;
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

// Compress one block with a fresh libdeflate compressor.
//
// (A thread-local cached compressor was benchmarked and showed no measurable
// gain — per-block setup is lost in the noise — so the simpler fresh
// construction stays.)
fn deflate_compress_fresh(input: &[u8], level: u32) -> Result<Vec<u8>> {
    // libdeflate levels are 1-12; map tarabg/bgzip levels 1:1 so the same
    // flag means the same effort class in both tools.
    let lvl = CompressionLvl::new(level as i32)
        .map_err(|_| anyhow::anyhow!("invalid compression level {level}"))?;
    let mut compressor = Compressor::new(lvl);
    let bound = compressor.deflate_compress_bound(input.len());
    let mut payload = vec![0u8; bound];
    let n = compressor
        .deflate_compress(input, &mut payload)
        .context("deflate compression failed")?;
    payload.truncate(n);
    Ok(payload)
}
/// Level-0 stored block: raw deflate `BFINAL=1, BTYPE=00` sub-block.
/// Validated to inflate to exactly `input` with any compliant decoder.
fn compress_block_stored(input: &[u8]) -> Vec<u8> {
    debug_assert!(input.len() <= MAX_UNCOMPRESSED_BLOCK);
    // payload = 5-byte stored header + raw bytes.
    let total = 18 + 5 + input.len() + 8;
    let mut crc = Hasher::new();
    crc.update(input);
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&[31, 139, 8, 4, 0, 0, 0, 0, 0, 255, 6, 0, b'B', b'C', 2, 0]);
    out.extend_from_slice(&((total - 1) as u16).to_le_bytes());
    // Stored deflate block, final.
    out.push(0x01);
    let len = input.len() as u16;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(input);
    out.extend_from_slice(&crc.finalize().to_le_bytes());
    out.extend_from_slice(&(input.len() as u32).to_le_bytes());
    out
}
/// Inflate exactly `expected_size` bytes from a raw deflate payload.
///
/// libdeflate needs the output buffer up front; every BGZF caller knows ISIZE
/// before decoding, so there is no guessing and no reallocation.
fn inflate_exact(payload: &[u8], expected_size: usize) -> Result<Vec<u8>> {
    let mut out = vec![0u8; expected_size];
    let n = Decompressor::new()
        .deflate_decompress(payload, &mut out)
        .context("invalid deflate payload")?;
    if n != expected_size {
        bail!("BGZF ISIZE mismatch");
    }
    Ok(out)
}
/// Validate a concatenated BGZF stream and return its decompressed bytes.
///
/// Kept as the reference whole-buffer validator (unit tests exercise it
/// directly); the streaming paths in `bgzf.rs` implement the same checks
/// block-by-block without buffering the full output.
/// Phase 5 hardening:
/// - Rejects ISIZE > 65536.
/// - Skips EOF marker blocks (ISIZE == 0 with zero-length deflate) mid-stream
///   instead of erroring, matching the spec which allows them as no-ops.
#[allow(dead_code)]
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
        let expected_crc = u32::from_le_bytes(data[payload_end..payload_end + 4].try_into()?);
        let expected_size = u32::from_le_bytes(data[payload_end + 4..size].try_into()?) as usize;
        // Phase 5: reject blocks claiming more than 65536 uncompressed bytes
        // before allocating the output buffer.
        if expected_size > 65_536 { bail!("BGZF ISIZE exceeds 65536"); }
        let decoded = inflate_exact(&data[payload_start..payload_end], expected_size)?;
        let mut crc = Hasher::new(); crc.update(&decoded);
        if crc.finalize() != expected_crc { bail!("BGZF CRC32 mismatch"); }
        // EOF marker (ISIZE == 0, empty deflate) may appear mid-stream; treat as no-op.
        result.extend_from_slice(&decoded);
        data = &data[size..];
    }
    Ok(result)
}
