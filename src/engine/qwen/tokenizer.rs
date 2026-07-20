// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Fixed GPT-2 byte-level BPE tokenizer used by Qwen2 and Qwen3 models.

use std::collections::HashMap;
use std::str;

use anyhow::{Context as _, Result, anyhow, bail, ensure};

use super::gguf::Gguf;

const TOKEN_TYPE_NORMAL: u32 = 1;
const TOKEN_TYPE_UNKNOWN: u32 = 2;
const TOKEN_TYPE_CONTROL: u32 = 3;
const TOKEN_TYPE_USER_DEFINED: u32 = 4;
const TOKEN_TYPE_UNUSED: u32 = 5;
const TOKEN_TYPE_BYTE: u32 = 6;

#[derive(Clone, Copy)]
struct Merge {
    rank: usize,
    token: u32,
}

/// Qwen's tokenizer vocabulary and fixed byte-level BPE merge table.
pub(crate) struct Tokenizer {
    token_pieces: Vec<Vec<u8>>,
    token_types: Vec<u32>,
    token_ids: HashMap<String, u32>,
    byte_tokens: [Option<u32>; 256],
    merges: HashMap<(u32, u32), Merge>,
    special_tokens: Vec<(String, u32)>,
    eos_token: u32,
}

impl Tokenizer {
    /// Construct a tokenizer from the canonical GGUF tokenizer arrays.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn from_gguf(gguf: &Gguf) -> Result<Self> {
        let token_texts = gguf.string_array("tokenizer.ggml.tokens")?.to_vec();
        let merge_texts = gguf.string_array("tokenizer.ggml.merges")?;
        let token_types: Vec<u32> = gguf
            .i32_array("tokenizer.ggml.token_type")?
            .iter()
            .copied()
            .enumerate()
            .map(|(index, token_type)| {
                u32::try_from(token_type).with_context(|| {
                    format!("token {index} has negative GGML token type {token_type}")
                })
            })
            .collect::<Result<_>>()?;
        ensure!(
            !token_texts.is_empty(),
            "GGUF tokenizer vocabulary is empty"
        );
        ensure!(
            token_types.len() == token_texts.len(),
            "tokenizer.ggml.token_type has {} entries, expected {}",
            token_types.len(),
            token_texts.len()
        );
        ensure!(
            u32::try_from(token_texts.len()).is_ok(),
            "tokenizer vocabulary is too large"
        );

        let byte_chars = byte_characters();
        let byte_values: HashMap<_, _> = byte_chars
            .iter()
            .copied()
            .enumerate()
            .map(|(index, character)| {
                let byte = u8::try_from(index).context("GPT-2 byte map index exceeds u8")?;
                Ok((character, byte))
            })
            .collect::<Result<_>>()?;
        let mut token_ids = HashMap::with_capacity(token_texts.len());
        for (index, text) in token_texts.iter().enumerate() {
            let id = u32::try_from(index).context("tokenizer token index exceeds u32")?;
            if token_ids.insert(text.clone(), id).is_some() {
                bail!("duplicate tokenizer token {text:?}");
            }
        }

        let mut token_pieces = Vec::with_capacity(token_texts.len());
        let mut special_tokens = Vec::new();
        for (index, (text, token_type)) in token_texts.iter().zip(&token_types).enumerate() {
            validate_token_type(*token_type, index)?;
            if is_special_type(*token_type) {
                token_pieces.push(text.as_bytes().to_vec());
                let id = u32::try_from(index).context("special token index exceeds u32")?;
                special_tokens.push((text.clone(), id));
            } else {
                token_pieces.push(decode_token_text(text, *token_type, &byte_values));
            }
        }
        special_tokens.sort_unstable_by(|left, right| {
            right
                .0
                .len()
                .cmp(&left.0.len())
                .then_with(|| left.1.cmp(&right.1))
        });

