//! Was a changed file part of what the user asked for?
//!
//! Deliberately conservative: a file is in scope when its path or changed
//! functions share a word with the request, or when it is one call away from a
//! file that does. When the request names nothing we can match, nothing is
//! flagged and the receipt says it could not tell.

use std::collections::HashSet;

use super::app::words;

/// Words too common in paths or requests to say anything about scope.
const NOISE: &[&str] = &[
    // request filler
    "the",
    "and",
    "for",
    "with",
    "make",
    "please",
    "can",
    "could",
    "would",
    "should",
    "you",
    "this",
    "that",
    "these",
    "those",
    "add",
    "fix",
    "change",
    "update",
    "into",
    "from",
    "our",
    "my",
    "use",
    "using",
    "also",
    "just",
    "not",
    "but",
    "all",
    "any",
    "some",
    "new",
    "get",
    "set",
    "let",
    "now",
    "then",
    "when",
    "what",
    "how",
    "why",
    "where",
    "there",
    "here",
    "have",
    "has",
    "need",
    "want",
    "like",
    "more",
    "less",
    "it",
    "its",
    "is",
    "are",
    "was",
    "be",
    "to",
    "of",
    "in",
    "on",
    "at",
    "by",
    "an",
    "or",
    "so",
    "do",
    "does",
    "doing",
    "done",
    "try",
    "bug",
    "issue",
    "error",
    "work",
    "working",
    "broken",
    "code",
    "file",
    "files",
    "function",
    "thing",
    "stuff",
    // path structure
    "src",
    "app",
    "lib",
    "libs",
    "components",
    "component",
    "utils",
    "util",
    "index",
    "main",
    "mod",
    "pages",
    "routes",
    "route",
    "server",
    "client",
    "public",
    "core",
    "common",
    "shared",
    "helpers",
    "helper",
    "types",
    "type",
    "tsx",
    "ts",
    "jsx",
    "js",
    "rs",
    "py",
    "go",
    "java",
    "kt",
    "swift",
    "dart",
    "vue",
    "svelte",
    "css",
    "scss",
    "html",
    "json",
    "md",
    "test",
    "tests",
    "spec",
    "hooks",
    "hook",
    "api",
    "page",
];

fn meaningful(text: &str) -> HashSet<String> {
    words(text)
        .into_iter()
        .filter(|word| word.len() >= 3 && !NOISE.contains(&word.as_str()))
        .map(|word| arbor_graph::stem(&word))
        .collect()
}

/// What the scope check knows about one changed file.
pub struct FileFacts<'a> {
    pub path: &'a str,
    pub functions: &'a [String],
    /// Indexes of other changed files this one calls or is called by.
    pub neighbours: &'a [usize],
    /// Package manifests and lockfiles follow from the change they support.
    pub supporting: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Scope {
    /// The request named nothing we could match to a file.
    Unknown,
    /// Whether each file (in input order) is within the request.
    Known(Vec<bool>),
}

pub fn classify(prompt: &str, files: &[FileFacts]) -> Scope {
    let asked = meaningful(prompt);
    if asked.is_empty() {
        return Scope::Unknown;
    }
    let direct: Vec<bool> = files
        .iter()
        .map(|file| {
            let mut terms = meaningful(file.path);
            for name in file.functions {
                terms.extend(meaningful(name));
            }
            !terms.is_disjoint(&asked)
        })
        .collect();
    if !direct.iter().any(|hit| *hit) {
        return Scope::Unknown;
    }
    Scope::Known(
        files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                direct[index]
                    || file.supporting
                    || file
                        .neighbours
                        .iter()
                        .any(|other| direct.get(*other).copied().unwrap_or(false))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file<'a>(path: &'a str, functions: &'a [String], neighbours: &'a [usize]) -> FileFacts<'a> {
        FileFacts {
            path,
            functions,
            neighbours,
            supporting: false,
        }
    }

    #[test]
    fn flags_a_file_the_request_never_reached() {
        let none: Vec<String> = vec![];
        let session = vec!["refreshSession".to_string()];
        let files = [
            file("components/PricingButton.tsx", &none, &[]),
            file("app/pricing/page.tsx", &none, &[0]),
            file("lib/auth/session.ts", &session, &[]),
        ];
        assert_eq!(
            classify("make the pricing button green", &files),
            Scope::Known(vec![true, true, false])
        );
    }

    #[test]
    fn a_neighbour_of_an_asked_file_is_in_scope() {
        let search = vec!["searchRecipes".to_string()];
        let none: Vec<String> = vec![];
        let files = [
            file("lib/recipes.ts", &search, &[]),
            file("lib/db.ts", &none, &[0]),
        ];
        assert_eq!(
            classify("add recipe search", &files),
            Scope::Known(vec![true, true])
        );
    }

    #[test]
    fn a_vague_request_flags_nothing() {
        let none: Vec<String> = vec![];
        let files = [file("lib/auth/session.ts", &none, &[])];
        assert_eq!(classify("fix the bug please", &files), Scope::Unknown);
        assert_eq!(classify("make it work", &files), Scope::Unknown);
    }

    #[test]
    fn package_changes_support_the_ask() {
        let none: Vec<String> = vec![];
        let files = [
            file("components/Chart.tsx", &none, &[]),
            FileFacts {
                path: "package.json",
                functions: &none,
                neighbours: &[],
                supporting: true,
            },
        ];
        assert_eq!(
            classify("add a chart", &files),
            Scope::Known(vec![true, true])
        );
    }
}
