use crate::block::{compress_block, BGZF_EOF, MAX_UNCOMPRESSED_BLOCK};
use anyhow::{bail, Context, Result};
use crc32fast::Hasher;
use libdeflater::Decompressor;
use rayon::prelude::*;
use std::io::{Read, Write};

pub type GziEntry = (u64, u64);

pub fn write_gzi<W: Write>(mut writer: W, entries: &[GziEntry]) -> Result<()> {
    writer.write_all(&(entries.len() as u64).to_le_bytes())?;
    for &(compressed, uncompressed) in entries {
        writer.write_all(&compressed.to_le_bytes())?;
        writer.write_all(&uncompressed.to_le_bytes())?;
    }
    Ok(())
}

/// Phase 3: count==0 (8 bytes total) is a valid empty index.
pub fn read_gzi<R: Read>(mut reader: R) -> Result<Vec<GziEntry>> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    if bytes.len() < 8 {
        bail!("truncated .gzi header");
    }
    let count = u64::from_le_bytes(bytes[0..8].try_into()?) as usize;
    if bytes.len() != 8 + count.checked_mul(16).context(".gzi entry count overflow")? {
        bail!("invalid .gzi length");
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut entries = Vec::with_capacity(count);
    let mut previous = (0, 0);
    for i in 0..count {
        let offset = 8 + i * 16;
        let entry = (
            u64::from_le_bytes(bytes[offset..offset + 8].try_into()?),
            u64::from_le_bytes(bytes[offset + 8..offset + 16].try_into()?),
        );
        if entry.0 <= previous.0 || entry.1 <= previous.1 {
            bail!(".gzi entries are not strictly increasing");
        }
        previous = entry;
        entries.push(entry);
    }
    Ok(entries)
}

