// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2026 Jarkko Sakkinen

//! Minimal, validated GGUF reader for the fixed CPU Qwen implementation.

use std::borrow::Cow;

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::str;

use anyhow::{Context as _, Result, anyhow, bail};
use memmap2::{Mmap, MmapOptions};
use zerocopy::FromBytes;

const GGUF_MAGIC: &[u8; 4] = b"GGUF";
const GGUF_VERSION: u32 = 3;
const DEFAULT_ALIGNMENT: usize = 32;
const MAX_DIMENSIONS: usize = 4;
const GGML_TYPE_F32: u32 = 0;
const GGML_TYPE_Q8_0: u32 = 8;
const Q8_0_BLOCK_ELEMENTS: usize = 32;
const Q8_0_BLOCK_BYTES: usize = 34;

/// Tensor storage formats supported by the CPU implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TensorType {
    F32,
    Q8_0,
}

/// An immutable tensor view into a loaded GGUF file.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tensor<'a> {
    pub(crate) dims: &'a [usize],
    pub(crate) kind: TensorType,
    data: &'a [u8],
}

impl<'a> Tensor<'a> {
    pub(crate) fn dimensions(&self) -> &[usize] {
        self.dims
    }

    pub(crate) fn tensor_type(&self) -> TensorType {
        self.kind
    }

    pub(crate) fn data(&self) -> &[u8] {
        self.data
    }

    /// Return `F32` tensor data without copying.
    pub(crate) fn f32_slice(&self) -> Result<&'a [f32]> {
        if self.kind != TensorType::F32 {
            bail!("tensor is {:?}, not F32", self.kind);
        }
        if cfg!(target_endian = "big") {
            bail!("F32 tensor views require a little-endian host");
        }
        // GGUF stores F32 little-endian, and every bit pattern is a valid f32
        // on a little-endian host. `ref_from_bytes_with_elems` validates both
        // alignment and size, so no `unsafe` is needed at the call site.
        let element_count = self.data.len() / size_of::<f32>();
        <[f32]>::ref_from_bytes_with_elems(self.data, element_count)
            .map_err(|err| anyhow!("F32 tensor does not form a valid f32 slice: {err}"))
    }

    /// Number of encoded bytes in one `Q8_0` row.
    pub(crate) fn q8_row_size(&self) -> Result<usize> {
        if self.kind != TensorType::Q8_0 {
            bail!("tensor is {:?}, not Q8_0", self.kind);
        }

        let columns = self.dims[0];
        Ok(columns / Q8_0_BLOCK_ELEMENTS * Q8_0_BLOCK_BYTES)
    }

    /// Return one encoded `Q8_0` row (`f16` scale followed by 32 i8 values per block).
    pub(crate) fn q8_row_bytes(&self, row: usize) -> Result<&[u8]> {
        let row_size = self.q8_row_size()?;
        let rows = self.data.len() / row_size;
        if row >= rows {
            bail!("Q8_0 row {row} is out of range for {rows} rows");
        }

        let start = row
            .checked_mul(row_size)
            .ok_or_else(|| anyhow!("Q8_0 row offset overflow"))?;
        Ok(&self.data[start..start + row_size])
    }
}

/// A fully loaded and validated GGUF v3 file.
pub(crate) struct Gguf {
    bytes: Mmap,
    data_start: usize,
    metadata: HashMap<String, MetadataValue>,
    tensors: HashMap<String, TensorInfo>,
}

