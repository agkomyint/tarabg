#[allow(dead_code)]
#[path = "../src/bgzf.rs"]
mod bgzf;
#[path = "../src/block.rs"]
mod block;
use std::io::Cursor;

fn first_isize(stream: &[u8]) -> u32 {
    let bsize = u16::from_le_bytes([stream[16], stream[17]]) as usize + 1;
    u32::from_le_bytes(stream[bsize - 4..bsize].try_into().unwrap())
}

// ── Existing tests ────────────────────────────────────────────────────────────

#[test]
fn all_levels_roundtrip() {
    let payload = (0..200_000).map(|n| (n % 251) as u8).collect::<Vec<_>>();
    for level in 0..=9 {
        let mut stream = Vec::new();
        for chunk in payload.chunks(block::MAX_UNCOMPRESSED_BLOCK) {
            stream.extend(block::compress_block(chunk, level).unwrap());
        }
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
    let mut gzi = Vec::new();
    bgzf::write_gzi(&mut gzi, &entries).unwrap();
    assert_eq!(bgzf::read_gzi(Cursor::new(gzi)).unwrap(), entries);
    let mut range = Vec::new();
    bgzf::decompress_range(
        Cursor::new(compressed),
        &mut range,
        65_270,
        Some(100),
        Some(&entries),
    )
    .unwrap();
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
    let input: Vec<u8> = (0..block::MAX_UNCOMPRESSED_BLOCK)
        .map(|i| (i % 256) as u8)
        .collect();
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

#[test]
fn auto_text_blocks_end_after_newlines_and_binary_does_not() {
    let mut input = vec![b'A'; 40_000];
    input.push(b'\n');
    input.extend(std::iter::repeat_n(b'B', 40_000));
    input.push(b'\n');

    let mut text = Vec::new();
    bgzf::compress_auto(Cursor::new(&input), &mut text, 6, 2).unwrap();
    assert_eq!(first_isize(&text), 40_001);

    let mut binary = Vec::new();
    bgzf::compress(Cursor::new(&input), &mut binary, 6, 2).unwrap();
    assert_eq!(first_isize(&binary), block::MAX_UNCOMPRESSED_BLOCK as u32);

    let mut restored = Vec::new();
    bgzf::decompress(Cursor::new(text), &mut restored).unwrap();
    assert_eq!(restored, input);
}

#[test]
fn auto_binary_detection_keeps_fixed_splits() {
    let mut input = vec![0u8; 80_000];
    input[100] = b'\n';
    let mut stream = Vec::new();
    bgzf::compress_auto(Cursor::new(&input), &mut stream, 6, 1).unwrap();
    assert_eq!(first_isize(&stream), block::MAX_UNCOMPRESSED_BLOCK as u32);
}

#[test]
fn ordinary_gzip_streaming_decode_and_test() {
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;

    let input = b"ordinary gzip compatibility\n".repeat(20_000);
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&input).unwrap();
    let gzip = encoder.finish().unwrap();

    bgzf::test(Cursor::new(&gzip)).unwrap();
    let mut restored = Vec::new();
    bgzf::decompress(Cursor::new(gzip), &mut restored).unwrap();
    assert_eq!(restored, input);
}

#[test]
fn corrupted_ordinary_gzip_is_rejected() {
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(b"crc must be checked").unwrap();
    let mut gzip = encoder.finish().unwrap();
    let crc = gzip.len() - 8;
    gzip[crc] ^= 1;
    assert!(bgzf::test(Cursor::new(gzip)).is_err());
}

#[test]
fn concatenated_gzip_members_decode_in_order() {
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;

    let member = |payload: &[u8]| {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(payload).unwrap();
        encoder.finish().unwrap()
    };
    let mut gzip = member(b"first member\n");
    gzip.extend(member(b"second member\n"));

    bgzf::test(Cursor::new(&gzip)).unwrap();
    let mut restored = Vec::new();
    bgzf::decompress(Cursor::new(gzip), &mut restored).unwrap();
    assert_eq!(restored, b"first member\nsecond member\n");
}

#[test]
fn every_truncated_block_prefix_is_rejected() {
    let block = block::compress_block(b"truncate every boundary", 6).unwrap();
    for end in 1..block.len() {
        assert!(
            block::decompress_bgzf(&block[..end]).is_err(),
            "accepted truncation at byte {end}"
        );
    }
}

#[test]
fn trailing_non_bgzf_bytes_are_rejected() {
    let mut stream = block::compress_block(b"valid first block", 6).unwrap();
    stream.extend_from_slice(b"junk");
    assert!(bgzf::test(Cursor::new(&stream)).is_err());
    assert!(block::decompress_bgzf(&stream).is_err());
}

#[test]
fn eof_markers_between_blocks_are_noops() {
    let mut stream = block::compress_block(b"before", 6).unwrap();
    stream.extend_from_slice(&block::BGZF_EOF);
    stream.extend(block::compress_block(b"after", 6).unwrap());
    stream.extend_from_slice(&block::BGZF_EOF);

    bgzf::test(Cursor::new(&stream)).unwrap();
    let mut restored = Vec::new();
    bgzf::decompress(Cursor::new(stream), &mut restored).unwrap();
    assert_eq!(restored, b"beforeafter");
}

#[test]
fn malformed_gzi_shapes_and_order_are_rejected() {
    for malformed in [Vec::new(), vec![0; 7], 1u64.to_le_bytes().to_vec()] {
        assert!(bgzf::read_gzi(Cursor::new(malformed)).is_err());
    }

    let mut duplicate = 2u64.to_le_bytes().to_vec();
    for entry in [(10u64, 20u64), (10, 30)] {
        duplicate.extend_from_slice(&entry.0.to_le_bytes());
        duplicate.extend_from_slice(&entry.1.to_le_bytes());
    }
    assert!(bgzf::read_gzi(Cursor::new(duplicate)).is_err());

    let mut descending = 2u64.to_le_bytes().to_vec();
    for entry in [(10u64, 30u64), (20, 25)] {
        descending.extend_from_slice(&entry.0.to_le_bytes());
        descending.extend_from_slice(&entry.1.to_le_bytes());
    }
    assert!(bgzf::read_gzi(Cursor::new(descending)).is_err());
}

#[test]
fn indexed_ranges_cover_boundaries_and_eof() {
    let payload = (0..200_000).map(|n| (n % 251) as u8).collect::<Vec<_>>();
    let mut compressed = Vec::new();
    let index = bgzf::compress_indexed(Cursor::new(&payload), &mut compressed, 6, 4).unwrap();

    for (offset, size) in [
        (0, 1),
        (65_279, 2),
        (65_280, 65_280),
        (199_999, 1),
        (200_000, 1),
    ] {
        let mut restored = Vec::new();
        bgzf::decompress_range(
            Cursor::new(&compressed),
            &mut restored,
            offset,
            Some(size),
            Some(&index),
        )
        .unwrap();
        let start = offset.min(payload.len() as u64) as usize;
        let end = start.saturating_add(size as usize).min(payload.len());
        assert_eq!(restored, payload[start..end], "range {offset}+{size}");
    }
}

#[test]
fn parallel_compression_is_deterministic_and_ordered() {
    let payload = (0..1_000_000)
        .map(|n| ((n * 31 + n / 17) % 251) as u8)
        .collect::<Vec<_>>();
    let mut single = Vec::new();
    let mut parallel = Vec::new();
    bgzf::compress(Cursor::new(&payload), &mut single, 6, 1).unwrap();
    bgzf::compress(Cursor::new(&payload), &mut parallel, 6, 8).unwrap();
    assert_eq!(parallel, single);
}

#[test]
fn text_mode_handles_records_larger_than_a_block() {
    let mut payload = b"##fileformat=VCFv4.2\n#CHROM\tPOS\n".to_vec();
    payload.extend(std::iter::repeat_n(
        b'X',
        block::MAX_UNCOMPRESSED_BLOCK * 2 + 17,
    ));
    payload.push(b'\n');
    payload.extend_from_slice(b"22\t1\n");

    let mut compressed = Vec::new();
    bgzf::compress_auto(Cursor::new(&payload), &mut compressed, 6, 4).unwrap();
    let mut restored = Vec::new();
    bgzf::decompress(Cursor::new(compressed), &mut restored).unwrap();
    assert_eq!(restored, payload);
}
