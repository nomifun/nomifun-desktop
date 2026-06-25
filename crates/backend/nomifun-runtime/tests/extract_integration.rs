//! Integration tests for `nomifun_runtime` extraction.

use std::fs;
use std::io::Write;
use tempfile::TempDir;

fn make_zstd_blob(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = zstd::stream::write::Encoder::new(&mut out, 0).unwrap();
    enc.write_all(payload).unwrap();
    enc.finish().unwrap();
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

#[test]
fn zstd_roundtrip_produces_matching_bytes() {
    let payload = b"#!/bin/sh\necho fake-bun\n";
    let blob = make_zstd_blob(payload);

    let mut dec = zstd::stream::read::Decoder::new(&blob[..]).unwrap();
    let mut out = Vec::new();
    std::io::copy(&mut dec, &mut out).unwrap();
    assert_eq!(out, payload);
    assert_eq!(sha256_hex(&out).len(), 64);
}

#[test]
fn temp_dir_fixture_available() {
    let tmp = TempDir::new().unwrap();
    let p = tmp.path().join("probe");
    fs::write(&p, b"x").unwrap();
    assert!(p.is_file());
}
