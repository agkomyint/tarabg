mod bgzf;
mod block;

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::{
    fs::File,
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    process,
};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "TaraBG: clean-room BGZF compressor/decompressor",
    args_override_self = true
)]
struct Args {
    /// Write output to standard output; keep input file.
    #[arg(short = 'c', long = "stdout")]
    stdout: bool,

    /// Decompress BGZF input.
    #[arg(short = 'd', long = "decompress")]
    decompress: bool,

    /// Test BGZF integrity; no output is written.
    #[arg(short = 't', long = "test")]
    test: bool,

    /// Create a .gzi index while compressing.
    #[arg(short = 'i', long = "index")]
    index: bool,

    /// Read or write this .gzi index file.
    #[arg(short = 'I', long = "index-name")]
    index_name: Option<PathBuf>,

    /// Rebuild a .gzi index for an existing BGZF file.
    #[arg(short = 'r', long = "reindex")]
    reindex: bool,

    /// Start decompression at this uncompressed byte offset (requires file input).
    #[arg(short = 'b', long = "offset")]
    offset: Option<u64>,

    /// Write at most this many uncompressed bytes (use with -b).
    #[arg(short = 's', long = "size")]
    size: Option<u64>,

    /// Compression level 0 through 9, or -1 for the default (6).
    #[arg(short = 'l', long = "level", visible_alias = "compress-level", default_value_t = 6, value_parser = clap::value_parser!(i32).range(-1..=9), allow_hyphen_values = true)]
    level: i32,

    /// Number of compression worker threads (must be >= 1).
    #[arg(short = '@', long = "threads", default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    threads: u32,

    /// Keep (don't remove) input file during default file-mode operations.
    #[arg(short = 'k', long = "keep")]
    keep: bool,

    /// Force unknown suffix handling and/or overwrite (repeat for both).
    #[arg(short = 'f', long = "force", action = clap::ArgAction::Count)]
    force: u8,

    /// Write output to FILE instead of the default path or stdout.
    #[arg(short = 'o')]
    output: Option<PathBuf>,

    /// Do not align text blocks at newline boundaries.
    #[arg(long = "binary")]
    binary: bool,

    /// Re-bgzip: compress the input as opaque bytes, splitting BGZF blocks
    /// at the uncompressed offsets of a `.gzi` index (requires `-I`).
    #[arg(short = 'g', long = "rebgzip")]
    rebgzip: bool,

    /// Input file(s), or omit / use - for standard input.
    #[arg(value_name = "FILE")]
    inputs: Vec<PathBuf>,
}

// ──────────────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────────────

fn open_input(path: &Path) -> Result<Box<dyn Read>> {
    if path.as_os_str() == "-" {
        Ok(Box::new(io::stdin()))
    } else {
        Ok(Box::new(BufReader::with_capacity(
            1 << 20,
            File::open(path).with_context(|| format!("opening {}", path.display()))?,
        )))
    }
}

fn is_stdin(path: &std::path::Path) -> bool {
    path.as_os_str() == "-"
}

/// Load the `.gzi` index required by `--rebgzip`.
///
/// Unlike random-access reads, re-bgzip never discovers a default index:
/// `-I` is mandatory, matching bgzip ("Index file name expected").
fn load_rebgzip_index(args: &Args) -> Result<Vec<bgzf::GziEntry>> {
    let index_path = args
        .index_name
        .clone()
        .context("an index name (-I) is required with --rebgzip")?;
    bgzf::read_gzi(BufReader::with_capacity(
        1 << 20,
        File::open(&index_path).context("opening .gzi index")?,
    ))
}
/// Resolve the requested compression level: `-1` selects the default (6),
/// matching bgzip. Clap guarantees the range -1..=9.
fn effective_level(args: &Args) -> u32 {
    if args.level < 0 {
        6
    } else {
        args.level as u32
    }
}

/// Determine the default decompressed output path by stripping known suffixes.
/// Returns None if the path does not have a recognised compressed suffix.
fn stripped_suffix(path: &std::path::Path) -> Option<PathBuf> {
    let name = path.to_str()?;
    for suffix in &[".bgzf", ".bgz", ".gz"] {
        if let Some(stem) = name.strip_suffix(suffix) {
            return Some(PathBuf::from(stem));
        }
    }
    None
}

fn forced_decompression_path(path: &Path, force: &mut u8) -> Result<PathBuf> {
    if let Some(path) = stripped_suffix(path) {
        return Ok(path);
    }
    let stem = path.with_extension("");
    if stem == path || path.extension().is_none() {
        bail!("can't find an extension in {} -- please rename", path.display());
    }
    if *force == 0 {
        bail!(
            "unknown extension .{} -- use -f to decompress to {}",
            path.extension().unwrap_or_default().to_string_lossy(),
            stem.display()
        );
    }
    *force -= 1;
    Ok(stem)
}

