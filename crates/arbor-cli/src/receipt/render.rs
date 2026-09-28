//! Receipt wording. Plain words only: "uses this", "pages", "sign in" — never
//! "callers", "entry points" or "blast radius" in text a person reads.

use super::{Affects, ChangedFile, Receipt, RouteKind};

const MAX_TESTS: usize = 4;

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn quote(prompt: &str) -> String {
    let single_line = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut short: String = single_line.chars().take(72).collect();
    if single_line.chars().count() > 72 {
        short.push('…');
    }
    short
}

/// Things to try by hand, most specific first.
pub fn tests(files: &[ChangedFile], affects: &Affects) -> Vec<String> {
    let mut tests = Vec::new();
    // Sensitive areas the turn touched outside the request come first.
    for file in files.iter().filter(|f| f.in_scope == Some(false)) {
        for area in &file.areas {
            let test = area.test().to_string();
            if !tests.contains(&test) {
                tests.push(test);
            }
        }
    }
    for surface in affects.routes.iter().filter(|s| s.kind == RouteKind::Page) {
        tests.push(format!("Open {} and check it works", surface.route));
    }
    for area in &affects.areas {
        let test = area.test().to_string();
        if !tests.contains(&test) {
            tests.push(test);
        }
    }
    for surface in affects.routes.iter().filter(|s| s.kind == RouteKind::Api) {
        tests.push(format!("Use the feature that calls {}", surface.route));
    }
    if tests.is_empty() {
        tests.push("Run the app and try what you asked for".into());
    }
    tests.truncate(MAX_TESTS);
    tests
}

/// The full receipt, as shown after the agent finishes.
/// Pages and APIs named in "Could affect" before the rest are counted.
const SHOWN_ROUTES: usize = 4;

pub fn text(receipt: &Receipt) -> String {
    let mut out = Vec::new();
    let functions: usize = receipt.files.iter().map(|f| f.functions.len()).sum();
    let mut head = format!("Arbor receipt · \"{}\"", quote(&receipt.prompt));
    if receipt.prompt.trim().is_empty() {
        head = "Arbor receipt".into();
    }
    out.push(head);
    let mut changed = format!("Changed {}", plural(receipt.files.len(), "file", "files"));
    if functions > 0 {
        changed.push_str(&format!(
            " · {}",
            plural(functions, "function", "functions")
        ));
    }
    out.push(changed);

    let unasked: Vec<&ChangedFile> = receipt.unasked().collect();
    let undo_hint = (!unasked.is_empty() && !receipt.before.is_empty()).then(|| {
        format!(
            "  Undo {}: `arbor receipt undo {} --unasked`",
            if unasked.len() == 1 { "it" } else { "them" },
            receipt.id
        )
    });
    if !unasked.is_empty() {
        out.push(String::new());
        // Sign in, payments, data and settings can break quietly, so they warn.
        let sensitive = unasked
            .iter()
            .any(|f| f.areas.iter().any(|a| a.sensitive()));
        if sensitive {
            out.push(format!("⚠ Not in your request ({}):", unasked.len()));
        } else {
            out.push(format!(
                "Also changed, not in your request ({}):",
                unasked.len()
            ));
        }
        for file in unasked {
            let mut line = format!("  {}", file.path);
            let areas: Vec<&str> = file.areas.iter().map(|a| a.label()).collect();
            if !areas.is_empty() {
                line.push_str(&format!(" · {}", areas.join(", ")));
            }
            if !file.functions.is_empty() {
                let names: Vec<&str> = file.functions.iter().take(3).map(String::as_str).collect();
                line.push_str(&format!(" · {}", names.join(", ")));
            }
            out.push(line);
        }
        out.extend(undo_hint);
    }

    let mut affect_parts: Vec<String> = Vec::new();
    affect_parts.extend(receipt.affects.areas.iter().map(|a| a.label().to_string()));
    let routes = &receipt.affects.routes;
    affect_parts.extend(routes.iter().take(SHOWN_ROUTES).map(|s| match s.kind {
        RouteKind::Page => format!("{} page", s.route),
        RouteKind::Api => format!("{} API", s.route),
    }));
    if routes.len() > SHOWN_ROUTES {
        affect_parts.push(plural(
            routes.len() - SHOWN_ROUTES,
            "more page or API",
            "more pages and APIs",
        ));
    }
    if receipt.affects.callers > 0 {
        affect_parts.push(plural(
            receipt.affects.callers,
            "other function uses this code",
            "other functions use this code",
        ));
    }
    out.push(String::new());
    if affect_parts.is_empty() {
        out.push("Could affect: nothing else we can see uses this code".into());
    } else {
        out.push(format!("Could affect: {}", affect_parts.join(" · ")));
    }

    out.push(String::new());
    out.push("Test before you ship:".into());
    for (index, test) in receipt.tests.iter().enumerate() {
        out.push(format!("  {}. {test}", index + 1));
    }
    for limit in &receipt.limits {
        out.push(format!("Note: {limit}"));
    }
    out.push(format!("Saved as {} · `arbor receipt show`", receipt.id));
    out.join("\n")
}

