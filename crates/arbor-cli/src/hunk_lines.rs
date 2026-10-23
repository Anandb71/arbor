//! Which lines of a diff hunk change code.
//!
//! A hunk that only adds a function's doc comment, its `#[test]` attribute
//! and a blank line after it touches lines outside the function's own span.
//! Counted as they are, those lines made the enclosing `mod tests` or class
//! "modified" too, and a comment-only edit implicated its whole symbol.
//!
//! So, per hunk:
//!
//! - If it removes any code line, every line it touches counts: deleting or
//!   commenting out code is a change wherever it lands.
//! - Otherwise only its added code lines count. Blank lines and comments are
//!   dropped. An attribute or decorator belongs to the item it annotates, and
//!   parsers keep it outside the item's span, so it counts as a change to
//!   the item's first line.
//!
//! Classification is per line and per language, from the hunk alone. Where a
//! line is ambiguous (a `*` line whose block comment opened before the hunk)
//! it counts as code: overstating is safer than hiding an edit.

use arbor_graph::ChangedRange;

/// A hunk's header and the lines it removes and adds, without their `-`/`+`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Hunk {
    pub header: String,
    pub removed: Vec<String>,
    pub added: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    Blank,
    Comment,
    /// `#[derive]`, `@Get('/x')`: part of the item below.
    Annotation,
    Code,
}

struct Syntax {
    line_comment: Option<&'static str>,
    hash_comment: bool,
    block_comment: bool,
    rust_attributes: bool,
    decorators: bool,
}

fn syntax_for(path: &str) -> Option<Syntax> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    let c_family = |decorators| Syntax {
        line_comment: Some("//"),
        hash_comment: false,
        block_comment: true,
        rust_attributes: false,
        decorators,
    };
    Some(match ext.as_str() {
        "rs" => Syntax {
            rust_attributes: true,
            ..c_family(false)
        },
        "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" | "java" | "kt" | "kts"
        | "scala" | "dart" | "swift" => c_family(true),
        "go" | "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "cs" | "php" => c_family(false),
        "py" | "pyi" => Syntax {
            line_comment: None,
            hash_comment: true,
            block_comment: false,
            rust_attributes: false,
            decorators: true,
        },
        "rb" | "sh" | "bash" | "zsh" => Syntax {
            line_comment: None,
            hash_comment: true,
            block_comment: false,
            rust_attributes: false,
            decorators: false,
        },
        _ => return None,
    })
}

/// Classifies a run of consecutive lines, tracking block comments that open
/// inside the run.
fn classify(lines: &[String], syntax: &Syntax) -> Vec<Line> {
    let mut in_block = false;
    lines
        .iter()
        .map(|raw| {
            let line = raw.trim();
            if in_block {
                if line.contains("*/") {
                    in_block = false;
                    // Code may follow the comment on the same line.
                    let rest = line
                        .split_once("*/")
                        .map(|(_, rest)| rest.trim())
                        .unwrap_or("");
                    return if rest.is_empty() {
                        Line::Comment
                    } else {
                        Line::Code
                    };
                }
                return Line::Comment;
            }
            if line.is_empty() {
                return Line::Blank;
            }
            if syntax.block_comment && line.starts_with("/*") {
                if let Some((_, rest)) = line[2..].split_once("*/") {
                    return if rest.trim().is_empty() {
                        Line::Comment
                    } else {
                        Line::Code
                    };
                }
                in_block = true;
                return Line::Comment;
            }
            if syntax
                .line_comment
                .is_some_and(|prefix| line.starts_with(prefix))
            {
                return Line::Comment;
            }
            if syntax.rust_attributes && (line.starts_with("#[") || line.starts_with("#![")) {
                return Line::Annotation;
            }
            if syntax.hash_comment && line.starts_with('#') {
                return Line::Comment;
            }
            if syntax.decorators
                && line.starts_with('@')
                && line[1..].starts_with(|c: char| c.is_alphabetic() || c == '_')
            {
                return Line::Annotation;
            }
            Line::Code
        })
        .collect()
}

/// `@@ -a,b +c,d @@` → `(c, d)`; an omitted count is 1.
fn new_side(header: &str) -> Option<(u32, u32)> {
    let plus = header.strip_prefix("@@")?.split('+').nth(1)?;
    let spec = plus.split_whitespace().next()?;
    let mut parts = spec.split(',');
    let start = parts.next()?.parse().ok()?;
    let count = match parts.next() {
        Some(count) => count.parse().ok()?,
        None => 1,
    };
    Some((start, count))
}