fn preserve_file_times(source: &Path, destination: &Path) -> Result<()> {
    let metadata = std::fs::metadata(source)?;
    let times = std::fs::FileTimes::new()
        .set_accessed(metadata.accessed()?)
        .set_modified(metadata.modified()?);
    File::options()
        .write(true)
        .open(destination)?
        .set_times(times)?;
    Ok(())
}

/// Write `data` to `final_path`, using a `.tmp` intermediary. Respects `force`.
fn write_file_atomic(data: &[u8], final_path: &Path, force: bool) -> Result<()> {
    check_overwrite(final_path, force)?;
    let tmp_path = tmp_path_for(final_path);
    {
        let mut f = BufWriter::with_capacity(
            1 << 20,
            File::create(&tmp_path).with_context(|| format!("creating {}", tmp_path.display()))?,
        );
        f.write_all(data)?;
        f.flush()?;
    }
    std::fs::rename(&tmp_path, final_path)
        .with_context(|| format!("renaming tmp to {}", final_path.display()))?;
    Ok(())
}

fn check_overwrite(final_path: &Path, force: bool) -> Result<()> {
    if final_path.exists() && !force {
        bail!(
            "output file already exists: {}  (use -f to overwrite)",
            final_path.display()
        );
    }
    Ok(())
}

fn tmp_path_for(final_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.tmp", final_path.display()))
}

