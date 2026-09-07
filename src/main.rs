mod bgzf;
mod block;

use anyhow::{Context, Result};
use clap::Parser;
use std::{fs::File, io::{self, BufReader, BufWriter, Read}, path::PathBuf};

#[derive(Parser, Debug)]
#[command(version, about = "TaraBG: clean-room BGZF compressor/decompressor")]
struct Args {
    /// Write output to standard output.
    #[arg(short = 'c')]
    stdout: bool,
    /// Decompress BGZF input.
    #[arg(short = 'd')]
    decompress: bool,
    /// Test BGZF integrity; no output is written.
    #[arg(short = 't')]
    test: bool,
    /// Compression level (0 through 9).
    #[arg(short = 'l', default_value_t = 6, value_parser = clap::value_parser!(u32).range(0..=9))]
    level: u32,
    /// Number of compression worker threads.
    #[arg(short = '@', default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    threads: u32,
    /// Input file, or omit / use - for standard input.
    input: Option<PathBuf>,
}

fn reader(path: &Option<PathBuf>) -> Result<Box<dyn Read>> {
    match path.as_deref() { Some(p) if p.as_os_str() != "-" => Ok(Box::new(BufReader::new(File::open(p).with_context(|| format!("opening {}", p.display()))?))), _ => Ok(Box::new(io::stdin())) }
}
fn main() -> Result<()> {
    let args = Args::parse();
    if args.test { return bgzf::test(reader(&args.input)?); }
    let input = reader(&args.input)?;
    // Phase 1 deliberately only writes stdout: this makes pipelines safe and explicit.
    let stdout = io::stdout(); let output = BufWriter::new(stdout.lock());
    if args.decompress { bgzf::decompress(input, output) } else { bgzf::compress(input, output, args.level, args.threads as usize) }
}
