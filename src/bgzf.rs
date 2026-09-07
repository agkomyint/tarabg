use crate::block::{compress_block, BGZF_EOF, MAX_UNCOMPRESSED_BLOCK};
use anyhow::{bail, Context, Result};
use crc32fast::Hasher;
use flate2::read::MultiGzDecoder;
use libdeflater::Decompressor;
use rayon::prelude::*;
use std::io::{self, BufRead, BufReader, Read, Write};

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

/// Read one text-aware chunk. Full chunks end immediately after the last
/// newline that fits, while lines longer than a block fall back to a hard
/// split. `pending` never grows beyond roughly one block.
fn read_text_chunk(
    input: &mut impl Read,
    pending: &mut Vec<u8>,
    eof: &mut bool,
    in_header: &mut bool,
) -> Result<Option<Vec<u8>>> {
    while pending.len() < MAX_UNCOMPRESSED_BLOCK && !*eof {
        let mut buf = vec![0u8; MAX_UNCOMPRESSED_BLOCK - pending.len()];
        let n = input.read(&mut buf)?;
        if n == 0 {
            *eof = true;
        } else {
            pending.extend_from_slice(&buf[..n]);
        }
    }
    if pending.is_empty() {
        return Ok(None);
    }
    if *in_header {
        let mut line_start = 0usize;
        let mut header_end = 0usize;
        while let Some(relative_end) = pending[line_start..].iter().position(|&byte| byte == b'\n')
        {
            let line_end = line_start + relative_end + 1;
            if matches!(pending.get(line_start), Some(b'#' | b'@')) {
                header_end = line_end;
                line_start = line_end;
            } else {
                *in_header = false;
                if header_end > 0 {
                    return Ok(Some(pending.drain(..header_end).collect()));
                }
                break;
            }
        }
        if line_start == 0 && !matches!(pending.first(), Some(b'#' | b'@')) {
            *in_header = false;
        }
    }
    let take = if pending.len() < MAX_UNCOMPRESSED_BLOCK {
        pending.len()
    } else {
        pending
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(MAX_UNCOMPRESSED_BLOCK, |at| at + 1)
    };
    Ok(Some(pending.drain(..take).collect()))
}

fn input_looks_text(input: &mut impl BufRead) -> Result<bool> {
    let sample = input.fill_buf()?;
    if sample.is_empty() || sample.contains(&0) {
        return Ok(false);
    }
    Ok(std::str::from_utf8(sample).is_ok())
}

fn next_chunk(
    input: &mut impl Read,
    text: bool,
    pending: &mut Vec<u8>,
    eof: &mut bool,
    in_header: &mut bool,
) -> Result<Option<Vec<u8>>> {
    if text {
        read_text_chunk(input, pending, eof, in_header)
    } else {
        read_chunk(input)
    }
}

/// Phase 6a: Streaming compression with a bounded pipeline.
///
/// Reads input in batches of `batch_size` chunks, compresses each batch in
/// parallel (when threads > 1), and writes the resulting blocks immediately.
/// Peak memory is bounded to roughly `batch_size * MAX_UNCOMPRESSED_BLOCK`.
pub fn compress<R: Read, W: Write>(input: R, output: W, level: u32, threads: usize) -> Result<()> {
    compress_mode(input, output, level, threads, false)
}

/// Compress with lightweight text detection and newline-aware block endings.
/// Binary callers should use `compress`, which always uses fixed-size chunks.
pub fn compress_auto<R: Read, W: Write>(
    input: R,
    output: W,
    level: u32,
    threads: usize,
) -> Result<()> {
    let mut input = BufReader::new(input);
    let text = input_looks_text(&mut input)?;
    compress_mode(input, output, level, threads, text)
}