/// One line per receipt for `arbor receipt list`.
pub fn line(receipt: &Receipt) -> String {
    let unasked = receipt.unasked().count();
    let flag = if unasked > 0 {
        format!(" · ⚠ {} not asked for", plural(unasked, "file", "files"))
    } else {
        String::new()
    };
    format!(
        "{}  {}  \"{}\" · {}{}",
        receipt.id,
        receipt.finished_at[..16.min(receipt.finished_at.len())].replace('T', " "),
        quote(&receipt.prompt),
        plural(receipt.files.len(), "file", "files"),
        flag
    )
}

#[cfg(test)]
mod tests {
    use super::super::{Area, ChangeKind, Surface};
    use super::*;

    fn file(
        path: &str,
        in_scope: Option<bool>,
        areas: Vec<Area>,
        functions: &[&str],
    ) -> ChangedFile {
        ChangedFile {
            path: path.into(),
            old_path: None,
            change: ChangeKind::Modified,
            additions: 3,
            deletions: 1,
            functions: functions.iter().map(|f| f.to_string()).collect(),
            areas,
            in_scope,
        }
    }

    fn receipt(files: Vec<ChangedFile>, affects: Affects) -> Receipt {
        let tests = tests(&files, &affects);
        Receipt {
            version: 1,
            id: "20260929T101203-abcdef12".into(),
            agent: "claude-code".into(),
            prompt: "make the pricing button green".into(),
            started_at: "2026-09-29T10:11:00Z".into(),
            finished_at: "2026-09-29T10:12:03Z".into(),
            scope_known: true,
            files,
            affects,
            tests,
            limits: vec![],
            before: "4b825dc6".into(),
            after: "9d1f2e3a".into(),
        }
    }

    #[test]
    fn leads_with_what_was_not_asked_for_and_tests_it_first() {
        let r = receipt(
            vec![
                file(
                    "components/PricingButton.tsx",
                    Some(true),
                    vec![],
                    &["PricingButton"],
                ),
                file(
                    "lib/auth/session.ts",
                    Some(false),
                    vec![Area::SignIn],
                    &["refreshSession"],
                ),
            ],
            Affects {
                routes: vec![Surface {
                    kind: RouteKind::Page,
                    route: "/pricing".into(),
                }],
                areas: vec![Area::SignIn],
                entry_points: vec![],
                callers: 6,
            },
        );
        let text = text(&r);
        assert!(
            text.contains(
                "⚠ Not in your request (1):\n  lib/auth/session.ts · Sign in · refreshSession\n  Undo it: `arbor receipt undo 20260929T101203-abcdef12 --unasked`"
            ),
            "{text}"
        );
        assert!(
            text.contains(
                "Could affect: Sign in · /pricing page · 6 other functions use this code"
            ),
            "{text}"
        );
        assert_eq!(r.tests[0], "Sign out, then sign back in");
        assert_eq!(r.tests[1], "Open /pricing and check it works");
        for jargon in ["caller", "entry point", "blast radius", "upstream"] {
            assert!(
                !text.to_lowercase().contains(jargon),
                "jargon {jargon:?} in {text}"
            );
        }
    }

    #[test]
    fn a_quiet_change_still_gets_something_to_try() {
        let r = receipt(
            vec![file("README.md", Some(true), vec![], &[])],
            Affects {
                routes: vec![],
                areas: vec![],
                entry_points: vec![],
                callers: 0,
            },
        );
        assert_eq!(
            r.tests,
            vec!["Run the app and try what you asked for".to_string()]
        );
        assert!(text(&r).contains("Could affect: nothing else we can see uses this code"));
    }

    #[test]
    fn a_long_list_of_pages_is_counted_not_listed() {
        let page = |route: &str| Surface {
            kind: RouteKind::Page,
            route: route.into(),
        };
        let r = receipt(
            vec![file("components/Nav.tsx", Some(true), vec![], &["Nav"])],
            Affects {
                routes: [
                    "/",
                    "/changelog",
                    "/dashboard",
                    "/pricing",
                    "/settings",
                    "/signup",
                ]
                .map(page)
                .to_vec(),
                areas: vec![],
                entry_points: vec![],
                callers: 0,
            },
        );
        assert!(
            text(&r).contains(
                "Could affect: / page · /changelog page · /dashboard page · /pricing page · 2 more pages and APIs"
            ),
            "{}",
            text(&r)
        );
    }
}
