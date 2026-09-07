use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, SystemTime},
};
use tempfile::{tempdir, NamedTempFile};

fn bgzip() -> Option<String> {
    ["bgzip", "bgzip.exe"]
        .into_iter()
        .find(|p| Command::new(p).arg("--version").output().is_ok())
        .map(str::to_owned)
}

/// Absolute path to the compiled tarabg binary.
fn tarabg() -> &'static str {
    env!("CARGO_BIN_EXE_tarabg")
}

// ── Existing interop test ─────────────────────────────────────────────────────

#[test]
fn interoperates_with_bgzip_when_available() {
    let Some(bgzip) = bgzip() else {
        eprintln!("skipping: bgzip is not installed");
        return;
    };
    let mut input = NamedTempFile::new().unwrap();
    input.write_all(&vec![b'A'; 100_000]).unwrap();
    let tara = tarabg();
    let compressed = Command::new(tara)
        .args(["-l", "6", "-c", input.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(compressed.status.success());
    let mut tara_gz = NamedTempFile::new().unwrap();
    tara_gz.write_all(&compressed.stdout).unwrap();
    tara_gz.flush().unwrap();
    assert!(Command::new(&bgzip)
        .args(["-t", tara_gz.path().to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    let decoded = Command::new(&bgzip)
        .args(["-d", "-c", tara_gz.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(decoded.stdout, std::fs::read(input.path()).unwrap());

    let native = Command::new(&bgzip)
        .args(["-l", "6", "-c", input.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(native.status.success());
    let mut native_gz = NamedTempFile::new().unwrap();
    native_gz.write_all(&native.stdout).unwrap();
    native_gz.flush().unwrap();
    let tara_decoded = Command::new(tara)
        .args(["-d", "-c", native_gz.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(tara_decoded.status.success());
    assert_eq!(tara_decoded.stdout, std::fs::read(input.path()).unwrap());

    let tara_index = NamedTempFile::new().unwrap();
    let indexed = Command::new(tara)
        .args([
            "-i",
            "-I",
            tara_index.path().to_str().unwrap(),
            "-c",
            input.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(indexed.status.success());
    let mut indexed_gz = NamedTempFile::new().unwrap();
    indexed_gz.write_all(&indexed.stdout).unwrap();
    indexed_gz.flush().unwrap();
    let expected = &std::fs::read(input.path()).unwrap()[65_270..65_370];
    let native_range = Command::new(&bgzip)
        .args([
            "-b",
            "65270",
            "-s",
            "100",
            "-I",
            tara_index.path().to_str().unwrap(),
            indexed_gz.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(native_range.status.success());
    assert_eq!(native_range.stdout, expected);

    let native_index = NamedTempFile::new().unwrap();
    let native_indexed = Command::new(&bgzip)
        .args([
            "-i",
            "-I",
            native_index.path().to_str().unwrap(),
            "-c",
            input.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(native_indexed.status.success());
    let mut native_indexed_gz = NamedTempFile::new().unwrap();
    native_indexed_gz.write_all(&native_indexed.stdout).unwrap();
    native_indexed_gz.flush().unwrap();
    let tara_range = Command::new(tara)
        .args([
            "-b",
            "65270",
            "-s",
            "100",
            "-I",
            native_index.path().to_str().unwrap(),
            native_indexed_gz.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(tara_range.status.success());
    assert_eq!(tara_range.stdout, expected);
}

// ── Phase 2 new compatibility tests ──────────────────────────────────────────

/// Default file-mode compress then decompress; verify content round-trips.
#[test]
fn tarabg_file_mode_compress_decompress() {
    let tara = tarabg();
    let dir = tempdir().unwrap();
    let input_path = dir.path().join("sample.txt");
    let gz_path = dir.path().join("sample.txt.gz");

    let content = b"Hello from TaraBG file-mode test!\n".repeat(100);
    fs::write(&input_path, &content).unwrap();

    // Compress: no -c, no -o  => writes sample.txt.gz, removes sample.txt
    let status = Command::new(tara)
        .arg(input_path.to_str().unwrap())
        .status()
        .unwrap();
    assert!(status.success(), "compression failed");
    assert!(gz_path.exists(), ".gz file not created");
    assert!(!input_path.exists(), "input not removed after compression");

    // Decompress: no -c, no -o  => writes sample.txt, removes sample.txt.gz
    let status = Command::new(tara)
        .arg("-d")
        .arg(gz_path.to_str().unwrap())
        .status()
        .unwrap();
    assert!(status.success(), "decompression failed");
    assert!(input_path.exists(), "decompressed file not created");
    assert!(!gz_path.exists(), ".gz not removed after decompression");

    let recovered = fs::read(&input_path).unwrap();
    assert_eq!(recovered, content, "content mismatch after roundtrip");
}

/// -k flag: input file is retained after file-mode compression.
#[test]
fn tarabg_keep_flag() {
    let tara = tarabg();
    let dir = tempdir().unwrap();
    let input_path = dir.path().join("keep_test.txt");
    let gz_path = dir.path().join("keep_test.txt.gz");

    fs::write(&input_path, b"data to keep").unwrap();

    let status = Command::new(tara)
        .arg("-k")
        .arg(input_path.to_str().unwrap())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(gz_path.exists(), ".gz file not created");
    assert!(input_path.exists(), "input removed despite -k flag");
}

/// -f flag: overwrites existing output; without -f, refuses to overwrite.
#[test]
fn tarabg_force_flag() {
    let tara = tarabg();
    let dir = tempdir().unwrap();
    let input_path = dir.path().join("force_test.txt");
    let gz_path = dir.path().join("force_test.txt.gz");

    fs::write(&input_path, b"force test data").unwrap();
    // Pre-create the output file so it already exists.
    fs::write(&gz_path, b"old content").unwrap();

    // Without -f: should fail because output exists.
    // Re-create input since it would be removed on success.
    let status_no_force = Command::new(tara)
        .arg("-k") // keep input so we can try again
        .arg(input_path.to_str().unwrap())
        .status()
        .unwrap();
    assert!(
        !status_no_force.success(),
        "expected failure when output exists without -f"
    );
    // Original gz remains untouched.
    assert_eq!(fs::read(&gz_path).unwrap(), b"old content");

    // With -f: should succeed and overwrite.
    let status_force = Command::new(tara)
        .args(["-f", "-k"])
        .arg(input_path.to_str().unwrap())
        .status()
        .unwrap();
    assert!(status_force.success(), "expected success with -f");
    // New gz should not be the old placeholder.
    let new_gz = fs::read(&gz_path).unwrap();
    assert_ne!(new_gz, b"old content", "output was not overwritten");
}

/// -o FILE: writes to the specified file, input retained.
#[test]
fn tarabg_output_flag() {
    let tara = tarabg();
    let dir = tempdir().unwrap();
    let input_path = dir.path().join("out_flag.txt");
    let custom_out: PathBuf = dir.path().join("custom_output.bgz");

    fs::write(&input_path, b"output flag test").unwrap();

    let status = Command::new(tara)
        .arg("-o")
        .arg(custom_out.to_str().unwrap())
        .arg(input_path.to_str().unwrap())
        .status()
        .unwrap();
    assert!(status.success(), "compression with -o failed");
    assert!(custom_out.exists(), "custom output file not created");
    // Input should be retained when -o is specified.
    assert!(input_path.exists(), "input removed despite -o usage");
}

/// Multiple positional files: compress two files in one invocation.
#[test]
fn tarabg_multiple_files() {
    let tara = tarabg();
    let dir = tempdir().unwrap();
    let file1 = dir.path().join("multi1.txt");
    let file2 = dir.path().join("multi2.txt");
    let gz1 = dir.path().join("multi1.txt.gz");
    let gz2 = dir.path().join("multi2.txt.gz");

    fs::write(&file1, b"first file content").unwrap();
    fs::write(&file2, b"second file content").unwrap();

    let status = Command::new(tara)
        .arg(file1.to_str().unwrap())
        .arg(file2.to_str().unwrap())
        .status()
        .unwrap();
    assert!(status.success(), "multi-file compression failed");
    assert!(gz1.exists(), "gz1 not created");
    assert!(gz2.exists(), "gz2 not created");
    assert!(!file1.exists(), "file1 not removed");
    assert!(!file2.exists(), "file2 not removed");
}

// ── Phase 4: -g / --rebgzip ───────────────────────────────────────────────
// Semantics (derived from black-box bgzip 1.19 behavior):
// - `-I index` is mandatory; without it rebgzip fails.
// - The input is treated as opaque bytes (never decompressed) and BGZF-
//   compressed with blocks split at the index's uncompressed offsets.
// - `-g` cannot be combined with `-i`/`-r`; `-d`/`-t`/`-b`/`-s` win over `-g`.

/// Uncompressed block sizes (ISIZEs, skipping the EOF marker) of a BGZF stream.
fn bgzf_isizes(data: &[u8]) -> Vec<u32> {
    let mut sizes = Vec::new();
    let mut off = 0usize;
    while off < data.len() {
        assert_eq!(&data[off..off + 4], &[31, 139, 8, 4], "not a BGZF block");
        let bsize = u16::from_le_bytes([data[off + 16], data[off + 17]]) as usize + 1;
        let isize = u32::from_le_bytes(data[off + bsize - 4..off + bsize].try_into().unwrap());
        if isize != 0 {
            sizes.push(isize);
        }
        off += bsize;
    }
    sizes
}

/// Pseudo-random incompressible bytes (deterministic LCG).
fn incompressible_bytes(n: usize) -> Vec<u8> {
    let mut x = 0x12345678u32;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            (x >> 16) as u8
        })
        .collect()
}

/// Build a multi-block fixture with tarabg alone: returns (raw bytes,
/// BGZF path, .gzi path). Incompressible input keeps the .gz file larger
/// than the first index boundary so splits are exercised.
fn rebgzip_fixture(dir: &std::path::Path) -> (Vec<u8>, PathBuf, PathBuf) {
    let payload = incompressible_bytes(200_000);
    let input = dir.join("data.bin");
    let gz = dir.join("data.bin.gz");
    let gzi = dir.join("data.bin.gz.gzi");
    fs::write(&input, &payload).unwrap();
    let status = Command::new(tarabg())
        .arg("-i")
        .arg("-I")
        .arg(gzi.to_str().unwrap())
        .arg("-o")
        .arg(gz.to_str().unwrap())
        .arg(input.to_str().unwrap())
        .status()
        .unwrap();
    assert!(status.success(), "fixture compression failed");
    assert!(gz.exists() && gzi.exists(), "fixture files missing");
    (payload, gz, gzi)
}

fn tarabg_decompress(path: &std::path::Path) -> Vec<u8> {
    let out = Command::new(tarabg())
        .args(["-d", "-c", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "decompression failed");
    out.stdout
}

/// `-g` without `-I` must fail (bgzip: "Index file name expected").
#[test]
fn rebgzip_requires_index() {
    let dir = tempdir().unwrap();
    let (_, gz, _) = rebgzip_fixture(dir.path());
    let status = Command::new(tarabg())
        .args(["-g", "-c", gz.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(!status.success(), "-g without -I should fail");
}

/// `-g` cannot be combined with `-i` or `-r`.
#[test]
fn rebgzip_rejects_index_creation() {
    let dir = tempdir().unwrap();
    let (_, gz, _) = rebgzip_fixture(dir.path());
    for extra in [&["-i", "-I", "x.gzi"][..], &["-r"][..]] {
        let mut cmd = Command::new(tarabg());
        cmd.arg("-g").args(extra).arg("-c").arg(&gz);
        // -r takes no -c value issue: pass input positionally instead.
        let status = cmd.status().unwrap();
        assert!(!status.success(), "-g with {extra:?} should fail");
    }
}

/// `-g` output decompresses to the exact input bytes.
#[test]
fn rebgzip_stdout_roundtrip() {
    let dir = tempdir().unwrap();
    let (_, gz, gzi) = rebgzip_fixture(dir.path());
    let out = Command::new(tarabg())
        .arg("-g")
        .arg("-I")
        .arg(gzi.to_str().unwrap())
        .arg("-c")
        .arg(gz.to_str().unwrap())
        .output()
        .unwrap();
    assert!(out.status.success(), "rebgzip failed");
    let recompressed = dir.path().join("re.gz");
    fs::write(&recompressed, &out.stdout).unwrap();
    assert_eq!(
        tarabg_decompress(&recompressed),
        fs::read(&gz).unwrap(),
        "rebgzip output must decode to the input bytes"
    );
}

/// File-mode `-g`: `x.gz` becomes `x.gz.gz`, input removed unless `-k`.
#[test]
fn rebgzip_file_mode() {
    let tara = tarabg();
    let dir = tempdir().unwrap();
    let (_, gz, gzi) = rebgzip_fixture(dir.path());
    let work = dir.path().join("work.gz");
    fs::copy(&gz, &work).unwrap();

    let status = Command::new(tara)
        .arg("-g")
        .arg("-I")
        .arg(gzi.to_str().unwrap())
        .arg(work.to_str().unwrap())
        .status()
        .unwrap();
    assert!(status.success(), "file-mode rebgzip failed");
    let out = dir.path().join("work.gz.gz");
    assert!(out.exists(), "reblocked output missing");
    assert!(!work.exists(), "input not removed");
    assert_eq!(tarabg_decompress(&out), fs::read(&gz).unwrap());

    // -k retains the input.
    fs::copy(&gz, &work).unwrap();
    let status = Command::new(tara)
        .args(["-g", "-k", "-f"])
        .arg("-I")
        .arg(gzi.to_str().unwrap())
        .arg(work.to_str().unwrap())
        .status()
        .unwrap();
    assert!(status.success(), "rebgzip -k failed");
    assert!(work.exists(), "input removed despite -k");
}

/// Against native bgzip: same splits at the index boundaries, same bytes.
#[test]
fn rebgzip_matches_bgzip_splits() {
    let Some(bgzip) = bgzip() else {
        eprintln!("skipping: bgzip is not installed");
        return;
    };
    let dir = tempdir().unwrap();
    let (_, gz, gzi) = rebgzip_fixture(dir.path());

    let native = Command::new(&bgzip)
        .arg("-g")
        .arg("-I")
        .arg(gzi.to_str().unwrap())
        .arg("-c")
        .arg(gz.to_str().unwrap())
        .output()
        .unwrap();
    assert!(native.status.success(), "bgzip -g failed");
    let ours = Command::new(tarabg())
        .arg("-g")
        .arg("-I")
        .arg(gzi.to_str().unwrap())
        .arg("-c")
        .arg(gz.to_str().unwrap())
        .output()
        .unwrap();
    assert!(ours.status.success(), "tarabg -g failed");

    // Both decode (via bgzip) to the exact input bytes...
    let input_bytes = fs::read(&gz).unwrap();
    for (label, blob) in [("bgzip", &native.stdout), ("tarabg", &ours.stdout)] {
        let tmp = dir.path().join(format!("{label}.gz"));
        fs::write(&tmp, blob).unwrap();
        assert!(Command::new(&bgzip)
            .args(["-t", tmp.to_str().unwrap()])
            .status()
            .unwrap()
            .success());
        let dec = Command::new(&bgzip)
            .args(["-d", "-c", tmp.to_str().unwrap()])
            .output()
            .unwrap();
        assert_eq!(dec.stdout, input_bytes, "{label} bytes mismatch");
    }
    // ...and split at the same uncompressed boundaries.
    assert_eq!(
        bgzf_isizes(&ours.stdout),
        bgzf_isizes(&native.stdout),
        "block splits differ from bgzip"
    );
}

#[test]
fn text_and_binary_block_splits_match_bgzip_when_available() {
    let Some(bgzip) = bgzip() else {
        eprintln!("skipping: bgzip is not installed");
        return;
    };
    let dir = tempdir().unwrap();
    let input = dir.path().join("records.vcf");
    let mut payload = b"##fileformat=VCFv4.2\n#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\n".to_vec();
    for position in 1..4000 {
        payload.extend_from_slice(
            format!("22\t{position}\trs{position}\tA\tG\t60\tPASS\tDP=40;AF=0.5\n").as_bytes(),
        );
    }
    fs::write(&input, &payload).unwrap();

    for extra in [&[][..], &["--binary"][..]] {
        let native = Command::new(&bgzip)
            .args(extra)
            .arg("-c")
            .arg(&input)
            .output()
            .unwrap();
        let ours = Command::new(tarabg())
            .args(extra)
            .arg("-c")
            .arg(&input)
            .output()
            .unwrap();
        assert!(native.status.success() && ours.status.success());
        assert_eq!(
            bgzf_isizes(&ours.stdout),
            bgzf_isizes(&native.stdout),
            "block splits differ for {extra:?}"
        );
        let ours_path = dir.path().join(if extra.is_empty() {
            "ours-text.gz"
        } else {
            "ours-binary.gz"
        });
        fs::write(&ours_path, &ours.stdout).unwrap();
        assert!(Command::new(&bgzip)
            .args(["-t", ours_path.to_str().unwrap()])
            .status()
            .unwrap()
            .success());
    }
}

/// An empty (0-entry) index falls back to default blocking; bytes exact.
#[test]
fn rebgzip_empty_index() {
    let dir = tempdir().unwrap();
    let (_, gz, _) = rebgzip_fixture(dir.path());
    let empty_gzi = dir.path().join("empty.gzi");
    fs::write(&empty_gzi, 0u64.to_le_bytes()).unwrap();
    let out = Command::new(tarabg())
        .arg("-g")
        .arg("-I")
        .arg(empty_gzi.to_str().unwrap())
        .arg("-c")
        .arg(gz.to_str().unwrap())
        .output()
        .unwrap();
    assert!(out.status.success(), "rebgzip with empty index failed");
    let recompressed = dir.path().join("re.gz");
    fs::write(&recompressed, &out.stdout).unwrap();
    assert_eq!(tarabg_decompress(&recompressed), fs::read(&gz).unwrap());
}

/// An index boundary exactly at EOF is a no-op, not a crash
/// (native bgzip segfaults here; we must stay graceful).
#[test]
fn rebgzip_eof_boundary_index() {
    let dir = tempdir().unwrap();
    let (_, gz, _) = rebgzip_fixture(dir.path());
    let len = fs::metadata(&gz).unwrap().len();
    let mut gzi_bytes = 1u64.to_le_bytes().to_vec();
    gzi_bytes.extend_from_slice(&9999u64.to_le_bytes());
    gzi_bytes.extend_from_slice(&len.to_le_bytes());
    let edge_gzi = dir.path().join("edge.gzi");
    fs::write(&edge_gzi, &gzi_bytes).unwrap();
    let out = Command::new(tarabg())
        .arg("-g")
        .arg("-I")
        .arg(edge_gzi.to_str().unwrap())
        .arg("-c")
        .arg(gz.to_str().unwrap())
        .output()
        .unwrap();
    assert!(out.status.success(), "rebgzip with EOF boundary failed");
    let recompressed = dir.path().join("re.gz");
    fs::write(&recompressed, &out.stdout).unwrap();
    assert_eq!(tarabg_decompress(&recompressed), fs::read(&gz).unwrap());
}

/// `-d`/`-t`/`-b` take precedence over `-g`, matching bgzip.
#[test]
fn rebgzip_defers_to_other_modes() {
    let tara = tarabg();
    let dir = tempdir().unwrap();
    let (payload, gz, gzi) = rebgzip_fixture(dir.path());

    // -g -d behaves like -d.
    let out = Command::new(tara)
        .args(["-g", "-d", "-c"])
        .arg(gz.to_str().unwrap())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, payload);

    // -g -t behaves like -t (no output, success).
    let status = Command::new(tara)
        .args(["-g", "-t"])
        .arg(gz.to_str().unwrap())
        .status()
        .unwrap();
    assert!(status.success());

    // -g -b/-s behaves like a raw range read.
    let out = Command::new(tara)
        .args(["-g", "-b", "100", "-s", "50", "-I", gzi.to_str().unwrap()])
        .arg(gz.to_str().unwrap())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, payload[100..150]);
}

// ── Parser parity (bgzip long aliases, -l -1, repeated flags) ──────────────
// Derived from black-box bgzip 1.19 probes: `--compress-level` and
// `--index-name` are accepted, `-l -1` means the default level (identical
// output to `-l 6`), and repeated flags take the last value.

/// `--index-name` behaves exactly like `-I`.
#[test]
fn parser_index_name_alias() {
    let tara = tarabg();
    let dir = tempdir().unwrap();
    let input = dir.path().join("a.txt");
    let idx = dir.path().join("a.gzi");
    fs::write(&input, b"alias test payload").unwrap();
    let status = Command::new(tara)
        .arg("-i")
        .arg("--index-name")
        .arg(idx.to_str().unwrap())
        .arg("-c")
        .arg(input.to_str().unwrap())
        .status()
        .unwrap();
    assert!(status.success(), "--index-name rejected");
    assert!(idx.exists(), "--index-name did not create the index");
}

/// `--compress-level` behaves exactly like `-l`.
#[test]
fn parser_compress_level_alias() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("b.txt");
    fs::write(&input, b"alias test payload".repeat(100)).unwrap();
    let out = Command::new(tarabg())
        .arg("--compress-level")
        .arg("1")
        .arg("-c")
        .arg(input.to_str().unwrap())
        .output()
        .unwrap();
    assert!(out.status.success(), "--compress-level rejected");
    let gz = dir.path().join("b.gz");
    fs::write(&gz, &out.stdout).unwrap();
    assert_eq!(tarabg_decompress(&gz), fs::read(&input).unwrap());
}

/// `-l -1` selects the default level: byte-identical output to `-l 6`.
#[test]
fn parser_level_minus_one_is_default() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("c.txt");
    fs::write(&input, b"default level payload".repeat(100)).unwrap();
    let run = |args: &[&str]| {
        Command::new(tarabg())
            .args(args)
            .arg(input.to_str().unwrap())
            .output()
            .unwrap()
            .stdout
    };
    let via_default_flag = run(&["-l", "-1", "-c"]);
    let via_six = run(&["-l", "6", "-c"]);
    assert_eq!(via_default_flag, via_six, "-l -1 must equal -l 6");
}

/// Repeated flags take the last value instead of erroring.
#[test]
fn parser_repeated_flag_last_wins() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("d.txt");
    fs::write(&input, b"repeat payload".repeat(100)).unwrap();
    let out = Command::new(tarabg())
        .args(["-l", "1", "-l", "6", "-c"])
        .arg(input.to_str().unwrap())
        .output()
        .unwrap();
    assert!(out.status.success(), "repeated -l rejected");
    let expected = Command::new(tarabg())
        .args(["-l", "6", "-c"])
        .arg(input.to_str().unwrap())
        .output()
        .unwrap()
        .stdout;
    assert_eq!(out.stdout, expected, "last -l value did not win");
}

#[test]
fn test_and_decompress_combination_is_an_integrity_test() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("test.txt");
    fs::write(&input, b"test mode").unwrap();
    let compressed = Command::new(tarabg())
        .args(["-c", input.to_str().unwrap()])
        .output()
        .unwrap();
    let gz = dir.path().join("test.gz");
    fs::write(&gz, compressed.stdout).unwrap();
    for flags in [["-t", "-d"], ["-d", "-t"]] {
        let out = Command::new(tarabg())
            .args(flags)
            .arg(&gz)
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(out.stdout.is_empty());
    }
}

#[test]
fn size_without_decompress_still_compresses_to_stdout() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("size.txt");
    let payload = b"size alone is a stdout modifier".repeat(100);
    fs::write(&input, &payload).unwrap();
    let out = Command::new(tarabg())
        .args(["-s", "5"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(out.status.success());
    let gz = dir.path().join("size.gz");
    fs::write(&gz, out.stdout).unwrap();
    assert_eq!(tarabg_decompress(&gz), payload);
    assert!(input.exists());
}

#[test]
fn indexed_range_from_stdin() {
    let dir = tempdir().unwrap();
    let (payload, gz, gzi) = rebgzip_fixture(dir.path());
    let mut child = Command::new(tarabg())
        .args(["-b", "100", "-s", "50", "-I"])
        .arg(&gzi)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let write_result = child
        .stdin
        .take()
        .unwrap()
        .write_all(&fs::read(gz).unwrap());
    if let Err(error) = write_result {
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, payload[100..150]);
}

#[test]
fn force_unknown_suffix_consumes_one_force_level() {
    let dir = tempdir().unwrap();
    let raw = dir.path().join("raw.txt");
    fs::write(&raw, b"unknown extension").unwrap();
    let compressed = Command::new(tarabg())
        .args(["-c", raw.to_str().unwrap()])
        .output()
        .unwrap();
    let odd = dir.path().join("payload.odd");
    fs::write(&odd, compressed.stdout).unwrap();

    let destination = dir.path().join("payload");
    fs::write(&destination, b"existing").unwrap();
    let once = Command::new(tarabg())
        .args(["-d", "-f", "-k"])
        .arg(&odd)
        .status()
        .unwrap();
    assert!(!once.success(), "one -f must not also overwrite the output");
    assert_eq!(fs::read(&destination).unwrap(), b"existing");

    let twice = Command::new(tarabg())
        .args(["-d", "-f", "-f", "-k"])
        .arg(&odd)
        .status()
        .unwrap();
    assert!(twice.success());
    assert_eq!(fs::read(destination).unwrap(), b"unknown extension");
}

#[test]
fn default_file_output_preserves_modified_time() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("dated.txt");
    fs::write(&input, b"timestamp").unwrap();
    let expected = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    fs::File::options()
        .write(true)
        .open(&input)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(expected))
        .unwrap();
    let status = Command::new(tarabg()).arg("-k").arg(&input).status().unwrap();
    assert!(status.success());
    let actual = fs::metadata(input.with_extension("txt.gz"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(actual, expected);
}
