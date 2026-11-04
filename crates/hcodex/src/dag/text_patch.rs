//! Pure, path-free experiment; not connected to model dispatch or file effects.

use std::ops::Range;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Untrusted patch description, validated against exact source bytes by `compile`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TextPatch {
    pub preimage_sha256: String,
    pub hunks: Vec<TextPatchHunk>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TextPatchHunk {
    pub start_line: usize,
    pub end_line: usize,
    pub range_sha256: String,
    pub replacement: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HunkGeometry {
    pub source: Range<usize>,
    pub result: Range<usize>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct CompiledTextPatch {
    pub content: Vec<u8>,
    pub geometry: Vec<HunkGeometry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TextPatchError {
    EmptySource,
    SourceTooLarge,
    SourceContainsNul,
    SourceNotUtf8,
    InvalidWholeHash,
    WholeHashMismatch,
    InvalidHunkCount,
    InvalidRange { hunk: usize },
    UnorderedOrOverlapping { hunk: usize },
    ReplacementContainsNul { hunk: usize },
    ReplacementTooLarge,
    InvalidRangeHash { hunk: usize },
    RangeOutOfBounds { hunk: usize },
    RangeHashMismatch { hunk: usize },
    OutputTooLarge,
}

impl std::fmt::Display for TextPatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "text patch rejected: {self:?}")
    }
}

impl std::error::Error for TextPatchError {}

impl TextPatch {
    pub const MAX_BYTES: usize = 1024 * 1024;
    pub const MAX_HUNKS: usize = 128;

    /// Validate and compile in memory. No path, fallback, or effect authority.
    /// Ranges address original lines including their LF, CRLF or CR terminator;
    /// replacement bytes are verbatim, including any intentional line joining.
    pub fn compile(&self, source: &[u8]) -> Result<CompiledTextPatch, TextPatchError> {
        if source.len() > Self::MAX_BYTES {
            return Err(TextPatchError::SourceTooLarge);
        }
        if source.is_empty() {
            return Err(TextPatchError::EmptySource);
        }
        if source.contains(&0) {
            return Err(TextPatchError::SourceContainsNul);
        }
        std::str::from_utf8(source).map_err(|_| TextPatchError::SourceNotUtf8)?;
        if !canonical_hash(&self.preimage_sha256) {
            return Err(TextPatchError::InvalidWholeHash);
        }
        if !hash_matches(source, &self.preimage_sha256) {
            return Err(TextPatchError::WholeHashMismatch);
        }
        self.validate_hunks()?;
        let spans = self.source_spans(source)?;
        let output_size =
            self.hunks
                .iter()
                .zip(&spans)
                .try_fold(source.len(), |size, (hunk, span)| {
                    size.checked_sub(span.len())
                        .and_then(|size| size.checked_add(hunk.replacement.len()))
                        .ok_or(TextPatchError::OutputTooLarge)
                })?;
        if output_size > Self::MAX_BYTES {
            return Err(TextPatchError::OutputTooLarge);
        }
        let mut content = Vec::with_capacity(output_size);
        let mut geometry = Vec::with_capacity(spans.len());
        let mut cursor = 0;
        for (hunk, span) in self.hunks.iter().zip(spans) {
            content.extend_from_slice(&source[cursor..span.start]);
            let start = content.len();
            content.extend_from_slice(hunk.replacement.as_bytes());
            cursor = span.end;
            geometry.push(HunkGeometry {
                source: span,
                result: start..content.len(),
            });
        }
        content.extend_from_slice(&source[cursor..]);
        Ok(CompiledTextPatch { content, geometry })
    }

    fn validate_hunks(&self) -> Result<(), TextPatchError> {
        if self.hunks.is_empty() || self.hunks.len() > Self::MAX_HUNKS {
            return Err(TextPatchError::InvalidHunkCount);
        }
        let mut previous_end = 0;
        let mut replacement_bytes = 0_usize;
        for (index, hunk) in self.hunks.iter().enumerate() {
            if hunk.start_line == 0 || hunk.end_line < hunk.start_line {
                return Err(TextPatchError::InvalidRange { hunk: index });
            }
            if hunk.start_line <= previous_end {
                return Err(TextPatchError::UnorderedOrOverlapping { hunk: index });
            }
            previous_end = hunk.end_line;
            if hunk.replacement.contains('\0') {
                return Err(TextPatchError::ReplacementContainsNul { hunk: index });
            }
            replacement_bytes = replacement_bytes
                .checked_add(hunk.replacement.len())
                .filter(|size| *size <= Self::MAX_BYTES)
                .ok_or(TextPatchError::ReplacementTooLarge)?;
            if !canonical_hash(&hunk.range_sha256) {
                return Err(TextPatchError::InvalidRangeHash { hunk: index });
            }
        }
        Ok(())
    }

    fn source_spans(&self, source: &[u8]) -> Result<Vec<Range<usize>>, TextPatchError> {
        // Traverse original lines once; retain only one span per bounded hunk,
        // rather than allocating an index entry for every source line.
        let mut lines = line_spans(source).enumerate();
        let mut spans = Vec::with_capacity(self.hunks.len());
        for (index, hunk) in self.hunks.iter().enumerate() {
            let mut start = None;
            let mut found = None;
            for (ordinal, line) in lines.by_ref() {
                let number = ordinal + 1;
                if number == hunk.start_line {
                    start = Some(line.start);
                }
                if number == hunk.end_line {
                    found = start.map(|start| start..line.end);
                    break;
                }
            }
            let span = found.ok_or(TextPatchError::RangeOutOfBounds { hunk: index })?;
            if !hash_matches(&source[span.clone()], &hunk.range_sha256) {
                return Err(TextPatchError::RangeHashMismatch { hunk: index });
            }
            spans.push(span);
        }
        Ok(spans)
    }
}

fn canonical_hash(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn hash_matches(bytes: &[u8], expected: &str) -> bool {
    format!("sha256:{:x}", Sha256::digest(bytes)) == expected
}

fn line_spans(source: &[u8]) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut cursor = 0;
    std::iter::from_fn(move || {
        if cursor == source.len() {
            return None;
        }
        let start = cursor;
        while cursor < source.len() && !matches!(source[cursor], b'\r' | b'\n') {
            cursor += 1;
        }
        if cursor < source.len() {
            let cr = source[cursor] == b'\r';
            cursor += 1;
            if cr && source.get(cursor) == Some(&b'\n') {
                cursor += 1;
            }
        }
        Some(start..cursor)
    })
}

#[cfg(test)]
mod tests;