impl Gguf {
    /// Load and validate a little-endian GGUF v3 file.
    pub(crate) fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        // Model files are verified and atomically published before they are mapped.
        let bytes = unsafe { MmapOptions::new().map(&file) }
            .with_context(|| format!("map {}", path.display()))?;
        Self::parse(bytes).with_context(|| format!("parse {}", path.display()))
    }

    fn parse(bytes: Mmap) -> Result<Self> {
        let mut reader = Reader::new(&bytes);
        if reader.take(GGUF_MAGIC.len())? != GGUF_MAGIC {
            bail!("invalid GGUF magic");
        }

        let version = reader.u32()?;
        if version != GGUF_VERSION {
            bail!("unsupported GGUF version {version}, expected {GGUF_VERSION}");
        }

        let tensor_count = reader.count("tensor count")?;
        let metadata_count = reader.count("metadata count")?;
        if metadata_count > reader.remaining() / 13 {
            bail!("metadata count {metadata_count} exceeds the file size");
        }

        let mut metadata = HashMap::new();
        for _ in 0..metadata_count {
            let name = reader.string("metadata key")?;
            validate_name(&name, "metadata key")?;
            let value_type = reader.u32()?;
            let value = MetadataValue::read(&mut reader, value_type)
                .with_context(|| format!("read metadata `{name}`"))?;
            if metadata.insert(name.clone(), value).is_some() {
                bail!("duplicate metadata key `{name}`");
            }
        }

        validate_architecture(&metadata)?;
        let alignment = metadata_alignment(&metadata)?;
        if tensor_count > reader.remaining() / 32 {
            bail!("tensor count {tensor_count} exceeds the file size");
        }

        let mut tensors = HashMap::new();
        for _ in 0..tensor_count {
            let name = reader.string("tensor name")?;
            validate_name(&name, "tensor name")?;
            let dimension_count = reader.u32()? as usize;
            if !(1..=MAX_DIMENSIONS).contains(&dimension_count) {
                bail!("tensor `{name}` has invalid dimension count {dimension_count}");
            }

            let mut dims = Vec::with_capacity(dimension_count);
            for _ in 0..dimension_count {
                let dimension = reader.usize("tensor dimension")?;
                if dimension == 0 {
                    bail!("tensor `{name}` has a zero dimension");
                }
                dims.push(dimension);
            }

            let kind = match reader.u32()? {
                GGML_TYPE_F32 => TensorType::F32,
                GGML_TYPE_Q8_0 => TensorType::Q8_0,
                value => bail!("tensor `{name}` uses unsupported GGML type {value}"),
            };
            let offset = reader.usize("tensor offset")?;
            if offset % alignment != 0 {
                bail!("tensor `{name}` offset {offset} is not {alignment}-byte aligned");
            }
            let byte_len = tensor_byte_len(&name, &dims, kind)?;
            let info = TensorInfo {
                dims,
                kind,
                offset,
                byte_len,
            };
            if tensors.insert(name.clone(), info).is_some() {
                bail!("duplicate tensor name `{name}`");
            }
        }

        let data_start = align_up(reader.position(), alignment)?;
        if data_start > bytes.len() {
            bail!("aligned tensor data start is past the end of the file");
        }
        validate_tensor_ranges(&bytes, data_start, &tensors)?;

        Ok(Self {
            bytes,
            data_start,
            metadata,
            tensors,
        })
    }

    /// Model architecture from `general.architecture`.
    pub(crate) fn architecture(&self) -> &str {
        match self.metadata.get("general.architecture") {
            Some(MetadataValue::String(value)) => value,
            _ => unreachable!("architecture was validated while loading"),
        }
    }

    pub(crate) fn string(&self, key: &str) -> Result<&str> {
        match self.metadata_value(key)? {
            MetadataValue::String(value) => Ok(value),
            value => metadata_type_error(key, "string", value),
        }
    }

    pub(crate) fn u32(&self, key: &str) -> Result<u32> {
        match self.metadata_value(key)? {
            MetadataValue::U32(value) => Ok(*value),
            value => metadata_type_error(key, "u32", value),
        }
    }

    pub(crate) fn f32(&self, key: &str) -> Result<f32> {
        match self.metadata_value(key)? {
            MetadataValue::F32(value) => Ok(*value),
            value => metadata_type_error(key, "f32", value),
        }
    }

    pub(crate) fn bool(&self, key: &str) -> Result<bool> {
        match self.metadata_value(key)? {
            MetadataValue::Bool(value) => Ok(*value),
            value => metadata_type_error(key, "bool", value),
        }
    }

    pub(crate) fn string_array(&self, key: &str) -> Result<&[String]> {
        match self.metadata_value(key)? {
            MetadataValue::Array(MetadataArray::String(values)) => Ok(values),
            value => metadata_type_error(key, "string array", value),
        }
    }

    pub(crate) fn i32_array(&self, key: &str) -> Result<Cow<'_, [i32]>> {
        match self.metadata_value(key)? {
            MetadataValue::Array(MetadataArray::I32(values)) => Ok(Cow::Borrowed(values)),
            MetadataValue::Array(MetadataArray::U32(values)) => {
                let values = values
                    .iter()
                    .map(|value| {
                        i32::try_from(*value).with_context(|| {
                            format!("GGUF metadata `{key}` value {value} does not fit i32")
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(Cow::Owned(values))
            }
            value => metadata_type_error(key, "i32 or u32 array", value),
        }
    }

    pub(crate) fn u32_array(&self, key: &str) -> Result<Cow<'_, [u32]>> {
        match self.metadata_value(key)? {
            MetadataValue::Array(MetadataArray::U32(values)) => Ok(Cow::Borrowed(values)),
            MetadataValue::Array(MetadataArray::I32(values)) => {
                let values = values
                    .iter()
                    .map(|value| {
                        u32::try_from(*value).with_context(|| {
                            format!("GGUF metadata `{key}` value {value} is negative")
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(Cow::Owned(values))
            }
            value => metadata_type_error(key, "i32 or u32 array", value),
        }
    }

    /// Look up a tensor by its GGUF name.
    pub(crate) fn tensor(&self, name: &str) -> Result<Tensor<'_>> {
        let info = self
            .tensors
            .get(name)
            .ok_or_else(|| anyhow!("missing tensor `{name}`"))?;
        let start = self
            .data_start
            .checked_add(info.offset)
            .ok_or_else(|| anyhow!("tensor `{name}` start overflow"))?;
        let end = start
            .checked_add(info.byte_len)
            .ok_or_else(|| anyhow!("tensor `{name}` end overflow"))?;
        Ok(Tensor {
            dims: &info.dims,
            kind: info.kind,
            data: &self.bytes[start..end],
        })
    }

    fn metadata_value(&self, key: &str) -> Result<&MetadataValue> {
        self.metadata
            .get(key)
            .ok_or_else(|| anyhow!("missing GGUF metadata `{key}`"))
    }
}

#[derive(Debug)]
struct TensorInfo {
    dims: Vec<usize>,
    kind: TensorType,
    offset: usize,
    byte_len: usize,
}

#[allow(dead_code)]
#[derive(Debug)]
enum MetadataValue {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    F32(f32),
    Bool(bool),
    String(String),
    Array(MetadataArray),
    U64(u64),
    I64(i64),
    F64(f64),
}

impl MetadataValue {
    fn read(reader: &mut Reader<'_>, value_type: u32) -> Result<Self> {
        match value_type {
            0 => Ok(Self::U8(reader.u8()?)),
            1 => Ok(Self::I8(reader.i8()?)),
            2 => Ok(Self::U16(reader.u16()?)),
            3 => Ok(Self::I16(reader.i16()?)),
            4 => Ok(Self::U32(reader.u32()?)),
            5 => Ok(Self::I32(reader.i32()?)),
            6 => Ok(Self::F32(reader.f32()?)),
            7 => Ok(Self::Bool(reader.bool()?)),
            8 => Ok(Self::String(reader.string("metadata string")?)),
            9 => Ok(Self::Array(MetadataArray::read(reader)?)),
            10 => Ok(Self::U64(reader.u64()?)),
            11 => Ok(Self::I64(reader.i64()?)),
            12 => Ok(Self::F64(reader.f64()?)),
            _ => bail!("unknown GGUF metadata type {value_type}"),
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            Self::U8(_) => "u8",
            Self::I8(_) => "i8",
            Self::U16(_) => "u16",
            Self::I16(_) => "i16",
            Self::U32(_) => "u32",
            Self::I32(_) => "i32",
            Self::F32(_) => "f32",
            Self::Bool(_) => "bool",
            Self::String(_) => "string",
            Self::Array(value) => value.type_name(),
            Self::U64(_) => "u64",
            Self::I64(_) => "i64",
            Self::F64(_) => "f64",
        }
    }
}

#[allow(dead_code)]
#[derive(Debug)]
enum MetadataArray {
    U8(Vec<u8>),
    I8(Vec<i8>),
    U16(Vec<u16>),
    I16(Vec<i16>),
    U32(Vec<u32>),
    I32(Vec<i32>),
    F32(Vec<f32>),
    Bool(Vec<bool>),
    String(Vec<String>),
    U64(Vec<u64>),
    I64(Vec<i64>),
    F64(Vec<f64>),
}

impl MetadataArray {
    fn read(reader: &mut Reader<'_>) -> Result<Self> {
        let element_type = reader.u32()?;
        if element_type == 9 {
            bail!("nested GGUF metadata arrays are invalid");
        }
        if element_type > 12 {
            bail!("unknown GGUF metadata array type {element_type}");
        }

        let count = reader.count("metadata array length")?;
        let minimum_size = match element_type {
            0 | 1 | 7 => 1,
            2 | 3 => 2,
            4..=6 => 4,
            8 | 10..=12 => 8,
            _ => unreachable!(),
        };
        if count > reader.remaining() / minimum_size {
            bail!("metadata array length {count} exceeds the file size");
        }

        macro_rules! read_array {
            ($method:ident) => {{
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push(reader.$method()?);
                }
                values
            }};
        }

        match element_type {
            0 => Ok(Self::U8(read_array!(u8))),
            1 => Ok(Self::I8(read_array!(i8))),
            2 => Ok(Self::U16(read_array!(u16))),
            3 => Ok(Self::I16(read_array!(i16))),
            4 => Ok(Self::U32(read_array!(u32))),
            5 => Ok(Self::I32(read_array!(i32))),
            6 => Ok(Self::F32(read_array!(f32))),
            7 => Ok(Self::Bool(read_array!(bool))),
            8 => {
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push(reader.string("metadata array string")?);
                }
                Ok(Self::String(values))
            }
            10 => Ok(Self::U64(read_array!(u64))),
            11 => Ok(Self::I64(read_array!(i64))),
            12 => Ok(Self::F64(read_array!(f64))),
            _ => unreachable!(),
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            Self::U8(_) => "u8 array",
            Self::I8(_) => "i8 array",
            Self::U16(_) => "u16 array",
            Self::I16(_) => "i16 array",
            Self::U32(_) => "u32 array",
            Self::I32(_) => "i32 array",
            Self::F32(_) => "f32 array",
            Self::Bool(_) => "bool array",
            Self::String(_) => "string array",
            Self::U64(_) => "u64 array",
            Self::I64(_) => "i64 array",
            Self::F64(_) => "f64 array",
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn position(&self) -> usize {
        self.position
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| anyhow!("GGUF offset overflow at byte {}", self.position))?;
        if end > self.bytes.len() {
            bail!(
                "truncated GGUF at byte {}: need {length} bytes, have {}",
                self.position,
                self.remaining()
            );
        }

        let value = &self.bytes[self.position..end];
        self.position = end;
        Ok(value)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?
            .try_into()
            .map_err(|_| anyhow!("GGUF scalar has an invalid byte width"))
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()?.cast_signed())
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.array()?))
    }

    fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.array()?))
    }

    fn bool(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => bail!("invalid GGUF boolean value {value}"),
        }
    }

    fn usize(&mut self, description: &str) -> Result<usize> {
        let value = self.u64()?;
        usize::try_from(value).map_err(|_| anyhow!("{description} {value} does not fit usize"))
    }

    fn count(&mut self, description: &str) -> Result<usize> {
        self.usize(description)
    }

    fn string(&mut self, description: &str) -> Result<String> {
        let length = self.usize(&format!("{description} length"))?;
        let position = self.position;
        let bytes = self.take(length)?;
        let value = str::from_utf8(bytes)
            .with_context(|| format!("{description} at byte {position} is not UTF-8"))?;
        Ok(value.to_owned())
    }
}

