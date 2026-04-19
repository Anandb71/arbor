//! Lightweight fallback parser for emerging language support.
//!
//! This parser is intentionally heuristic-based (line scanner + simple token rules)
//! so Arbor can provide useful symbol indexing for languages that are not yet
//! wired to a full Tree-sitter grammar in every runtime path.

use crate::node::{CodeNode, NodeKind};

/// Extra language extensions supported via fallback parsing.
pub const FALLBACK_EXTENSIONS: &[&str] = &[
    "kt", "kts",   // Kotlin
    "swift", // Swift
    "rb",    // Ruby
    "php", "phtml", // PHP
    "sh", "bash", "zsh", // Shell
];

pub fn is_fallback_supported_extension(ext: &str) -> bool {
    let ext = ext.to_ascii_lowercase();
    FALLBACK_EXTENSIONS.iter().any(|e| *e == ext)
}

pub fn parse_fallback_source(source: &str, file_path: &str, ext: &str) -> Vec<CodeNode> {
    let ext = ext.to_ascii_lowercase();
    let mut nodes = Vec::new();

    for (idx, line) in source.lines().enumerate() {
        let line_no = idx as u32 + 1;
        let trimmed = line.trim_start();

        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }

        let candidate = match ext.as_str() {
            "kt" | "kts" => parse_kotlin_line(trimmed),
            "swift" => parse_swift_line(trimmed),
            "rb" => parse_ruby_line(trimmed),
            "php" | "phtml" => parse_php_line(trimmed),
            "sh" | "bash" | "zsh" => parse_shell_line(trimmed),
            _ => None,
        };

        if let Some((name, kind)) = candidate {
            let col = (line.len().saturating_sub(trimmed.len())) as u32;
            let node = CodeNode::new(&name, &name, kind, file_path)
                .with_lines(line_no, line_no)
                .with_column(col)
                .with_signature(trimmed.to_string());
            nodes.push(node);
        }
    }

    nodes
}

fn parse_kotlin_line(line: &str) -> Option<(String, NodeKind)> {
    if let Some(rest) = line.strip_prefix("fun ") {
        return take_ident(rest).map(|name| (name, NodeKind::Function));
    }

    if let Some(rest) = line.strip_prefix("class ") {
        return take_ident(rest).map(|name| (name, NodeKind::Class));
    }

    if let Some(rest) = line.strip_prefix("data class ") {
        return take_ident(rest).map(|name| (name, NodeKind::Class));
    }

    if let Some(rest) = line.strip_prefix("interface ") {
        return take_ident(rest).map(|name| (name, NodeKind::Interface));
    }

    if let Some(rest) = line.strip_prefix("object ") {
        return take_ident(rest).map(|name| (name, NodeKind::Class));
    }

    if let Some(rest) = line.strip_prefix("enum class ") {
        return take_ident(rest).map(|name| (name, NodeKind::Enum));
    }

    None
}

fn parse_swift_line(line: &str) -> Option<(String, NodeKind)> {
    if let Some(rest) = line.strip_prefix("func ") {
        return take_ident(rest).map(|name| (name, NodeKind::Function));
    }

    if let Some(rest) = line.strip_prefix("class ") {
        return take_ident(rest).map(|name| (name, NodeKind::Class));
    }

    if let Some(rest) = line.strip_prefix("struct ") {
        return take_ident(rest).map(|name| (name, NodeKind::Struct));
    }

    if let Some(rest) = line.strip_prefix("enum ") {
        return take_ident(rest).map(|name| (name, NodeKind::Enum));
    }

    if let Some(rest) = line.strip_prefix("protocol ") {
        return take_ident(rest).map(|name| (name, NodeKind::Interface));
    }

    if let Some(rest) = line.strip_prefix("extension ") {
        return take_ident(rest).map(|name| (name, NodeKind::Module));
    }

    None
}

fn parse_ruby_line(line: &str) -> Option<(String, NodeKind)> {
    if let Some(rest) = line.strip_prefix("def ") {
        return take_ident(rest.trim_start_matches("self.")).map(|name| (name, NodeKind::Function));
    }

    if let Some(rest) = line.strip_prefix("class ") {
        return take_ident(rest).map(|name| (name, NodeKind::Class));
    }

    if let Some(rest) = line.strip_prefix("module ") {
        return take_ident(rest).map(|name| (name, NodeKind::Module));
    }

    None
}

fn parse_php_line(line: &str) -> Option<(String, NodeKind)> {
    if let Some(rest) = line.strip_prefix("function ") {
        return take_ident(rest).map(|name| (name, NodeKind::Function));
    }

    if let Some(rest) = line.strip_prefix("class ") {
        return take_ident(rest).map(|name| (name, NodeKind::Class));
    }

    if let Some(rest) = line.strip_prefix("interface ") {
        return take_ident(rest).map(|name| (name, NodeKind::Interface));
    }

    if let Some(rest) = line.strip_prefix("trait ") {
        return take_ident(rest).map(|name| (name, NodeKind::Interface));
    }

    None
}

