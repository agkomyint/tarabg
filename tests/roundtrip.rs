#[path = "../src/block.rs"] mod block;
#[allow(dead_code)]
#[path = "../src/bgzf.rs"] mod bgzf;
use std::io::Cursor;

// ── Existing tests ────────────────────────────────────────────────────────────

#[test]
fn all_levels_roundtrip() {
    let payload = (0..200_000).map(|n| (n % 251) as u8).collect::<Vec<_>>();
    for level in 0..=9 {
        let mut stream = Vec::new();
        for chunk in payload.chunks(block::MAX_UNCOMPRESSED_BLOCK) { stream.extend(block::compress_block(chunk, level).unwrap()); }
        stream.extend(block::BGZF_EOF);
        assert_eq!(block::decompress_bgzf(&stream).unwrap(), payload);
    }
}

#[test]
fn rejects_corrupt_crc() {
    let mut stream = block::compress_block(b"integrity matters", 6).unwrap();
    let crc_offset = stream.len() - 8;
    stream[crc_offset] ^= 1;
    assert!(block::decompress_bgzf(&stream).is_err());
}

#[test]
fn gzi_roundtrip_and_random_access() {
    let payload = (0..200_000).map(|n| (n % 251) as u8).collect::<Vec<_>>();
    let mut compressed = Vec::new();
    let entries = bgzf::compress_indexed(Cursor::new(&payload), &mut compressed, 6, 2).unwrap();
    assert_eq!(entries.len(), 3); // The first (0, 0) block is implicit in .gzi.
    assert_eq!(bgzf::reindex(Cursor::new(&compressed)).unwrap(), entries);
    let mut gzi = Vec::new(); bgzf::write_gzi(&mut gzi, &entries).unwrap();
    assert_eq!(bgzf::read_gzi(Cursor::new(gzi)).unwrap(), entries);
    let mut range = Vec::new();
    bgzf::decompress_range(Cursor::new(compressed), &mut range, 65_270, Some(100), Some(&entries)).unwrap();
    assert_eq!(range, payload[65_270..65_370]);
}

// ── Phase 1 additional fixtures ───────────────────────────────────────────────

/// Compress empty bytes, verify decompress returns empty.
#[test]
fn empty_input_roundtrip() {
    let mut compressed = Vec::new();
    bgzf::compress(Cursor::new(b""), &mut compressed, 6, 1).unwrap();
    let mut out = Vec::new();
    bgzf::decompress(Cursor::new(&compressed), &mut out).unwrap();
    assert!(out.is_empty());
}

/// Compress a single byte, verify roundtrip.
#[test]
fn single_byte_roundtrip() {
    let input = b"X";
    let mut compressed = Vec::new();
    bgzf::compress(Cursor::new(&input[..]), &mut compressed, 6, 1).unwrap();
    let mut out = Vec::new();
    bgzf::decompress(Cursor::new(&compressed), &mut out).unwrap();
    assert_eq!(out, input);
}

/// Compress exactly MAX_UNCOMPRESSED_BLOCK bytes (block boundary), verify roundtrip.
#[test]
fn exact_block_boundary() {
    let input: Vec<u8> = (0..block::MAX_UNCOMPRESSED_BLOCK).map(|i| (i % 256) as u8).collect();
    let mut compressed = Vec::new();
    bgzf::compress(Cursor::new(&input), &mut compressed, 6, 1).unwrap();
    let mut out = Vec::new();
    bgzf::decompress(Cursor::new(&compressed), &mut out).unwrap();
    assert_eq!(out, input);
}

/// Compress pseudo-random (incompressible) bytes, verify roundtrip.
#[test]
fn incompressible_input() {
    let input: Vec<u8> = (0u32..1000).map(|i| ((i * 7 + 13) & 0xFF) as u8).collect();
    let mut compressed = Vec::new();
    bgzf::compress(Cursor::new(&input), &mut compressed, 6, 1).unwrap();
    let mut out = Vec::new();
    bgzf::decompress(Cursor::new(&compressed), &mut out).unwrap();
    assert_eq!(out, input);
}

/// Compress 65000 zero bytes (highly compressible), verify roundtrip.
#[test]
fn highly_compressible() {
    let input = vec![0u8; 65_000];
    let mut compressed = Vec::new();
    bgzf::compress(Cursor::new(&input), &mut compressed, 6, 1).unwrap();
    let mut out = Vec::new();
    bgzf::decompress(Cursor::new(&compressed), &mut out).unwrap();
    assert_eq!(out, input);
}

/// Compress 500_000 bytes (multi-block), verify roundtrip.
#[test]
fn multi_block_long() {
    let input: Vec<u8> = (0u32..500_000).map(|i| (i % 251) as u8).collect();
    let mut compressed = Vec::new();
    bgzf::compress(Cursor::new(&input), &mut compressed, 6, 2).unwrap();
    let mut out = Vec::new();
    bgzf::decompress(Cursor::new(&compressed), &mut out).unwrap();
    assert_eq!(out, input);
}

/// Give a truncated gzip header (4 bytes), expect error.
#[test]
fn rejects_truncated_header() {
    let bad = [31u8, 139, 8, 4];
    assert!(block::decompress_bgzf(&bad).is_err());
}

/// Write a .gzi with 0 entries (just 8 zero bytes), read it back, expect empty Vec.
#[test]
fn empty_gzi() {
    let mut gzi_bytes = Vec::new();
    bgzf::write_gzi(&mut gzi_bytes, &[]).unwrap();
    assert_eq!(gzi_bytes.len(), 8);
    let entries = bgzf::read_gzi(Cursor::new(&gzi_bytes)).unwrap();
    assert!(entries.is_empty());
}

// ── Phase 6c: Streaming tests ─────────────────────────────────────────────────

/// Compress 2_000_000 bytes with threads=2 (streaming batch pipeline),
/// then decompress and verify exact round-trip.
#[test]
fn streaming_compress_large() {
    let input: Vec<u8> = (0u32..2_000_000).map(|i| (i % 251) as u8).collect();
    let mut compressed = Vec::new();
    bgzf::compress(Cursor::new(&input), &mut compressed, 6, 2).unwrap();
    let mut out = Vec::new();
    bgzf::decompress(Cursor::new(&compressed), &mut out).unwrap();
    assert_eq!(out, input);
}

/// Compress 2_000_000 bytes, then decompress block-by-block via the streaming
/// decompress path, and verify exact round-trip.
#[test]
fn streaming_decompress_large() {
    let input: Vec<u8> = (0u32..2_000_000).map(|i| (i % 199) as u8).collect();
    let mut compressed = Vec::new();
    bgzf::compress(Cursor::new(&input), &mut compressed, 6, 1).unwrap();
    // Use the streaming decompress (block-by-block).
    let mut out = Vec::new();
    bgzf::decompress(Cursor::new(&compressed), &mut out).unwrap();
    assert_eq!(out, input);
}
