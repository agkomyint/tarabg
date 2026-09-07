#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::{self, Cursor};

#[allow(dead_code)]
#[path = "../../src/block.rs"]
mod block;
#[allow(dead_code)]
#[path = "../../src/bgzf.rs"]
mod bgzf;

fuzz_target!(|data: &[u8]| {
    let _ = bgzf::test(Cursor::new(data));
    let _ = bgzf::decompress(Cursor::new(data), io::sink());
    let _ = bgzf::reindex(Cursor::new(data));
});
