//! Finds `arbor` executables that shadow one another on `PATH`.
//!
//! A shell runs the first `arbor` on `PATH`. An old `cargo install` left in
//! `~/.cargo/bin` therefore keeps answering after a newer npm or Homebrew
//! install, silently: a review once ran 1.9.0, which counts every symbol in a
//! touched file as changed, while 3.0.3 sat unused further down `PATH`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// One `arbor` found on `PATH`, in search order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Install {
    pub path: PathBuf,
    /// `None` when it didn't answer `--version` in time.
    pub version: Option<(u64, u64, u64)>,
}

/// File names a shell would run for `arbor`.
fn executable_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["arbor.exe", "arbor.cmd", "arbor.bat"]
    } else {
        &["arbor"]
    }
}

/// Every `arbor` executable on `PATH`, first (the one that runs) first.
pub(crate) fn installs_on_path() -> Vec<Install> {
    let Some(path) = std::env::var_os("PATH") else {
        return Vec::new();
    };
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut found = Vec::new();
    for dir in std::env::split_paths(&path) {
        for name in executable_names() {
            let candidate = dir.join(name);
            if !candidate.is_file() {
                continue;
            }
            let canonical = candidate
                .canonicalize()
                .unwrap_or_else(|_| candidate.clone());
            if seen.contains(&canonical) {
                continue;
            }
            seen.push(canonical);
            let version = version_of(&candidate);
            found.push(Install {
                path: candidate,
                version,
            });
            // On Windows a directory's `.exe` wins over its `.cmd`.
            break;
        }
    }
    found
}

/// Runs `path --version`, giving up after five seconds.
fn version_of(path: &Path) -> Option<(u64, u64, u64)> {
    let mut child = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                return None;
            }
        }
    }
    let output = child.wait_with_output().ok()?;
    parse_version(&String::from_utf8_lossy(&output.stdout))
}

/// `arbor 3.0.3` → `(3, 0, 3)`.
pub(crate) fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let word = text.split_whitespace().find(|word| {
        word.trim_start_matches('v')
            .starts_with(|c: char| c.is_ascii_digit())
    })?;
    let mut parts = word
        .trim_start_matches('v')
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u64>().ok());
    Some((
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
        parts.next().flatten().unwrap_or(0),
    ))
}

/// The newest install that the first one on `PATH` hides, if any.
pub(crate) fn shadowed_newer(installs: &[Install]) -> Option<&Install> {
    let first = installs.first()?.version?;
    installs[1..]
        .iter()
        .filter(|install| install.version.is_some_and(|version| version > first))
        .max_by_key(|install| install.version)
}

pub(crate) fn format_version(version: Option<(u64, u64, u64)>) -> String {
    match version {
        Some((major, minor, patch)) => format!("{major}.{minor}.{patch}"),
        None => "unknown version".to_string(),
    }
}

/// How to remove an install, judged from where it lives.
pub(crate) fn removal_hint(path: &Path) -> String {
    let text = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    if text.contains("/.cargo/bin/") {
        "cargo uninstall arbor-graph-cli".to_string()
    } else if text.contains("/npm") || text.contains("node_modules") {
        "npm uninstall -g @anandb71/arbor-cli".to_string()
    } else if text.contains("/homebrew/")
        || text.contains("/cellar/")
        || text.contains("/linuxbrew/")
    {
        "brew uninstall arbor".to_string()
    } else if text.contains("/scoop/") {
        "scoop uninstall arbor".to_string()
    } else {
        format!("delete {}", path.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(path: &str, version: Option<(u64, u64, u64)>) -> Install {
        Install {
            path: PathBuf::from(path),
            version,
        }
    }

    #[test]
    fn versions_parse_from_the_cli_banner() {
        assert_eq!(parse_version("arbor 3.0.3\n"), Some((3, 0, 3)));
        assert_eq!(parse_version("arbor v1.9.0"), Some((1, 9, 0)));
        assert_eq!(parse_version("arbor 3.1.0-rc.1"), Some((3, 1, 0)));
        assert_eq!(parse_version("arbor 4"), Some((4, 0, 0)));
        assert_eq!(parse_version("error: unknown flag"), None);
    }

    #[test]
    fn an_older_first_install_shadows_a_newer_one() {
        let installs = [
            install("/home/u/.cargo/bin/arbor", Some((1, 9, 0))),
            install("/usr/local/bin/arbor", Some((3, 0, 3))),
            install("/opt/arbor", Some((2, 6, 0))),
        ];
        assert_eq!(
            shadowed_newer(&installs).map(|i| i.path.clone()),
            Some(PathBuf::from("/usr/local/bin/arbor"))
        );
    }

    #[test]
    fn a_newest_first_install_or_unknown_versions_are_fine() {
        let newest_first = [
            install("/usr/local/bin/arbor", Some((3, 0, 3))),
            install("/home/u/.cargo/bin/arbor", Some((1, 9, 0))),
        ];
        assert!(shadowed_newer(&newest_first).is_none());
        let unknown = [
            install("/a/arbor", None),
            install("/b/arbor", Some((3, 0, 3))),
        ];
        assert!(shadowed_newer(&unknown).is_none());
        assert!(shadowed_newer(&[]).is_none());
    }

    #[test]
    fn removal_hints_follow_the_install_location() {
        assert_eq!(
            removal_hint(Path::new(r"C:\Users\u\.cargo\bin\arbor.exe")),
            "cargo uninstall arbor-graph-cli"
        );
        assert_eq!(
            removal_hint(Path::new(r"C:\Users\u\AppData\Roaming\npm\arbor.cmd")),
            "npm uninstall -g @anandb71/arbor-cli"
        );
        assert_eq!(
            removal_hint(Path::new("/opt/homebrew/bin/arbor")),
            "brew uninstall arbor"
        );
        assert!(removal_hint(Path::new("/opt/arbor")).starts_with("delete "));
    }
}
