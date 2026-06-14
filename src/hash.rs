// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

const HASHLINE_MODULUS: u32 = 0x1000;

/// Compute a stable locality-preserving hash for a source line.
///
/// The line text is whitespace-normalized before hashing so that minor
/// whitespace changes do not invalidate the hash.  The result is a 3-digit
/// hex string in `[000, fff]`.
pub(crate) fn hash_line(_line: usize, text: &str) -> String {
    let text = text.strip_suffix('\r').unwrap_or(text);
    let mut hasher = xxhash_rust::xxh32::Xxh32::new(0);
    for token in text.split_whitespace() {
        hasher.update(token.as_bytes());
    }
    format!("{:03x}", hasher.digest() % HASHLINE_MODULUS)
}

/// Compute a BLAKE3 content hash for the full source text.
pub(crate) fn hash_text(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}