fn validate_name(name: &str, description: &str) -> Result<()> {
    if name.is_empty() {
        bail!("empty {description}");
    }
    if name.contains('\0') {
        bail!("{description} contains NUL");
    }
    Ok(())
}

fn validate_architecture(metadata: &HashMap<String, MetadataValue>) -> Result<()> {
    let architecture = match metadata.get("general.architecture") {
        Some(MetadataValue::String(value)) => value.as_str(),
        Some(value) => {
            bail!(
                "GGUF metadata `general.architecture` is {}, expected string",
                value.type_name()
            )
        }
        None => bail!("missing GGUF metadata `general.architecture`"),
    };
    if !matches!(
        architecture,
        "qwen2" | "qwen2moe" | "qwen3" | "qwen3moe" | "qwen3vl" | "clip"
    ) {
        bail!("unsupported GGUF architecture `{architecture}`");
    }
    Ok(())
}

fn metadata_alignment(metadata: &HashMap<String, MetadataValue>) -> Result<usize> {
    let alignment = match metadata.get("general.alignment") {
        Some(MetadataValue::U32(value)) => *value as usize,
        Some(value) => {
            bail!(
                "GGUF metadata `general.alignment` is {}, expected u32",
                value.type_name()
            )
        }
        None => DEFAULT_ALIGNMENT,
    };
    if !alignment.is_power_of_two() {
        bail!("GGUF alignment {alignment} is not a nonzero power of two");
    }
    Ok(alignment)
}

