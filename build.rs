// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Fetches the Florence-2-base-ft INT8 ONNX graphs and tokenizer into `OUT_DIR`
//! so they can be embedded into the binary with `include_bytes!`. Downloads are
//! pinned by SHA-256; a mismatch fails the build rather than embedding bad data.

use sha2::{Digest, Sha256};
use std::io::Read as _;
use std::path::{Path, PathBuf};

const BASE: &str = "https://huggingface.co/onnx-community/Florence-2-base-ft/resolve/main";

/// `(remote path, local file name, sha256)`.
const FILES: &[(&str, &str, &str)] = &[
    (
        "onnx/vision_encoder_int8.onnx",
        "vision_encoder_int8.onnx",
        "d7876c1ab0f7ec11998942ca189e99a775c5a4a912b813c7745d0f6fa9343487",
    ),
    (
        "onnx/embed_tokens_int8.onnx",
        "embed_tokens_int8.onnx",
        "6b2258db1c8ee9b160576ccde3cd3814d83a2edaed0dd1c6ca9ff3c38fa62214",
    ),
    (
        "onnx/encoder_model_int8.onnx",
        "encoder_model_int8.onnx",
        "f4ad7a68f1fb875d3bcf735ea14a7021b7ba7e83baf7cf10289881b4ed6d9b85",
    ),
    (
        "onnx/decoder_model_int8.onnx",
        "decoder_model_int8.onnx",
        "c529b26bafce2ee76f886f3a0e374bb646b07a6d8b7640fd8a50d7a48843dd67",
    ),
    (
        "tokenizer.json",
        "tokenizer.json",
        "d69dcdb2323e124ac4f800cb9863ddccea0d7bb11e16125e8df3bd60f2f8aeac",
    ),
];

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    for (remote, name, sha) in FILES {
        let path = out_dir.join(name);
        if path.exists() && sha256_file(&path) == *sha {
            continue;
        }
        let url = format!("{BASE}/{remote}");
        let bytes = download(&url);
        let got = hex::encode(Sha256::digest(&bytes));
        assert_eq!(&got, sha, "checksum mismatch for {name} from {url}");
        std::fs::write(&path, &bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    }
    println!("cargo:rerun-if-changed=build.rs");
}

fn download(url: &str) -> Vec<u8> {
    let mut response = ureq::get(url)
        .call()
        .unwrap_or_else(|e| panic!("download {url}: {e}"));
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .read_to_end(&mut bytes)
        .unwrap_or_else(|e| panic!("read {url}: {e}"));
    bytes
}

fn sha256_file(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => hex::encode(Sha256::digest(&bytes)),
        Err(_) => String::new(),
    }
}
