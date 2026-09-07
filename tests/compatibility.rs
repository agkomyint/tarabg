use std::{io::Write, process::Command};
use tempfile::NamedTempFile;

fn bgzip() -> Option<String> { ["bgzip", "bgzip.exe"].into_iter().find(|p| Command::new(p).arg("--version").output().is_ok()).map(str::to_owned) }

#[test]
fn interoperates_with_bgzip_when_available() {
    let Some(bgzip) = bgzip() else { eprintln!("skipping: bgzip is not installed"); return; };
    let mut input = NamedTempFile::new().unwrap();
    input.write_all(&vec![b'A'; 100_000]).unwrap();
    let tara = env!("CARGO_BIN_EXE_tarabg");
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
}
