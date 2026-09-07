#[path = "../src/block.rs"] mod block;
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
