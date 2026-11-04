use super::*;
use sha2::{Digest, Sha256};

fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn hunk(start: usize, end: usize, bytes: &[u8], replacement: &str) -> TextPatchHunk {
    TextPatchHunk {
        start_line: start,
        end_line: end,
        range_sha256: hash(bytes),
        replacement: replacement.to_owned(),
    }
}

fn patch(source: &[u8], hunks: Vec<TextPatchHunk>) -> TextPatch {
    TextPatch {
        preimage_sha256: hash(source),
        hunks,
    }
}

#[test]
fn compiles_known_sha256_fixture_without_effects() -> crate::Result<()> {
    let digest = "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let patch = TextPatch {
        preimage_sha256: digest.into(),
        hunks: vec![TextPatchHunk {
            start_line: 1,
            end_line: 1,
            range_sha256: digest.into(),
            replacement: "XYZ".into(),
        }],
    };
    assert_eq!(
        patch.compile(b"abc")?,
        CompiledTextPatch {
            content: b"XYZ".to_vec(),
            geometry: vec![HunkGeometry {
                source: 0..3,
                result: 0..3
            }],
        }
    );
    Ok(())
}

#[test]
fn multiple_hunks_use_original_coordinates_and_verbatim_replacements() -> crate::Result<()> {
    let source = b"a\r\nb\rc\nd";
    let compiled = patch(
        source,
        vec![hunk(1, 1, b"a\r\n", "AA\nextra\n"), hunk(3, 3, b"c\n", "")],
    )
    .compile(source)?;
    assert_eq!(compiled.content, b"AA\nextra\nb\rd");
    assert_eq!(
        compiled.geometry,
        vec![
            HunkGeometry {
                source: 0..3,
                result: 0..9
            },
            HunkGeometry {
                source: 5..7,
                result: 11..11
            },
        ]
    );
    assert_eq!(
        patch(
            b"a\nb\n",
            vec![hunk(1, 1, b"a\n", "A"), hunk(2, 2, b"b\n", "B")]
        )
        .compile(b"a\nb\n")?
        .content,
        b"AB"
    );
    Ok(())
}

#[test]
fn unicode_bom_final_line_empty_result_and_noop_are_byte_exact() -> crate::Result<()> {
    let source = "\u{feff}한글\r\n끝".as_bytes();
    let result = patch(source, vec![hunk(2, 2, "끝".as_bytes(), "終\n")]).compile(source)?;
    assert_eq!(result.content, "\u{feff}한글\r\n終\n".as_bytes());
    assert_eq!(
        result.geometry,
        vec![HunkGeometry {
            source: 11..14,
            result: 11..15
        }]
    );
    assert_eq!(
        patch(b"a", vec![hunk(1, 1, b"a", "")])
            .compile(b"a")?
            .content,
        b""
    );
    assert_eq!(
        patch(b"a", vec![hunk(1, 1, b"a", "a")])
            .compile(b"a")?
            .content,
        b"a"
    );
    Ok(())
}

#[test]
fn multiline_range_includes_original_mixed_terminators() -> crate::Result<()> {
    let source = b"a\r\nb\rc\n";
    let result = patch(source, vec![hunk(1, 2, b"a\r\nb\r", "joined\n")]).compile(source)?;
    assert_eq!(result.content, b"joined\nc\n");
    assert_eq!(
        result.geometry,
        vec![HunkGeometry {
            source: 0..5,
            result: 0..7
        }]
    );
    Ok(())
}

#[test]
fn rejects_stale_and_malformed_hashes() {
    let source = b"a\nb\n";
    let mut p = patch(source, vec![hunk(1, 1, b"a\n", "A\n")]);
    p.preimage_sha256 = hash(b"different");
    assert_eq!(p.compile(source), Err(TextPatchError::WholeHashMismatch));
    p.preimage_sha256 = hash(source).to_uppercase();
    assert_eq!(p.compile(source), Err(TextPatchError::InvalidWholeHash));
    p.preimage_sha256 = hash(source);
    p.hunks[0].range_sha256 = hash(b"b\n");
    assert_eq!(
        p.compile(source),
        Err(TextPatchError::RangeHashMismatch { hunk: 0 })
    );
    p.hunks[0].range_sha256 = "sha256:absent".into();
    assert_eq!(
        p.compile(source),
        Err(TextPatchError::InvalidRangeHash { hunk: 0 })
    );
    for malformed in [
        "sha256:123".to_owned(),
        format!("sha256:{}", "g".repeat(64)),
    ] {
        p.preimage_sha256 = malformed.clone();
        assert_eq!(p.compile(source), Err(TextPatchError::InvalidWholeHash));
        p.preimage_sha256 = hash(source);
        p.hunks[0].range_sha256 = malformed;
        assert_eq!(
            p.compile(source),
            Err(TextPatchError::InvalidRangeHash { hunk: 0 })
        );
    }
}