fn compress_mode<R: Read, W: Write>(
    mut input: R,
    mut output: W,
    level: u32,
    threads: usize,
    text: bool,
) -> Result<()> {
    let batch_size = threads.max(1) * 4;
    // Build the pool once, outside the batch loop.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads.max(1))
        .build()?;

    let mut pending = Vec::with_capacity(MAX_UNCOMPRESSED_BLOCK);
    let mut eof = false;
    let mut in_header = true;
    loop {
        // Read up to batch_size chunks.
        let mut chunks: Vec<Vec<u8>> = Vec::with_capacity(batch_size);
        for _ in 0..batch_size {
            match next_chunk(&mut input, text, &mut pending, &mut eof, &mut in_header)? {
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
    input: R,
    output: W,
    level: u32,
    threads: usize,
) -> Result<Vec<GziEntry>> {
    compress_indexed_mode(input, output, level, threads, false)
}

pub fn compress_indexed_auto<R: Read, W: Write>(
    input: R,
    output: W,
    level: u32,
    threads: usize,
) -> Result<Vec<GziEntry>> {
    let mut input = BufReader::new(input);
    let text = input_looks_text(&mut input)?;
    compress_indexed_mode(input, output, level, threads, text)
}

fn compress_indexed_mode<R: Read, W: Write>(
    mut input: R,
    mut output: W,
    level: u32,
    threads: usize,
    text: bool,
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

    let mut pending = Vec::with_capacity(MAX_UNCOMPRESSED_BLOCK);
    let mut eof = false;
    let mut in_header = true;
    loop {
        // Read up to batch_size chunks.
        let mut chunks: Vec<Vec<u8>> = Vec::with_capacity(batch_size);
        for _ in 0..batch_size {
            match next_chunk(&mut input, text, &mut pending, &mut eof, &mut in_header)? {
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

/// Compress one batch of chunks in parallel and write the blocks in order.
fn flush_batch(
    pool: &rayon::ThreadPool,
    batch: &mut Vec<Vec<u8>>,
    output: &mut impl Write,
    level: u32,
) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let blocks: Vec<Vec<u8>> = pool.install(|| {
        batch
            .par_iter()
            .map(|c| compress_block(c, level))
            .collect::<Result<Vec<_>>>()
    })?;
    for block in &blocks {
        output.write_all(block)?;
    }
    batch.clear();
    Ok(())
}

/// Re-bgzip: compress `input` as opaque bytes, split into BGZF blocks at the
/// uncompressed offsets listed in `index` (the first block starting at 0 is
/// implicit, as in `.gzi`).
///
/// This mirrors `bgzip -g -I index`: the input is NOT decompressed first, so
/// re-blocking an existing file (for example at a new `-l` level) preserves
/// the original uncompressed block boundaries. Segments larger than one BGZF
/// block (huge index gaps, or an empty index) are sub-split, so peak memory
/// stays bounded to roughly `batch_size * MAX_UNCOMPRESSED_BLOCK` plus a
/// small spill buffer — never the whole input.
pub fn rebgzip<R: Read, W: Write>(
    mut input: R,
    mut output: W,
    level: u32,
    threads: usize,
    index: &[GziEntry],
) -> Result<()> {
    let mut bounds: Vec<u64> = index.iter().map(|(_, u)| *u).filter(|&u| u > 0).collect();
    bounds.sort_unstable();
    bounds.dedup();

    let batch_size = threads.max(1) * 4;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads.max(1))
        .build()?;
    // Cap the spill buffer so a pathological index gap cannot grow memory
    // without bound; overflow is emitted as plain MAX-sized sub-blocks.
    const SPILL_CAP: usize = 1 << 20;

    let mut batch: Vec<Vec<u8>> = Vec::with_capacity(batch_size);
    let mut pending: Vec<u8> = Vec::new();
    let mut pos: u64 = 0; // uncompressed offset of pending[0]
    let mut bi: usize = 0; // next boundary in bounds

    // Move the first `take` bytes of pending into the batch, sub-splitting
    // anything larger than one BGZF block.
    let emit = |pending: &mut Vec<u8>, take: usize, batch: &mut Vec<Vec<u8>>| {
        let seg: Vec<u8> = pending.drain(..take).collect();
        for piece in seg.chunks(MAX_UNCOMPRESSED_BLOCK) {
            batch.push(piece.to_vec());
        }
    };

    loop {
        let chunk = match read_chunk(&mut input)? {
            None => break,
            Some(c) => c,
        };
        pending.extend_from_slice(&chunk);

        // Emit every boundary now fully buffered.
        while bi < bounds.len() && bounds[bi] <= pos + pending.len() as u64 {
            let take = (bounds[bi] - pos) as usize;
            if take > 0 {
                emit(&mut pending, take, &mut batch);
            }
            pos = bounds[bi];
            bi += 1;
        }
        // Bound memory when the next boundary is far away (or absent).
        while pending.len() > SPILL_CAP {
            emit(&mut pending, MAX_UNCOMPRESSED_BLOCK, &mut batch);
            pos += MAX_UNCOMPRESSED_BLOCK as u64;
        }
        if batch.len() >= batch_size {
            flush_batch(&pool, &mut batch, &mut output, level)?;
        }
    }

    // Flush the tail (a boundary exactly at EOF yields no empty block).
    // (`pos` is not advanced here — nothing reads offsets past EOF.)
    while !pending.is_empty() {
        let take = pending.len().min(MAX_UNCOMPRESSED_BLOCK);
        emit(&mut pending, take, &mut batch);
        if batch.len() >= batch_size {
            flush_batch(&pool, &mut batch, &mut output, level)?;
        }
    }
    flush_batch(&pool, &mut batch, &mut output, level)?;

    output.write_all(&BGZF_EOF)?;
    Ok(())
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
fn is_bgzf_prefix(prefix: &[u8]) -> bool {
    prefix.len() >= 18 && prefix[0..4] == [31, 139, 8, 4] && prefix[12..16] == [b'B', b'C', 2, 0]
}

pub fn decompress<R: Read, W: Write>(input: R, mut output: W) -> Result<()> {
    let mut input = BufReader::new(input);
    let prefix = input.fill_buf()?;
    if prefix.is_empty() {
        return Ok(());
    }
    if !is_bgzf_prefix(prefix) {
        if prefix.len() < 3 || prefix[0..3] != [31, 139, 8] {
            bail!("not a gzip or BGZF stream");
        }
        let mut decoder = MultiGzDecoder::new(input);
        io::copy(&mut decoder, &mut output).context("invalid gzip stream")?;
        return Ok(());
    }
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

pub fn test<R: Read>(input: R) -> Result<()> {
    let mut input = BufReader::new(input);
    let prefix = input.fill_buf()?;
    if prefix.is_empty() {
        return Ok(());
    }
    if !is_bgzf_prefix(prefix) {
        if prefix.len() < 3 || prefix[0..3] != [31, 139, 8] {
            bail!("not a gzip or BGZF stream");
        }
        let mut decoder = MultiGzDecoder::new(input);
        io::copy(&mut decoder, &mut io::sink()).context("invalid gzip stream")?;
        return Ok(());
    }
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