fn tensor_byte_len(name: &str, dims: &[usize], kind: TensorType) -> Result<usize> {
    let elements = dims.iter().try_fold(1usize, |count, dimension| {
        count
            .checked_mul(*dimension)
            .ok_or_else(|| anyhow!("tensor `{name}` element count overflow"))
    })?;

    match kind {
        TensorType::F32 => elements
            .checked_mul(size_of::<f32>())
            .ok_or_else(|| anyhow!("tensor `{name}` byte size overflow")),
        TensorType::Q8_0 => {
            if !dims[0].is_multiple_of(Q8_0_BLOCK_ELEMENTS) {
                bail!(
                    "Q8_0 tensor `{name}` row width {} is not divisible by {Q8_0_BLOCK_ELEMENTS}",
                    dims[0]
                );
            }
            let blocks = elements / Q8_0_BLOCK_ELEMENTS;
            blocks
                .checked_mul(Q8_0_BLOCK_BYTES)
                .ok_or_else(|| anyhow!("tensor `{name}` byte size overflow"))
        }
    }
}

fn align_up(value: usize, alignment: usize) -> Result<usize> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(|| anyhow!("GGUF data alignment overflow"))
}

fn validate_tensor_ranges(
    bytes: &[u8],
    data_start: usize,
    tensors: &HashMap<String, TensorInfo>,
) -> Result<()> {
    let mut ranges = Vec::with_capacity(tensors.len());
    for (name, info) in tensors {
        let start = data_start
            .checked_add(info.offset)
            .ok_or_else(|| anyhow!("tensor `{name}` start overflow"))?;
        let end = start
            .checked_add(info.byte_len)
            .ok_or_else(|| anyhow!("tensor `{name}` end overflow"))?;
        if end > bytes.len() {
            bail!(
                "tensor `{name}` range {start}..{end} exceeds file size {}",
                bytes.len()
            );
        }
        if info.kind == TensorType::F32
            && !(bytes[start..].as_ptr() as usize).is_multiple_of(align_of::<f32>())
        {
            bail!("F32 tensor `{name}` data is not aligned");
        }
        ranges.push((start, end, name.as_str()));
    }

    ranges.sort_unstable_by_key(|(start, _, _)| *start);
    for pair in ranges.windows(2) {
        let (_, previous_end, previous_name) = pair[0];
        let (next_start, _, next_name) = pair[1];
        if previous_end > next_start {
            bail!("tensor `{previous_name}` overlaps tensor `{next_name}`");
        }
    }
    Ok(())
}

fn metadata_type_error<T>(key: &str, expected: &str, value: &MetadataValue) -> Result<T> {
    bail!(
        "GGUF metadata `{key}` is {}, expected {expected}",
        value.type_name()
    )
}
