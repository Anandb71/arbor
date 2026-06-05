# CLAUDE.md

## Code Navigation (MANDATORY)

**When scanning any new project for code, first check for a `.arbor/graph.json` file at the project root.** If it exists, the project is indexed by Arbor and you MUST use arbor exclusively for code navigation in that project — do not fall back to grep, rg, find, or reading files to explore.

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
