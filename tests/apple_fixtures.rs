//! Verification against fixtures extracted from a real NoteStore.
//!
//! The fixtures contain personal notes, so they are NOT in the repo. Point
//! ANDREIOS_FIXTURES at a directory with `tables/<id>.pb` + `tables/<id>.md`
//! (expected output) and `notes/<id>.json`; without it these tests are skipped.

use std::path::PathBuf;

use andrei_os::apple::notes::{table, transcript};

fn fixtures() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var("ANDREIOS_FIXTURES").ok()?);
    p.is_dir().then_some(p)
}

#[test]
fn tables_match_reference_decoder() {
    let Some(dir) = fixtures() else {
        eprintln!("ANDREIOS_FIXTURES not set: skipping");
        return;
    };
    let mut n = 0;
    for e in std::fs::read_dir(dir.join("tables")).unwrap() {
        let path = e.unwrap().path();
        if path.extension().and_then(|x| x.to_str()) != Some("pb") {
            continue;
        }
        let blob = std::fs::read(&path).unwrap();
        let expected = std::fs::read_to_string(path.with_extension("md")).unwrap();
        let t = table::decode(&blob).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(table::to_markdown(&t), expected, "{}", path.display());
        n += 1;
    }
    assert!(n > 0, "no table fixtures found");
    eprintln!("{n} tables verified");
}

#[test]
fn transcripts_decode() {
    let Some(dir) = fixtures() else { return };
    let tdir = dir.join("audio_transcripts");
    if !tdir.is_dir() {
        return;
    }
    let mut n = 0;
    for e in std::fs::read_dir(tdir).unwrap() {
        let path = e.unwrap().path();
        if path.extension().and_then(|x| x.to_str()) != Some("pb") {
            continue;
        }
        let blob = std::fs::read(&path).unwrap();
        let t = transcript::decode(&blob).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let reference = std::fs::read_to_string(path.with_extension("txt")).unwrap_or_default();
        // Same words in the same order as the reference (we only differ in
        // whitespace around punctuation).
        let norm = |s: &str| {
            s.chars()
                .filter(|c| c.is_alphanumeric() || c.is_whitespace())
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        };
        assert_eq!(norm(&t.text), norm(&reference), "{}", path.display());
        n += 1;
    }
    eprintln!("{n} transcripts verified");
}
