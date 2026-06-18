//! Claude Code harness: CLAUDE.md directives + `.claude/settings.json` hooks +
//! arbor command permissions.

use super::{Harness, Result, Scope};
use colored::Colorize;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Markers wrapping the Arbor block in CLAUDE.md so re-running `arbor hook`
/// replaces it in place instead of appending a duplicate.
const BEGIN_MARKER: &str =
    "<!-- BEGIN arbor-claude-guidance (auto-installed by `arbor hook claude`; remove this block to disable) -->";
const END_MARKER: &str = "<!-- END arbor-claude-guidance -->";

/// arbor commands allow-listed so the agent runs them without a prompt.
const PERMISSIONS: &[&str] = &[
    "Bash(arbor query *)",
    "Bash(arbor file-graph *)",
    "Bash(arbor callers *)",
    "Bash(arbor callees *)",
    "Bash(arbor map *)",
    "Bash(arbor path *)",
    "Bash(arbor inspect *)",
    "Bash(arbor diff *)",
    "Bash(arbor entry-points *)",
    "Bash(arbor refactor *)",
    "Bash(arbor export *)",
];

pub struct Claude;

impl Harness for Claude {
    fn apply(&self, scope: &Scope) -> Result<()> {
        let root = match scope {
            Scope::Project(p) => p.clone(),
            Scope::Global => dirs::home_dir().ok_or("could not resolve home directory")?,
        };

        apply_directives(&root, scope)?;
        apply_settings(&root)?;

        println!(
            "{} Arbor wired into Claude Code at {}",
            "✓".green(),
            root.display()
        );
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// CLAUDE.md directives
// ---------------------------------------------------------------------------

/// Locate the CLAUDE.md to edit (root preferred, then `.claude/`), or pick the
/// default location to create one.
fn claude_md_path(root: &Path, scope: &Scope) -> PathBuf {
    match scope {
        // Global config conventionally lives at ~/.claude/CLAUDE.md.
        Scope::Global => root.join(".claude").join("CLAUDE.md"),
        Scope::Project(_) => {
            let at_root = root.join("CLAUDE.md");
            if at_root.exists() {
                return at_root;
            }
            let in_dir = root.join(".claude").join("CLAUDE.md");
            if in_dir.exists() {
                return in_dir;
            }
            // Neither exists — create at project root (the common convention).
            at_root
        }
    }
}

fn apply_directives(root: &Path, scope: &Scope) -> Result<()> {
    let path = claude_md_path(root, scope);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let existing = fs::read_to_string(&path).unwrap_or_default();
    let updated = upsert_block(&existing, &directives_block());

    if updated == existing {
        println!("  {} CLAUDE.md already up to date", "•".dimmed());
        return Ok(());
    }

    let verb = if existing.is_empty() {
        "created"
    } else {
        "updated"
    };
    fs::write(&path, updated)?;
    println!("  {} {} {}", "✓".green(), verb, path.display());
    Ok(())
}

/// Replace the existing marker-delimited Arbor block, or append a fresh one. A
/// brand-new file gets a `# CLAUDE.md` header before the block.
fn upsert_block(existing: &str, block: &str) -> String {
    if let (Some(start), Some(end)) = (existing.find(BEGIN_MARKER), existing.find(END_MARKER)) {
        let end = end + END_MARKER.len();
        let mut out = String::with_capacity(existing.len());
        out.push_str(&existing[..start]);
        out.push_str(block);
        out.push_str(&existing[end..]);
        return out;
    }

    if existing.is_empty() {
        return format!("# CLAUDE.md\n\n{block}\n");
    }

    let mut out = existing.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(block);
    out.push('\n');
    out
}

fn directives_block() -> String {
    // The arbor guidance mirrors the reference CLAUDE.md, wrapped in markers so
    // re-running the command updates it in place.
    format!("{BEGIN_MARKER}\n{GUIDANCE}{END_MARKER}")
}

const GUIDANCE: &str = r#"## Code Navigation (MANDATORY)

This project is indexed by Arbor. **You MUST use arbor for all codebase exploration.** grep, rg, and find are blocked by hooks and will fail.

### Rules

1. **NEVER use `rg` for anything.** It is blocked. Use `arbor query "term" .` instead.
2. **NEVER use recursive grep** (`grep -r`, `grep -R`, piped from find). Use `arbor query` or `arbor callers`.
3. **NEVER use `find . -name`** to discover files. Use `arbor query "pattern" .` or `arbor file-graph "path" .`.
4. **Allowed grep**: `grep "pattern" /explicit/single/file.java` — only on a specific, known file you've already identified via arbor.
5. **NEVER Read a file to "explore" it.** Use `arbor file-graph "path" .` first to see structure, THEN Read the specific line range.

### Decision tree

| Intent | Command |
|--------|---------|
| "Where is X defined?" | `arbor query "X" . --exclude-test` |
| "What calls X?" | `arbor callers "X" .` |
| "What does X call?" | `arbor callees "X" .` |
| "What's in this file?" | `arbor file-graph "path" .` |
| "How does A connect to B?" | `arbor path "A" "B" .` |
| "What changed?" | `arbor diff .` |
| "Multi-term search" | `arbor query "term1\|term2" . --exclude-test` |

### Project map (auto-injected)

A ranked project skeleton is automatically injected into context on your first tool call each day via a PostToolUse hook. You do NOT need to run it manually — it arrives as output from your first Bash call.

The map shows the most important symbols sorted by PageRank centrality. Entry points are marked with ★. Use it to orient before diving deeper.

To manually re-run with different options:
```bash
arbor map . --exclude-test                    # default: 1024 token budget
arbor map . --exclude-test --tokens 2048      # more detail
arbor map . --exclude-test --focus "service"  # boost symbols in service layer
arbor map . --exclude-test --focus-changed    # boost symbols in files you're editing
```

### Finding symbols
- `arbor query "name" . --exclude-test` — fuzzy search for symbols (production code only)
- `arbor query "term1|term2|term3" . --exclude-test` — multi-term OR search (replaces grep with alternation)
- `arbor inspect "symbol" .` — full detail on one symbol (file, lines, role, centrality, caller/callee counts)

### Understanding relationships
- `arbor callers "symbol" .` — who calls this? (one hop upstream)
- `arbor callees "symbol" .` — what does this call? (one hop downstream)
- `arbor path "start" "end" .` — shortest path between two symbols in the call graph

### Structural views
- `arbor file-graph "src/path/File.java" .` — all symbols + internal edges within a file
- `arbor entry-points .` — list HTTP handlers, main functions, webhooks, jobs

### Impact analysis
- `arbor diff . --json` — blast radius of current git changes
- `arbor refactor "symbol" .` — blast radius of changing a specific symbol

### All commands support `--json` for structured output.

**Important:** Do NOT redirect stderr with `2>/dev/null` on arbor commands. First-run indexing logs go to stderr — suppressing them makes it look like the command is hanging.

### Workflow

1. **Orient**: `arbor map . --exclude-test` — understand the project structure
2. **Locate**: `arbor query "name" . --exclude-test` — find specific symbols
3. **Navigate**: `arbor callers`/`callees`/`path` — trace relationships
4. **Read**: Only use `Read` AFTER arbor has identified the specific file and line range you need

When `arbor query` returns test files but you need production code, do NOT fall back to grep. Instead:
1. Pick a symbol from the results (e.g., a test field or builder method)
2. Run `arbor callers "symbol" .` to trace upstream into production code
3. Or run `arbor file-graph "src/main/..." .` if you already know the production file path

"#;

// ---------------------------------------------------------------------------
// .claude/settings.json — hooks + permissions
// ---------------------------------------------------------------------------

fn settings_path(root: &Path) -> PathBuf {
    root.join(".claude").join("settings.json")
}

fn apply_settings(root: &Path) -> Result<()> {
    let path = settings_path(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut settings: Value = match fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str(&text)
            .map_err(|e| format!("{} is not valid JSON: {e}", path.display()))?,
        _ => json!({}),
    };

    if !settings.is_object() {
        settings = json!({});
    }

    let hooks_changed = ensure_hooks(&mut settings);
    let perms_changed = ensure_permissions(&mut settings);

    if !hooks_changed && !perms_changed {
        println!("  {} settings.json already up to date", "•".dimmed());
        return Ok(());
    }

    fs::write(&path, serde_json::to_string_pretty(&settings)? + "\n")?;
    println!("  {} settings written to {}", "✓".green(), path.display());
    Ok(())
}

/// Arbor's three hook commands (bare `arbor`, matching the reference install).
fn arbor_hook_commands() -> (String, String, String) {
    // PreToolUse #1: auto-init .arbor/ if an arbor command runs before setup.
    let init = "echo \"$CLAUDE_TOOL_INPUT\" | grep -q '\\barbor\\b' && \
[ ! -d .arbor ] && arbor init . >/dev/null 2>&1; exit 0"
        .to_string();
    // PreToolUse #2: block rg, recursive grep, and find -name; steer to arbor.
    let block = "CMD=$(echo \"$CLAUDE_TOOL_INPUT\" | jq -r '.command // .input // .'); \
echo \"$CMD\" | grep -qE '\\brg\\b' && \
echo 'BLOCK: Use arbor query \"symbol\" . instead of rg. \
Multi-term: arbor query \"term1|term2\" . --exclude-test' && exit 1; \
echo \"$CMD\" | grep -qE '\\bgrep\\b' && \
echo \"$CMD\" | grep -qE '(-[a-zA-Z]*r|-[a-zA-Z]*R|--recursive|\\*\\*/|\\.\\.?/)' && \
echo 'BLOCK: Use arbor query/callers/callees instead of recursive grep. \
Grep on a single known file is OK.' && exit 1; \
echo \"$CMD\" | grep -qE 'find\\s+\\..*(-name|-type)' && \
echo 'BLOCK: Use arbor query \"pattern\" . to find files/symbols. \
Use arbor file-graph for file contents.' && exit 1; exit 0"
        .to_string();
    // PostToolUse: inject the project skeleton once per day.
    let map = "FLAG=\".arbor/.map-injected-$(date +%Y%m%d)\"; \
[ -f \"$FLAG\" ] && exit 0; touch \"$FLAG\"; \
echo '--- arbor map (project skeleton) ---'; \
arbor map . --exclude-test 2>/dev/null; echo '--- end arbor map ---'; exit 0"
        .to_string();
    (init, block, map)
}

/// Insert Arbor hooks into the settings tree. Returns true if anything changed.
fn ensure_hooks(settings: &mut Value) -> bool {
    let (init, block, map) = arbor_hook_commands();

    let hooks = settings
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let hooks = hooks.as_object_mut().unwrap();

    let pre = add_bash_hooks(hooks, "PreToolUse", &[init, block]);
    let post = add_bash_hooks(hooks, "PostToolUse", &[map]);
    pre || post
}

/// Ensure each command in `cmds` is registered under `event` for the Bash
/// matcher. Skips commands already present (exact match). Returns true if any
/// command was added.
fn add_bash_hooks(
    hooks: &mut serde_json::Map<String, Value>,
    event: &str,
    cmds: &[String],
) -> bool {
    let entries = hooks.entry(event.to_string()).or_insert_with(|| json!([]));
    if !entries.is_array() {
        *entries = json!([]);
    }
    let entries = entries.as_array_mut().unwrap();

    // Which commands are already installed for this event?
    let existing: Vec<String> = entries
        .iter()
        .flat_map(|group| group.get("hooks").and_then(|h| h.as_array()))
        .flatten()
        .filter_map(|h| h.get("command").and_then(|c| c.as_str()))
        .map(String::from)
        .collect();

    let to_add: Vec<&String> = cmds
        .iter()
        .filter(|c| !existing.iter().any(|e| e == *c))
        .collect();

    if to_add.is_empty() {
        return false;
    }

    // Find (or create) the Bash matcher group and append to its hooks array.
    let group = match entries
        .iter_mut()
        .find(|g| g.get("matcher").and_then(|m| m.as_str()) == Some("Bash"))
    {
        Some(g) => g,
        None => {
            entries.push(json!({ "matcher": "Bash", "hooks": [] }));
            entries.last_mut().unwrap()
        }
    };

    let group_hooks = group.as_object_mut().and_then(|o| {
        o.entry("hooks".to_string())
            .or_insert_with(|| json!([]))
            .as_array_mut()
    });
    let Some(group_hooks) = group_hooks else {
        return false;
    };

    for cmd in to_add {
        group_hooks.push(json!({ "type": "command", "command": cmd }));
    }
    true
}

/// Add the arbor command allow-list under `permissions.allow`. Returns true if
/// any entry was added.
fn ensure_permissions(settings: &mut Value) -> bool {
    let perms = settings
        .as_object_mut()
        .unwrap()
        .entry("permissions")
        .or_insert_with(|| json!({}));
    if !perms.is_object() {
        *perms = json!({});
    }
    let allow = perms
        .as_object_mut()
        .unwrap()
        .entry("allow")
        .or_insert_with(|| json!([]));
    if !allow.is_array() {
        *allow = json!([]);
    }
    let allow = allow.as_array_mut().unwrap();

    let mut changed = false;
    for entry in PERMISSIONS {
        let present = allow.iter().any(|v| v.as_str() == Some(*entry));
        if !present {
            allow.push(json!(entry));
            changed = true;
        }
    }
    changed
}