        let mut byte_tokens = [None; 256];
        for (byte, character) in byte_chars.iter().enumerate() {
            byte_tokens[byte] = token_ids.get(&character.to_string()).copied();
        }
        for (byte, token) in byte_tokens.iter().enumerate() {
            ensure!(
                token.is_some(),
                "tokenizer vocabulary has no byte token for 0x{byte:02x}"
            );
        }

        let mut merges = HashMap::with_capacity(merge_texts.len());
        for (rank, text) in merge_texts.iter().enumerate() {
            let (left, right) = text
                .split_once(' ')
                .ok_or_else(|| anyhow!("invalid tokenizer merge {rank}: {text:?}"))?;
            ensure!(
                !left.is_empty() && !right.is_empty() && !right.contains(' '),
                "invalid tokenizer merge {rank}: {text:?}"
            );
            let left_id = token_ids
                .get(left)
                .copied()
                .ok_or_else(|| anyhow!("merge {rank} references missing token {left:?}"))?;
            let right_id = token_ids
                .get(right)
                .copied()
                .ok_or_else(|| anyhow!("merge {rank} references missing token {right:?}"))?;
            let merged_text = format!("{left}{right}");
            let merged_id = token_ids
                .get(&merged_text)
                .copied()
                .ok_or_else(|| anyhow!("merge {rank} produces missing token {merged_text:?}"))?;
            merges.entry((left_id, right_id)).or_insert(Merge {
                rank,
                token: merged_id,
            });
        }

        let eos_token = gguf.u32("tokenizer.ggml.eos_token_id")?;
        ensure!(
            (eos_token as usize) < token_texts.len(),
            "tokenizer EOS token {eos_token} is outside the vocabulary"
        );

        Ok(Self {
            token_pieces,
            token_types,
            token_ids,
            byte_tokens,
            merges,
            special_tokens,
            eos_token,
        })
    }

    /// Encode text with Qwen2 pre-tokenization and GPT-2 byte-level BPE.
    pub(crate) fn encode(&self, text: &str, parse_special: bool) -> Result<Vec<u32>> {
        let mut output = Vec::new();
        if !parse_special {
            self.encode_ordinary(text, &mut output)?;
            return Ok(output);
        }

        let mut start = 0;
        while start < text.len() {
            let Some((offset, length, token)) = self.next_special(&text[start..]) else {
                self.encode_ordinary(&text[start..], &mut output)?;
                break;
            };
            self.encode_ordinary(&text[start..start + offset], &mut output)?;
            output.push(token);
            start += offset + length;
        }
        Ok(output)
    }

    /// Look up a token by its exact GGUF spelling.
    pub(crate) fn token_id(&self, text: &str) -> Option<u32> {
        self.token_ids.get(text).copied()
    }

    /// Return decoded bytes for one token. A UTF-8 scalar may span several pieces.
    pub(crate) fn token_piece(&self, id: u32) -> Result<&[u8]> {
        self.token_pieces
            .get(id as usize)
            .map(Vec::as_slice)
            .ok_or_else(|| anyhow!("token {id} is outside the vocabulary"))
    }

    pub(crate) fn is_special(&self, id: u32) -> bool {
        self.token_types
            .get(id as usize)
            .copied()
            .is_some_and(is_special_type)
    }

    pub(crate) fn eos_token(&self) -> u32 {
        self.eos_token
    }

    pub(crate) fn is_eos(&self, id: u32) -> bool {
        id == self.eos_token
    }

    fn encode_ordinary(&self, text: &str, output: &mut Vec<u32>) -> Result<()> {
        for word in qwen2_split(text) {
            let mut symbols = Vec::with_capacity(word.len());
            for byte in word.bytes() {
                let token = self.byte_tokens[byte as usize].ok_or_else(|| {
                    anyhow!("tokenizer vocabulary has no byte token for 0x{byte:02x}")
                })?;
                symbols.push(token);
            }
            self.apply_bpe(&mut symbols);
            output.extend(symbols);
        }
        Ok(())
    }

    fn apply_bpe(&self, symbols: &mut Vec<u32>) {
        while symbols.len() > 1 {
            let best = symbols
                .windows(2)
                .filter_map(|pair| {
                    self.merges
                        .get(&(pair[0], pair[1]))
                        .copied()
                        .map(|merge| (merge.rank, pair[0], pair[1], merge.token))
                })
                .min_by_key(|(rank, _, _, _)| *rank);
            let Some((_, left, right, merged)) = best else {
                break;
            };

            let mut read = 0;
            let mut write = 0;
            while read < symbols.len() {
                if read + 1 < symbols.len() && symbols[read] == left && symbols[read + 1] == right {
                    symbols[write] = merged;
                    read += 2;
                } else {
                    symbols[write] = symbols[read];
                    read += 1;
                }
                write += 1;
            }
            symbols.truncate(write);
        }
    }

    fn next_special(&self, text: &str) -> Option<(usize, usize, u32)> {
        let mut best: Option<(usize, usize, u32)> = None;
        for (special, token) in &self.special_tokens {
            let Some(offset) = text.find(special) else {
                continue;
            };
            let candidate = (offset, special.len(), *token);
            if best.is_none_or(|current| {
                candidate.0 < current.0 || (candidate.0 == current.0 && candidate.1 > current.1)
            }) {
                best = Some(candidate);
            }
        }
        best
    }
}

