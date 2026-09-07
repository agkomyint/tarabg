#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

#[allow(dead_code)]
#[path = "../../src/block.rs"]
mod block;
#[allow(dead_code)]
#[path = "../../src/bgzf.rs"]
mod bgzf;

fuzz_target!(|data: &[u8]| {
    let _ = bgzf::read_gzi(Cursor::new(data));
});