/// Read up to MAX_UNCOMPRESSED_BLOCK bytes with few large reads.
///
/// Allocates one block-sized buffer and fills it directly, avoiding the
/// per-byte overhead of `Take::read_to_end` and geometric reallocation.
fn read_chunk(input: &mut impl Read) -> Result<Option<Vec<u8>>> {
    let mut buf = vec![0u8; MAX_UNCOMPRESSED_BLOCK];
    let mut filled = 0usize;
    while filled < buf.len() {
        match input.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    if filled == 0 {
        return Ok(None);
    }
    buf.truncate(filled);
    Ok(Some(buf))
}

/// Phase 6a: Streaming compression with a bounded pipeline.
///
/// Reads input in batches of `batch_size` chunks, compresses each batch in
/// parallel (when threads > 1), and writes the resulting blocks immediately.
/// Peak memory is bounded to roughly `batch_size * MAX_UNCOMPRESSED_BLOCK`.
pub fn compress<R: Read, W: Write>(
    mut input: R,
    mut output: W,
    level: u32,
    threads: usize,
) -> Result<()> {
    let batch_size = threads.max(1) * 4;
    // Build the pool once, outside the batch loop.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads.max(1))
        .build()?;

    loop {
        // Read up to batch_size chunks.
        let mut chunks: Vec<Vec<u8>> = Vec::with_capacity(batch_size);
        for _ in 0..batch_size {
            match read_chunk(&mut input)? {
                None => break,
                Some(chunk) => chunks.push(chunk),
            }
        }
        if chunks.is_empty() {
            break;
        }

        // Compress in parallel.
        let blocks: Vec<Vec<u8>> = pool.install(|| {
            chunks
                .par_iter()
                .map(|c| compress_block(c, level))
                .collect::<Result<Vec<_>>>()
        })?;

        for block in &blocks {
            output.write_all(block)?;
        }
    }

    output.write_all(&BGZF_EOF)?;
    Ok(())
}

/// Phase 6a: Streaming indexed compression.
///
/// Same bounded pipeline as `compress`, but accumulates `GziEntry` records
/// as blocks are written, suitable for creating `.gzi` indexes.
pub fn compress_indexed<R: Read, W: Write>(
    mut input: R,
    mut output: W,
    level: u32,
    threads: usize,
) -> Result<Vec<GziEntry>> {
    let batch_size = threads.max(1) * 4;
    // Build the pool once, outside the batch loop.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads.max(1))
        .build()?;

    let mut entries = Vec::new();
    let mut compressed_offset = 0u64;
    let mut uncompressed_offset = 0u64;
    // Track whether we have written any block at all (the first block's entry is implicit).
    let mut block_number = 0usize;

    loop {
        // Read up to batch_size chunks.
        let mut chunks: Vec<Vec<u8>> = Vec::with_capacity(batch_size);
        for _ in 0..batch_size {
            match read_chunk(&mut input)? {
                None => break,
                Some(chunk) => chunks.push(chunk),
            }
        }
        if chunks.is_empty() {
            break;
        }

        // Compress in parallel.
        let blocks: Vec<Vec<u8>> = pool.install(|| {
            chunks
                .par_iter()
                .map(|c| compress_block(c, level))
                .collect::<Result<Vec<_>>>()
        })?;

        for (i, block) in blocks.iter().enumerate() {
            // The first block's (0, 0) entry is implicit in .gzi; push entry *before*
            // writing every block after the first.
            if block_number != 0 {
                entries.push((compressed_offset, uncompressed_offset));
            }
            output.write_all(block)?;
            compressed_offset += block.len() as u64;
            uncompressed_offset += chunks[i].len() as u64;
            block_number += 1;
        }
    }

    output.write_all(&BGZF_EOF)?;
    Ok(entries)
}

// ── Phase 6b: Block-by-block streaming decompression ─────────────────────────
// Optimized: single-pass parse, two reads per block (header + rest), decode
// directly from the payload slice. No reconstructed-block copy and no second
// header parse. ISIZE is checked before allocating the output buffer.

/// Find the BC subfield (BSIZE) in a full extra field.
fn find_bsize(extra: &[u8]) -> Result<usize> {
    let mut cursor = 0usize;
    let mut size = None;
    while cursor < extra.len() {
        if cursor + 4 > extra.len() {
            bail!("malformed BGZF extra field");
        }
        let slen = u16::from_le_bytes([extra[cursor + 2], extra[cursor + 3]]) as usize;
        if cursor + 4 + slen > extra.len() {
            bail!("malformed BGZF subfield");
        }
        if &extra[cursor..cursor + 2] == b"BC" && slen == 2 {
            size = Some(u16::from_le_bytes([extra[cursor + 4], extra[cursor + 5]]) as usize + 1);
        }
        cursor += 4 + slen;
    }
    size.context("missing BGZF BC subfield")
}

/// Read and validate one BGZF block.
///
/// Returns `None` only on clean EOF (zero bytes available). Returns
/// `Some((data, compressed_size))` otherwise; `data` is empty for EOF-marker
/// blocks (ISIZE == 0), which callers treat as no-ops while still advancing
/// compressed offsets. Errors on truncated or malformed data.
fn read_decoded_block(r: &mut impl Read) -> Result<Option<(Vec<u8>, usize)>> {
    // Probe for clean EOF without consuming a partial block silently: a zero
    // first-byte read means end of stream; any later short read is truncation.
    let mut first = [0u8; 1];
    if r.read(&mut first)? == 0 {
        return Ok(None);
    }
    let mut header = [0u8; 18];
    header[0] = first[0];
    r.read_exact(&mut header[1..])
        .context("truncated BGZF block header")?;
    if header[0..4] != [31, 139, 8, 4] {
        bail!("not a BGZF block");
    }
    let xlen = u16::from_le_bytes([header[10], header[11]]) as usize;
    if xlen < 6 {
        bail!("BGZF xlen too small");
    }
    let extra_remaining = xlen - 6;

    // Fast path: bgzip always writes xlen == 6 with BC first, so BSIZE sits at
    // header bytes 16..18 and no extra read is needed.
    let bsize = if extra_remaining == 0 {
        u16::from_le_bytes([header[16], header[17]]) as usize + 1
    } else {
        let mut extra_tail = vec![0u8; extra_remaining];
        r.read_exact(&mut extra_tail)
            .context("truncated BGZF extra field")?;
        let mut extra_full = Vec::with_capacity(xlen);
        extra_full.extend_from_slice(&header[12..18]);
        extra_full.extend_from_slice(&extra_tail);
        find_bsize(&extra_full)?
    };
    if bsize < 26 {
        bail!("truncated or invalid BGZF block size");
    }
    let header_len = 10 + 2 + xlen;
    let payload_len = bsize
        .checked_sub(header_len + 8)
        .context("invalid BSIZE in BGZF block")?;

    let mut payload = vec![0u8; payload_len];
    r.read_exact(&mut payload)
        .context("truncated BGZF block payload")?;
    let mut trailer = [0u8; 8];
    r.read_exact(&mut trailer)
        .context("truncated BGZF block trailer")?;
    let expected_crc = u32::from_le_bytes(trailer[0..4].try_into()?);
    let expected_size = u32::from_le_bytes(trailer[4..8].try_into()?) as usize;
    if expected_size > 65_536 {
        bail!("BGZF ISIZE exceeds 65536");
    }
    if expected_size == 0 {
        // EOF marker: no-op, but report its compressed size so index offsets
        // past mid-stream markers stay correct.
        return Ok(Some((Vec::new(), bsize)));
    }

    // The output size (ISIZE) is known before decoding, so libdeflate gets an
    // exactly-sized buffer: no guessing, no reallocation.
    let mut decoded = vec![0u8; expected_size];
    let n = Decompressor::new()
        .deflate_decompress(&payload, &mut decoded)
        .context("invalid deflate payload")?;
    if n != expected_size {
        bail!("BGZF ISIZE mismatch");
    }
    let mut crc = Hasher::new();
    crc.update(&decoded);
    if crc.finalize() != expected_crc {
        bail!("BGZF CRC32 mismatch");
    }
    Ok(Some((decoded, bsize)))
}

/// Phase 6b: Streaming decompression.
///
/// Reads one BGZF block at a time; writes decompressed bytes immediately.
/// Peak memory is bounded to one uncompressed block (~65 KiB).
pub fn decompress<R: Read, W: Write>(mut input: R, mut output: W) -> Result<()> {
    loop {
        match read_decoded_block(&mut input)? {
            None => break,
            Some((data, _)) => {
                if !data.is_empty() {
                    output.write_all(&data)?;
                }
            }
        }
    }
    Ok(())
}

pub fn test<R: Read>(mut input: R) -> Result<()> {
    // Streaming integrity check: validate each block without accumulating
    // the full decompressed output (bounded ~65 KiB instead of input size).
    while read_decoded_block(&mut input)?.is_some() {}
    Ok(())
}

pub fn reindex<R: Read>(mut input: R) -> Result<Vec<GziEntry>> {
    // Single-pass streaming scan: validate each block while recording offsets.
    // No whole-file buffering; peak memory is one block.
    let mut entries = Vec::new();
    let mut compressed: u64 = 0;
    let mut uncompressed: u64 = 0;
    let mut is_first = true;
    loop {
        match read_decoded_block(&mut input)? {
            None => break,
            Some((data, csize)) => {
                if !is_first && !data.is_empty() {
                    entries.push((compressed, uncompressed));
                }
                compressed += csize as u64;
                uncompressed += data.len() as u64;
                is_first = false;
            }
        }
    }
    Ok(entries)
}

fn discard_bytes(input: &mut impl Read, mut n: u64) -> Result<()> {
    let mut buf = [0u8; 8192];
    while n > 0 {
        let want = (n.min(buf.len() as u64)) as usize;
        let got = input.read(&mut buf[..want])?;
        if got == 0 {
            bail!(".gzi compressed offset is beyond input");
        }
        n -= got as u64;
    }
    Ok(())
}

pub fn decompress_range<R: Read, W: Write>(
    mut input: R,
    mut output: W,
    offset: u64,
    size: Option<u64>,
    index: Option<&[GziEntry]>,
) -> Result<()> {
    // Streaming range: skip compressed bytes without decoding the prefix,
    // then decode block-by-block and stop as soon as `size` is fulfilled.
    // Never decodes or buffers the full tail.
    let (compressed_start, uncompressed_start) = index
        .and_then(|entries| {
            entries
                .iter()
                .copied()
                .take_while(|entry| entry.1 <= offset)
                .last()
        })
        .unwrap_or((0, 0));
    if compressed_start > 0 {
        discard_bytes(&mut input, compressed_start)?;
    }
    let mut uoff = uncompressed_start;
    let mut remaining = size.unwrap_or(u64::MAX);
    if remaining == 0 {
        // Still validate the stream prefix? No — historically a zero-size read
        // returns empty without error even at EOF; match that.
        return Ok(());
    }
    loop {
        if remaining == 0 {
            break;
        }
        match read_decoded_block(&mut input)? {
            None => break,
            Some((data, _)) => {
                if data.is_empty() {
                    continue;
                }
                let blen = data.len() as u64;
                if uoff + blen <= offset {
                    uoff += blen;
                    continue;
                }
                let skip = offset.saturating_sub(uoff) as usize;
                uoff += blen;
                if skip >= data.len() {
                    continue;
                }
                let available = &data[skip..];
                let take = (remaining.min(available.len() as u64)) as usize;
                output.write_all(&available[..take])?;
                remaining -= take as u64;
            }
        }
    }
    Ok(())
}