/// Incremental, strict UTF-8 decoder for token pieces that split scalars.
#[derive(Default)]
pub(crate) struct Utf8Decoder {
    pending: Vec<u8>,
}

impl Utf8Decoder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Add one piece and return the complete UTF-8 prefix, if any.
    pub(crate) fn push(&mut self, piece: &[u8]) -> Result<Option<String>> {
        self.pending.extend_from_slice(piece);
        match str::from_utf8(&self.pending) {
            Ok(text) => {
                let text = text.to_owned();
                self.pending.clear();
                Ok((!text.is_empty()).then_some(text))
            }
            Err(error) if error.error_len().is_none() => {
                let complete = error.valid_up_to();
                if complete == 0 {
                    return Ok(None);
                }
                let text = str::from_utf8(&self.pending[..complete])?.to_owned();
                self.pending.drain(..complete);
                Ok(Some(text))
            }
            Err(error) => bail!(
                "token pieces contain invalid UTF-8 at buffered byte {}",
                error.valid_up_to()
            ),
        }
    }

    /// Finish decoding, failing if the final scalar is incomplete.
    pub(crate) fn finish(&mut self) -> Result<String> {
        let text = str::from_utf8(&self.pending)
            .context("token pieces end with incomplete UTF-8")?
            .to_owned();
        self.pending.clear();
        Ok(text)
    }
}

fn validate_token_type(token_type: u32, index: usize) -> Result<()> {
    ensure!(
        matches!(
            token_type,
            TOKEN_TYPE_NORMAL
                | TOKEN_TYPE_UNKNOWN
                | TOKEN_TYPE_CONTROL
                | TOKEN_TYPE_USER_DEFINED
                | TOKEN_TYPE_UNUSED
                | TOKEN_TYPE_BYTE
        ),
        "token {index} has unknown GGML token type {token_type}"
    );
    Ok(())
}

fn is_special_type(token_type: u32) -> bool {
    matches!(token_type, TOKEN_TYPE_CONTROL | TOKEN_TYPE_USER_DEFINED)
}

fn decode_token_text(text: &str, token_type: u32, byte_values: &HashMap<char, u8>) -> Vec<u8> {
    if token_type == TOKEN_TYPE_BYTE
        && let Some(byte) = parse_byte_token(text)
    {
        return vec![byte];
    }

    let mut output = Vec::with_capacity(text.len());
    for character in text.chars() {
        if let Some(byte) = byte_values.get(&character) {
            output.push(*byte);
        } else {
            let mut encoded = [0; 4];
            output.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
        }
    }
    output
}

