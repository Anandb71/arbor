//! Path classification and display helpers shared by CLI and MCP.

/// Whether a file path looks like a test, spec, or fixture.
///
/// Matches on path segments and conventional suffixes rather than a raw
/// `contains("test")` substring, so `contest/handler.rs` is not treated as a test.
pub fn is_test_file(file_path: &str) -> bool {
    let lower = file_path.to_lowercase().replace('\\', "/");
    let segments: Vec<&str> = lower.split('/').collect();
    let filename = segments.last().copied().unwrap_or("");

    segments.iter().any(|s| {
        *s == "test"
            || *s == "tests"
            || *s == "spec"
            || *s == "specs"
            || *s == "fixture"
            || *s == "fixtures"
            || *s == "mock"
            || *s == "mocks"
            || *s == "__tests__"
            || *s == "__mocks__"
            || *s == "testfixtures"
    }) || lower.ends_with("test.java")
        || lower.ends_with("tests.java")
        || lower.ends_with("test.rs")
        || lower.ends_with("test.ts")
        || lower.ends_with("test.tsx")
        || lower.ends_with("test.js")
        || lower.ends_with("test.jsx")
        || lower.ends_with("test.py")
        || lower.ends_with("test.go")
        || lower.ends_with("_test.go")
        || lower.ends_with("tests.cs")
        || lower.ends_with("test.cs")
        || lower.ends_with("_test.dart")
        || lower.contains(".spec.")
        || lower.contains(".test.")
        || filename.starts_with("test_")
        || filename == "conftest.py"
}

/// Bundled, hashed, or generated assets that should not appear in a map.
pub fn is_minified_or_generated(file_path: &str) -> bool {
    let lower = file_path.to_lowercase().replace('\\', "/");
    lower.ends_with(".min.js")
        || lower.ends_with(".min.css")
        || lower.contains(".chunk.")
        || lower.contains(".bundle.")
        || lower.contains("/dist/")
        || lower.contains("/build/")
        || lower.contains("/resources/monitor/")
        || lower.contains("/resources/static/")
        || lower.contains("/generated/")
        || {
            let filename = lower.rsplit('/').next().unwrap_or("");
            let parts: Vec<&str> = filename.split('.').collect();
            parts.len() >= 3
                && parts[1].len() >= 8
                && parts[1].chars().all(|c| c.is_ascii_hexdigit())
        }
}

/// Compress a fully-qualified signature to `name(arg, arg)` for map output.
pub fn shorten_signature(sig: &str) -> String {
    let sig = sig.trim();
    let paren_start = match sig.find('(') {
        Some(i) => i,
        None => {
            if sig.len() <= 80 {
                return sig.to_string();
            }
            return format!("{}...", &sig[..77]);
        }
    };

    let before_paren = &sig[..paren_start];
    let name = before_paren
        .split_whitespace()
        .last()
        .unwrap_or(before_paren)
        .trim();

    let paren_end = match sig.rfind(')') {
        Some(i) => i,
        None => return format!("{}(...)", name),
    };

    let params_str = &sig[paren_start + 1..paren_end];
    let param_names = extract_param_names(params_str);

    let result = if param_names.is_empty() {
        format!("{}()", name)
    } else {
        format!("{}({})", name, param_names.join(", "))
    };

    if result.len() > 80 {
        format!("{}(...)", name)
    } else {
        result
    }
}

fn extract_param_names(params_str: &str) -> Vec<&str> {
    if params_str.trim().is_empty() {
        return Vec::new();
    }

    let mut names = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0;
    let bytes = params_str.as_bytes();

    for (i, byte) in bytes.iter().enumerate() {
        match byte {
            b'<' | b'(' => depth += 1,
            b'>' | b')' => depth -= 1,
            b',' if depth == 0 => {
                if let Some(name) = last_word_of_param(&params_str[start..i]) {
                    names.push(name);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if let Some(name) = last_word_of_param(&params_str[start..]) {
        names.push(name);
    }
    names
}

fn last_word_of_param(param: &str) -> Option<&str> {
    let trimmed = param.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(colon_pos) = trimmed.find(':') {
        let before_colon = trimmed[..colon_pos].trim();
        return before_colon.split_whitespace().last();
    }
    trimmed.split_whitespace().last()
}

/// Strip `root` from a stored node path, using forward slashes.
pub fn make_relative(file_path: &str, root: &str) -> String {
    file_path
        .strip_prefix(root)
        .unwrap_or(file_path)
        .trim_start_matches('/')
        .trim_start_matches('\\')
        .replace('\\', "/")
}

/// Collapse a long path to `first/.../grandparent/parent/file`.
pub fn compress_path(file_path: &str, root: &str) -> String {
    let relative = make_relative(file_path, root);
    let parts: Vec<&str> = relative.split('/').collect();

    if parts.len() <= 4 {
        return relative;
    }

    let first = parts[0];
    let filename = parts[parts.len() - 1];
    let parent = parts[parts.len() - 2];
    let grandparent = parts[parts.len() - 3];

    format!("{first}/.../{grandparent}/{parent}/{filename}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_matches_conventional_layouts() {
        assert!(is_test_file("src/tests/foo.rs"));
        assert!(is_test_file("src/foo.test.ts"));
        assert!(is_test_file("pkg/conftest.py"));
        assert!(!is_test_file("src/lib.rs"));
        assert!(!is_test_file("src/handlers/user.rs"));
    }

    #[test]
    fn minified_and_hashed_assets_are_filtered() {
        assert!(is_minified_or_generated("web/app.min.js"));
        assert!(is_minified_or_generated("web/main.d094b1b69ba24b63.js"));
        assert!(!is_minified_or_generated("src/main.rs"));
    }

    #[test]
    fn shorten_signature_keeps_param_names() {
        assert_eq!(
            shorten_signature("pub fn index(path: &Path, limit: usize)"),
            "index(path, limit)"
        );
        assert_eq!(shorten_signature("function fetch()"), "fetch()");
    }
}
