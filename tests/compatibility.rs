use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::Command,
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
    let Some(bgzip) = bgzip() else { eprintln!("skipping: bgzip is not installed"); return; };
    let mut input = NamedTempFile::new().unwrap();
    input.write_all(&vec![b'A'; 100_000]).unwrap();
    let tara = tarabg();
    let compressed = Command::new(tara).args(["-l", "6", "-c", input.path().to_str().unwrap()]).output().unwrap();
    assert!(compressed.status.success());
    let mut tara_gz = NamedTempFile::new().unwrap();
    tara_gz.write_all(&compressed.stdout).unwrap(); tara_gz.flush().unwrap();
    assert!(Command::new(&bgzip).args(["-t", tara_gz.path().to_str().unwrap()]).status().unwrap().success());
    let decoded = Command::new(&bgzip).args(["-d", "-c", tara_gz.path().to_str().unwrap()]).output().unwrap();
    assert_eq!(decoded.stdout, std::fs::read(input.path()).unwrap());

    let native = Command::new(&bgzip).args(["-l", "6", "-c", input.path().to_str().unwrap()]).output().unwrap();
    assert!(native.status.success());
    let mut native_gz = NamedTempFile::new().unwrap();
    native_gz.write_all(&native.stdout).unwrap(); native_gz.flush().unwrap();
    let tara_decoded = Command::new(tara).args(["-d", "-c", native_gz.path().to_str().unwrap()]).output().unwrap();
    assert!(tara_decoded.status.success());
    assert_eq!(tara_decoded.stdout, std::fs::read(input.path()).unwrap());

    let tara_index = NamedTempFile::new().unwrap();
    let indexed = Command::new(tara).args(["-i", "-I", tara_index.path().to_str().unwrap(), "-c", input.path().to_str().unwrap()]).output().unwrap();
    assert!(indexed.status.success());
    let mut indexed_gz = NamedTempFile::new().unwrap();
    indexed_gz.write_all(&indexed.stdout).unwrap(); indexed_gz.flush().unwrap();
    let expected = &std::fs::read(input.path()).unwrap()[65_270..65_370];
    let native_range = Command::new(&bgzip).args(["-b", "65270", "-s", "100", "-I", tara_index.path().to_str().unwrap(), indexed_gz.path().to_str().unwrap()]).output().unwrap();
    assert!(native_range.status.success());
    assert_eq!(native_range.stdout, expected);

    let native_index = NamedTempFile::new().unwrap();
    let native_indexed = Command::new(&bgzip).args(["-i", "-I", native_index.path().to_str().unwrap(), "-c", input.path().to_str().unwrap()]).output().unwrap();
    assert!(native_indexed.status.success());
    let mut native_indexed_gz = NamedTempFile::new().unwrap();
    native_indexed_gz.write_all(&native_indexed.stdout).unwrap(); native_indexed_gz.flush().unwrap();
    let tara_range = Command::new(tara).args(["-b", "65270", "-s", "100", "-I", native_index.path().to_str().unwrap(), native_indexed_gz.path().to_str().unwrap()]).output().unwrap();
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
        .arg("-k")                           // keep input so we can try again
        .arg(input_path.to_str().unwrap())
        .status()
        .unwrap();
    assert!(!status_no_force.success(), "expected failure when output exists without -f");
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