/// Stream `produce` directly into a tmp file, then atomically rename.
/// Avoids buffering the entire (possibly multi-GB) output in RAM.
fn stream_file_atomic(
    final_path: &Path,
    force: bool,
    produce: impl FnOnce(&mut BufWriter<File>) -> Result<()>,
) -> Result<()> {
    check_overwrite(final_path, force)?;
    let tmp_path = tmp_path_for(final_path);
    {
        let file =
            File::create(&tmp_path).with_context(|| format!("creating {}", tmp_path.display()))?;
        let mut w = BufWriter::with_capacity(1 << 20, file);
        produce(&mut w)?;
        w.flush()?;
    }
    std::fs::rename(&tmp_path, final_path)
        .with_context(|| format!("renaming tmp to {}", final_path.display()))?;
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// Per-file processing
// ──────────────────────────────────────────────────────────────────────────────

fn process_one(path: &Path, args: &Args) -> Result<()> {
    let use_stdin = is_stdin(path);

    // ── -b / -s random-access range ──────────────────────────────────────────
    if args.offset.is_some() || (args.size.is_some() && args.decompress) {
        let index_path = args.index_name.clone().or_else(|| {
            if use_stdin {
                return None;
            }
            let p = PathBuf::from(format!("{}.gzi", path.display()));
            if p.exists() {
                Some(p)
            } else {
                None
            }
        });
        let index = index_path
            .map(|p| {
                bgzf::read_gzi(BufReader::with_capacity(
                    1 << 20,
                    File::open(&p).context("opening .gzi index")?,
                ))
            })
            .transpose()?;
        if args.offset.unwrap_or(0) > 0 && index.is_none() {
            bail!("-b with a non-zero offset requires a .gzi index (use -I)");
        }
        let input = open_input(path)?;
        let stdout = io::stdout();
        let output = BufWriter::with_capacity(1 << 20, stdout.lock());
        return bgzf::decompress_range(
            input,
            output,
            args.offset.unwrap_or(0),
            args.size,
            index.as_deref(),
        );
    }

    // ── -t test ──────────────────────────────────────────────────────────────
    if args.test {
        return bgzf::test(open_input(path)?);
    }

    // ── -r reindex ───────────────────────────────────────────────────────────
    if args.reindex {
        let entries = bgzf::reindex(open_input(path)?)?;
        let index_path = if let Some(ref p) = args.index_name {
            p.clone()
        } else if !use_stdin {
            // Default: <input>.gzi
            PathBuf::from(format!("{}.gzi", path.display()))
        } else {
            bail!("an index name (-I) is required when input is stdin");
        };
        return bgzf::write_gzi(
            BufWriter::new(File::create(&index_path).context("creating .gzi index")?),
            &entries,
        );
    }

    // ── Determine output mode: stdout vs file ─────────────────────────────────
    let to_stdout = args.stdout
        || args.size.is_some()
        || use_stdin
        || args
            .output
            .as_deref()
            .map(|p| p.as_os_str() == "-")
            .unwrap_or(false);

    if to_stdout {
        // ── Stdout pipeline ───────────────────────────────────────────────────
        let input = open_input(path)?;
        let stdout = io::stdout();
        let mut output = BufWriter::with_capacity(1 << 20, stdout.lock());
        let level = effective_level(args);

        if args.index {
            let entries = if args.binary {
                bgzf::compress_indexed(input, &mut output, level, args.threads as usize)?
            } else {
                bgzf::compress_indexed_auto(input, &mut output, level, args.threads as usize)?
            };
            let index_path = if let Some(ref p) = args.index_name {
                p.clone()
            } else if !use_stdin {
                PathBuf::from(format!("{}.gzi", path.display()))
            } else {
                bail!("an index name (-I) is required when writing index in stdout mode with stdin input");
            };
            bgzf::write_gzi(
                BufWriter::new(File::create(&index_path).context("creating .gzi index")?),
                &entries,
            )?;
        } else if args.decompress {
            bgzf::decompress(input, output)?;
        } else if args.rebgzip {
            // -d/-t/-b already handled above take precedence over -g,
            // matching bgzip; here -g re-blocks the raw input bytes.
            let index = load_rebgzip_index(args)?;
            bgzf::rebgzip(input, &mut output, level, args.threads as usize, &index)?;
        } else {
            if args.binary {
                bgzf::compress(input, output, level, args.threads as usize)?;
            } else {
                bgzf::compress_auto(input, output, level, args.threads as usize)?;
            }
        }
        return Ok(());
    }

    // ── File-mode ─────────────────────────────────────────────────────────────
    // Explicit -o overrides default naming.
    let mut remaining_force = args.force;
    let final_output: PathBuf = if let Some(ref out) = args.output {
        out.clone()
    } else if args.decompress {
        forced_decompression_path(path, &mut remaining_force)?
    } else {
        // Default compression: append .gz
        PathBuf::from(format!("{}.gz", path.display()))
    };

    // ── File-mode: stream directly to a tmp file, then rename atomically. ────
    // Peak RAM stays bounded (~batch × block + 1 MiB buffers) instead of
    // buffering the entire output in memory.
    let level = effective_level(args);
    let threads = args.threads as usize;
    let is_index = args.index;
    let is_decompress = args.decompress;
    let is_rebgzip = args.rebgzip;

    if is_index {
        let mut input = open_input(path)?;
        let mut entries_out: Vec<bgzf::GziEntry> = Vec::new();
        stream_file_atomic(&final_output, remaining_force > 0, |w| {
            entries_out = if args.binary {
                bgzf::compress_indexed(&mut input, &mut *w, level, threads)?
            } else {
                bgzf::compress_indexed_auto(&mut input, &mut *w, level, threads)?
            };
            Ok(())
        })?;
        // Determine index path.
        let index_path = if let Some(ref p) = args.index_name {
            p.clone()
        } else {
            PathBuf::from(format!("{}.gzi", final_output.display()))
        };
        let mut idx_bytes: Vec<u8> = Vec::new();
        bgzf::write_gzi(&mut idx_bytes, &entries_out)?;
        write_file_atomic(&idx_bytes, &index_path, remaining_force > 0)?;
    } else if is_decompress {
        let mut input = open_input(path)?;
        stream_file_atomic(&final_output, remaining_force > 0, |w| {
            bgzf::decompress(&mut input, &mut *w)
        })?;
    } else if is_rebgzip {
        let index = load_rebgzip_index(args)?;
        let mut input = open_input(path)?;
        stream_file_atomic(&final_output, remaining_force > 0, |w| {
            bgzf::rebgzip(&mut input, &mut *w, level, threads, &index)
        })?;
    } else {
        let mut input = open_input(path)?;
        stream_file_atomic(&final_output, remaining_force > 0, |w| {
            if args.binary {
                bgzf::compress(&mut input, &mut *w, level, threads)
            } else {
                bgzf::compress_auto(&mut input, &mut *w, level, threads)
            }
        })?;
    }

    // Remove input only on success and when not keeping it.
    if args.output.is_none() && !use_stdin {
        preserve_file_times(path, &final_output)?;
    }
    if !args.keep && args.output.is_none() {
        std::fs::remove_file(path)
            .with_context(|| format!("removing input file {}", path.display()))?;
    }

    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// main
// ──────────────────────────────────────────────────────────────────────────────

fn main() {
    let args = Args::parse();

    // -g cannot be combined with producing an index, matching bgzip.
    if args.rebgzip && (args.index || args.reindex) {
        eprintln!("tarabg: can't produce an index and rebgzip simultaneously");
        process::exit(1);
    }

    if (args.stdout || args.offset.is_some() || args.size.is_some())
        && args
            .output
            .as_deref()
            .is_some_and(|path| path.as_os_str() != "-")
    {
        eprintln!("tarabg: cannot write to an explicit file and stdout at the same time");
        process::exit(1);
    }

    if (args.index || args.reindex)
        && args.output.is_none()
        && args.index_name.is_some()
        && args.inputs.len() > 1
    {
        eprintln!("tarabg: cannot specify one index filename with multiple input files");
        process::exit(1);
    }

    // Normalise inputs: if none provided, use stdin placeholder.
    let inputs: Vec<PathBuf> = if args.inputs.is_empty() {
        vec![PathBuf::from("-")]
    } else {
        args.inputs.clone()
    };

    let mut any_error = false;
    for path in &inputs {
        if let Err(e) = process_one(path, &args) {
            eprintln!("tarabg: {}: {e:#}", path.display());
            any_error = true;
        }
    }
    if any_error {
        process::exit(1);
    }
}