fn parse_byte_token(text: &str) -> Option<u8> {
    let hex = text.strip_prefix("<0x")?.strip_suffix('>')?;
    if hex.len() != 2 {
        return None;
    }
    u8::from_str_radix(hex, 16).ok()
}

fn byte_characters() -> [char; 256] {
    let mut characters = ['\0'; 256];
    let mut extra = 0_u32;
    for byte in u8::MIN..=u8::MAX {
        let codepoint = if (b'!'..=b'~').contains(&byte)
            || (0xa1..=0xac).contains(&byte)
            || (0xae..=0xff).contains(&byte)
        {
            u32::from(byte)
        } else {
            let codepoint = 256 + extra;
            extra += 1;
            codepoint
        };
        characters[usize::from(byte)] = char::from_u32(codepoint).expect("GPT-2 byte map is valid");
    }
    characters
}

fn qwen2_split(text: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let length = qwen2_word_len(&text[start..]);
        debug_assert!(length > 0);
        words.push(&text[start..start + length]);
        start += length;
    }
    words
}

fn qwen2_word_len(text: &str) -> usize {
    if let Some(length) = contraction_len(text) {
        return length;
    }

    let mut characters = text.char_indices();
    let (_, first) = characters
        .next()
        .expect("word matching requires nonempty text");
    if first.is_alphabetic() {
        return take_while_chars(text, char::is_alphabetic);
    }
    if first != '\r' && first != '\n' && !first.is_alphabetic() && !first.is_numeric() {
        let first_end = first.len_utf8();
        if text[first_end..]
            .chars()
            .next()
            .is_some_and(char::is_alphabetic)
        {
            return first_end + take_while_chars(&text[first_end..], char::is_alphabetic);
        }
    }
    if first.is_numeric() {
        return text
            .chars()
            .take_while(|character| character.is_numeric())
            .take(3)
            .map(char::len_utf8)
            .sum();
    }

    let punctuation_start = usize::from(first == ' ');
    let punctuation_text = &text[punctuation_start..];
    let punctuation_len = take_while_chars(punctuation_text, |character| {
        !character.is_whitespace() && !character.is_alphabetic() && !character.is_numeric()
    });
    if punctuation_len > 0 {
        let mut length = punctuation_start + punctuation_len;
        length += take_while_chars(&text[length..], |character| {
            character == '\r' || character == '\n'
        });
        return length;
    }

    if first.is_whitespace() {
        let whitespace_len = take_while_chars(text, char::is_whitespace);
        let whitespace = &text[..whitespace_len];
        if let Some((offset, character)) = whitespace
            .char_indices()
            .rev()
            .find(|(_, character)| *character == '\r' || *character == '\n')
        {
            return offset + character.len_utf8();
        }
        if text[whitespace_len..]
            .chars()
            .next()
            .is_some_and(|character| !character.is_whitespace())
            && whitespace.chars().nth_back(1).is_some()
        {
            return whitespace_len
                - whitespace
                    .chars()
                    .next_back()
                    .expect("nonempty whitespace has a final character")
                    .len_utf8();
        }
        return whitespace_len;
    }

    first.len_utf8()
}

fn contraction_len(text: &str) -> Option<usize> {
    const CONTRACTIONS: [&str; 7] = ["'re", "'ve", "'ll", "'s", "'t", "'m", "'d"];

    CONTRACTIONS.iter().find_map(|suffix| {
        text.get(..suffix.len())
            .filter(|prefix| prefix.eq_ignore_ascii_case(suffix))
            .map(str::len)
    })
}

fn take_while_chars(text: &str, predicate: impl Fn(char) -> bool) -> usize {
    let mut end = 0;
    for character in text.chars() {
        if !predicate(character) {
            break;
        }
        end += character.len_utf8();
    }
    end
}
