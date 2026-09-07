use crate::block::{compress_block, decompress_bgzf, BGZF_EOF, MAX_UNCOMPRESSED_BLOCK};
use anyhow::Result;
use rayon::prelude::*;
use std::io::{Read, Write};

pub fn compress<R: Read, W: Write>(mut input: R, mut output: W, level: u32, threads: usize) -> Result<()> {
    let mut bytes = Vec::new(); input.read_to_end(&mut bytes)?;
    let chunks: Vec<&[u8]> = bytes.chunks(MAX_UNCOMPRESSED_BLOCK).collect();
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads.max(1)).build()?;
    let blocks = pool.install(|| chunks.par_iter().map(|chunk| compress_block(chunk, level)).collect::<Result<Vec<_>>>());
    for block in blocks? { output.write_all(&block)?; }
    output.write_all(&BGZF_EOF)?;
    Ok(())
}

pub fn decompress<R: Read, W: Write>(mut input: R, mut output: W) -> Result<()> {
    let mut bytes = Vec::new(); input.read_to_end(&mut bytes)?;
    output.write_all(&decompress_bgzf(&bytes)?)?;
    Ok(())
}

pub fn test<R: Read>(mut input: R) -> Result<()> {
    let mut bytes = Vec::new(); input.read_to_end(&mut bytes)?;
    decompress_bgzf(&bytes).map(|_| ())
}