/// The ranges of `hunk` (in the new file) that change code.
pub(crate) fn code_ranges(hunk: &Hunk, path: &str) -> Vec<ChangedRange> {
    let whole = || arbor_graph::parse_unified_diff_ranges(&hunk.header, path);
    let Some(syntax) = syntax_for(path) else {
        return whole();
    };
    let Some((start, count)) = new_side(&hunk.header) else {
        return whole();
    };
    let removed = classify(&hunk.removed, &syntax);
    if removed
        .iter()
        .any(|line| matches!(line, Line::Code | Line::Annotation))
    {
        return whole();
    }
    // Only blank lines and comments were removed, and nothing was added in
    // their place: no code changed.
    if count == 0 || hunk.added.len() != count as usize {
        return if count == 0 { Vec::new() } else { whole() };
    }

    let added = classify(&hunk.added, &syntax);
    let mut lines: Vec<u32> = Vec::new();
    let mut annotated = false;
    for (offset, kind) in added.iter().enumerate() {
        let number = start + offset as u32;
        match kind {
            Line::Blank | Line::Comment => {}
            Line::Annotation => annotated = true,
            Line::Code => {
                annotated = false;
                lines.push(number);
            }
        }
    }
    // An annotation added above an existing item: the item starts right
    // after the hunk.
    if annotated {
        lines.push(start + count);
    }

    let mut ranges: Vec<ChangedRange> = Vec::new();
    for number in lines {
        match ranges.last_mut() {
            Some(last) if last.end_line + 1 == number => last.end_line = number,
            _ => ranges.push(ChangedRange::new(path, number, number)),
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk(header: &str, removed: &[&str], added: &[&str]) -> Hunk {
        Hunk {
            header: header.to_string(),
            removed: removed.iter().map(|s| s.to_string()).collect(),
            added: added.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn spans(ranges: &[ChangedRange]) -> Vec<(u32, u32)> {
        ranges.iter().map(|r| (r.start_line, r.end_line)).collect()
    }

    #[test]
    fn a_documented_test_counts_only_its_own_lines() {
        // The hunk that made `mod tests` look modified.
        let h = hunk(
            "@@ -509,0 +510,6 @@ pub mod mac {",
            &[],
            &[
                "        /// Rejected before anything is read.",
                "        #[test]",
                "        fn refused() {",
                "            assert!(true);",
                "        }",
                "",
            ],
        );
        assert_eq!(spans(&code_ranges(&h, "src/policy.rs")), [(512, 514)]);
    }

    #[test]
    fn comments_and_blank_lines_alone_change_no_code() {
        let h = hunk("@@ -10,0 +11,3 @@", &[], &["// note", "", "/* more */"]);
        assert!(code_ranges(&h, "src/lib.rs").is_empty());
        let jsdoc = hunk("@@ -4,0 +5,3 @@", &[], &["/**", " * Explains.", " */"]);
        assert!(code_ranges(&jsdoc, "src/app.ts").is_empty());
        let deleted_comment = hunk("@@ -7,2 +6,0 @@", &["# old note", ""], &[]);
        assert!(code_ranges(&deleted_comment, "app.py").is_empty());
    }

    #[test]
    fn removing_code_counts_every_touched_line() {
        // Commenting code out is a change, though the new line is a comment.
        let h = hunk(
            "@@ -12 +12 @@",
            &["    charge(card);"],
            &["    // charge(card);"],
        );
        assert_eq!(spans(&code_ranges(&h, "src/pay.rs")), [(12, 12)]);
        // A pure deletion keeps its anchor.
        let deletion = hunk("@@ -20,2 +19,0 @@", &["x += 1;", "y += 1;"], &[]);
        assert_eq!(spans(&code_ranges(&deletion, "src/pay.rs")), [(19, 19)]);
        // Removing an attribute changes the item.
        let attribute = hunk("@@ -3 +2,0 @@", &["#[cfg(test)]"], &[]);
        assert_eq!(spans(&code_ranges(&attribute, "src/lib.rs")), [(2, 2)]);
    }

    #[test]
    fn an_annotation_above_an_existing_item_reaches_the_item() {
        let h = hunk("@@ -40,0 +41 @@", &[], &["    @Get('/drafts')"]);
        assert_eq!(
            spans(&code_ranges(&h, "src/drafts.controller.ts")),
            [(42, 42)]
        );
        let py = hunk("@@ -8,0 +9,2 @@", &[], &["@cache", "# why"]);
        assert_eq!(spans(&code_ranges(&py, "app.py")), [(11, 11)]);
    }

    #[test]
    fn code_after_a_closing_comment_still_counts() {
        let h = hunk("@@ -1,0 +1,2 @@", &[], &["/* start", "end */ run();"]);
        assert_eq!(spans(&code_ranges(&h, "src/lib.rs")), [(2, 2)]);
        let inline = hunk("@@ -1,0 +1 @@", &[], &["/* why */ run();"]);
        assert_eq!(spans(&code_ranges(&inline, "src/lib.rs")), [(1, 1)]);
    }

    #[test]
    fn unknown_languages_and_odd_hunks_keep_the_whole_range() {
        let h = hunk("@@ -1,0 +1,2 @@", &[], &["# heading", ""]);
        assert_eq!(spans(&code_ranges(&h, "README.md")), [(1, 2)]);
        // The body doesn't match the header: don't guess.
        let short = hunk("@@ -1,0 +1,3 @@", &[], &["// a"]);
        assert_eq!(spans(&code_ranges(&short, "src/lib.rs")), [(1, 3)]);
    }

    #[test]
    fn decorators_are_not_email_or_matrix_operators() {
        // `@` followed by a non-letter is code (Python's matrix multiply).
        let h = hunk("@@ -1,0 +1 @@", &[], &["@ weights"]);
        assert_eq!(spans(&code_ranges(&h, "nn.py")), [(1, 1)]);
    }
}