#[test]
fn rejects_invalid_ranges_and_phantom_final_lines() {
    for (start, end) in [(0, 1), (2, 1)] {
        assert_eq!(
            patch(b"a\n", vec![hunk(start, end, b"a\n", "A")]).compile(b"a\n"),
            Err(TextPatchError::InvalidRange { hunk: 0 })
        );
    }
    for source in [b"a\n".as_slice(), b"a\r", b"a\r\n", b"a"] {
        assert_eq!(
            patch(source, vec![hunk(2, 2, b"", "A")]).compile(source),
            Err(TextPatchError::RangeOutOfBounds { hunk: 0 })
        );
    }
    for ranges in [[(1, 2), (2, 2)], [(2, 2), (1, 1)]] {
        let p = patch(
            b"a\nb\n",
            ranges
                .into_iter()
                .map(|(s, e)| hunk(s, e, b"", "x"))
                .collect(),
        );
        assert_eq!(
            p.compile(b"a\nb\n"),
            Err(TextPatchError::UnorderedOrOverlapping { hunk: 1 })
        );
    }
}

#[test]
fn rejects_unsupported_text_and_resource_limits() {
    for (source, error) in [
        (b"".as_slice(), TextPatchError::EmptySource),
        (b"a\0", TextPatchError::SourceContainsNul),
        (b"\xff", TextPatchError::SourceNotUtf8),
    ] {
        assert_eq!(
            patch(source, vec![hunk(1, 1, source, "A")]).compile(source),
            Err(error)
        );
    }
    assert_eq!(
        patch(b"a", vec![]).compile(b"a"),
        Err(TextPatchError::InvalidHunkCount)
    );
    assert_eq!(
        patch(b"a", vec![hunk(1, 1, b"a", "x"); TextPatch::MAX_HUNKS + 1]).compile(b"a"),
        Err(TextPatchError::InvalidHunkCount)
    );
    assert_eq!(
        patch(b"a", vec![hunk(1, 1, b"a", "\0")]).compile(b"a"),
        Err(TextPatchError::ReplacementContainsNul { hunk: 0 })
    );
    let oversized = vec![b'a'; TextPatch::MAX_BYTES + 1];
    assert_eq!(
        patch(&oversized, vec![hunk(1, 1, b"", "x")]).compile(&oversized),
        Err(TextPatchError::SourceTooLarge)
    );
    let replacement = "x".repeat(TextPatch::MAX_BYTES);
    assert!(
        patch(b"a", vec![hunk(1, 1, b"a", &replacement)])
            .compile(b"a")
            .is_ok()
    );
    assert_eq!(
        patch(b"a\nb", vec![hunk(1, 1, b"a\n", &replacement)]).compile(b"a\nb"),
        Err(TextPatchError::OutputTooLarge)
    );
    assert_eq!(
        patch(
            b"a\nb",
            vec![hunk(1, 1, b"a\n", &replacement), hunk(2, 2, b"b", "x")]
        )
        .compile(b"a\nb"),
        Err(TextPatchError::ReplacementTooLarge)
    );
}

#[test]
fn wire_input_rejects_unknown_fields() {
    assert!(
        serde_json::from_str::<TextPatch>(r#"{"preimage_sha256":"x","hunks":[],"path":"a.rs"}"#)
            .is_err()
    );
    assert!(
        serde_json::from_str::<TextPatchHunk>(
            r#"{"start_line":1,"end_line":1,"range_sha256":"x","replacement":"a","fallback":true}"#
        )
        .is_err()
    );
}

#[test]
fn finite_corpus_preserves_unselected_bytes() -> crate::Result<()> {
    // Independent splice oracle: offsets follow the fixture's byte lengths,
    // not the compiler's line scanner or returned geometry.
    for first in ["a\n", "한\r\n", "\u{feff}x\r"] {
        for middle in ["b\n", "나\r\n", "y\r"] {
            for last in ["c", "끝\n", "z\r\n"] {
                let source = format!("{first}{middle}{last}");
                for replacement in ["", "X", "줄\n둘\r\n"] {
                    let result = patch(
                        source.as_bytes(),
                        vec![hunk(2, 2, middle.as_bytes(), replacement)],
                    )
                    .compile(source.as_bytes())?;
                    assert_eq!(
                        result.content,
                        format!("{first}{replacement}{last}").as_bytes()
                    );
                    assert_eq!(
                        result.geometry,
                        vec![HunkGeometry {
                            source: first.len()..first.len() + middle.len(),
                            result: first.len()..first.len() + replacement.len()
                        }]
                    );
                }
            }
        }
    }
    Ok(())
}