fn parse_shell_line(line: &str) -> Option<(String, NodeKind)> {
    if let Some(rest) = line.strip_prefix("function ") {
        return take_ident(rest).map(|name| (name, NodeKind::Function));
    }

    // foo() {
    if let Some(paren_idx) = line.find("()") {
        let name = line[..paren_idx].trim();
        if !name.is_empty() {
            return Some((name.to_string(), NodeKind::Function));
        }
    }

    None
}

fn take_ident(input: &str) -> Option<String> {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            break;
        }
    }

    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_supports_requested_extensions() {
        for ext in ["kt", "swift", "rb", "php", "sh"] {
            assert!(is_fallback_supported_extension(ext));
        }
    }

    #[test]
    fn parses_kotlin_function() {
        let source = "fun fetchUser(id: String): User = TODO()";
        let nodes = parse_fallback_source(source, "sample.kt", "kt");
        assert!(nodes.iter().any(|n| n.name == "fetchUser"));
    }

    #[test]
    fn parses_shell_function() {
        let source = "deploy_prod() { echo hi; }";
        let nodes = parse_fallback_source(source, "deploy.sh", "sh");
        assert!(nodes.iter().any(|n| n.name == "deploy_prod"));
    }

    #[test]
    fn parses_swift_source() {
        let source = r#"
class UserManager {
    func getUser() -> User {
        return User()
    }
}

struct Point {
    var x: Double
    var y: Double
}

enum Status {
    case active
    case inactive
}
"#;
        let nodes = parse_fallback_source(source, "Users.swift", "swift");
        assert!(nodes.iter().any(|n| n.name == "UserManager" && matches!(n.kind, NodeKind::Class)));
        assert!(nodes.iter().any(|n| n.name == "getUser" && matches!(n.kind, NodeKind::Function)));
        assert!(nodes.iter().any(|n| n.name == "Point" && matches!(n.kind, NodeKind::Struct)));
        assert!(nodes.iter().any(|n| n.name == "Status" && matches!(n.kind, NodeKind::Enum)));
    }

    #[test]
    fn parses_ruby_source() {
        let source = r#"
class ApplicationController
  def index
    render json: { status: "ok" }
  end

  def show
    @user = User.find(params[:id])
  end
end

module Authentication
end
"#;
        let nodes = parse_fallback_source(source, "controller.rb", "rb");
        assert!(nodes.iter().any(|n| n.name == "ApplicationController" && matches!(n.kind, NodeKind::Class)));
        assert!(nodes.iter().any(|n| n.name == "index" && matches!(n.kind, NodeKind::Function)));
        assert!(nodes.iter().any(|n| n.name == "show" && matches!(n.kind, NodeKind::Function)));
        assert!(nodes.iter().any(|n| n.name == "Authentication" && matches!(n.kind, NodeKind::Module)));
    }

    #[test]
    fn parses_php_source() {
        let source = r#"
class PaymentProcessor {
    function processPayment($amount) {
        return true;
    }
}

interface Gateway {
}

function helper() {
}
"#;
        let nodes = parse_fallback_source(source, "payment.php", "php");
        assert!(nodes.iter().any(|n| n.name == "PaymentProcessor" && matches!(n.kind, NodeKind::Class)));
        assert!(nodes.iter().any(|n| n.name == "processPayment" && matches!(n.kind, NodeKind::Function)));
        assert!(nodes.iter().any(|n| n.name == "Gateway" && matches!(n.kind, NodeKind::Interface)));
        assert!(nodes.iter().any(|n| n.name == "helper" && matches!(n.kind, NodeKind::Function)));
    }

    #[test]
    fn fallback_ignores_comments() {
        // Lines starting with // or # should not produce nodes
        let source = r#"
// class NotAClass
# def not_a_function
fun realFunction(x: Int): Int = x
"#;
        let nodes = parse_fallback_source(source, "test.kt", "kt");
        assert!(!nodes.iter().any(|n| n.name == "NotAClass"));
        assert!(!nodes.iter().any(|n| n.name == "not_a_function"));
        assert!(nodes.iter().any(|n| n.name == "realFunction"));
    }

    #[test]
    fn fallback_kotlin_class_and_data_class() {
        let source = r#"
class Repository {
}
data class UserDto(val name: String)
object Singleton
"#;
        let nodes = parse_fallback_source(source, "models.kt", "kt");
        assert!(nodes.iter().any(|n| n.name == "Repository" && matches!(n.kind, NodeKind::Class)));
        assert!(nodes.iter().any(|n| n.name == "UserDto" && matches!(n.kind, NodeKind::Class)));
        assert!(nodes.iter().any(|n| n.name == "Singleton" && matches!(n.kind, NodeKind::Class)));
    }

    #[test]
    fn fallback_empty_source_returns_empty() {
        let nodes = parse_fallback_source("", "empty.kt", "kt");
        assert!(nodes.is_empty());
    }

    #[test]
    fn fallback_unsupported_extension_returns_empty() {
        // Make sure unsupported extensions don't panic
        assert!(!is_fallback_supported_extension("xyz"));
        assert!(!is_fallback_supported_extension("rs"));
    }

    #[test]
    fn parses_shell_function_keyword() {
        let source = "function deploy_staging { echo staging; }";
        let nodes = parse_fallback_source(source, "deploy.bash", "bash");
        assert!(nodes.iter().any(|n| n.name == "deploy_staging"));
    }
}
