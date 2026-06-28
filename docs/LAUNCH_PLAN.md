# Arbor v2.2.3 "Agent Brain" Launch Plan

This document outlines the strategy for launching Arbor v2.2.3 and growing our GitHub presence from ~120 to 1000+ stars by targeting AI agent developers and open-source contributors.

## Strategic Focus: "The Agent's Second Brain"

We position Arbor not just as an AST visualizer or impact checker, but as **the semantic memory layer for coding agents (Claude, Cursor, Cline, Aider)**. 

### Core Value Proposition:
- **90% fewer tokens**: Stop feeding agents full files. Let them query structural graphs.
- **Explainable reasoning**: Deterministic callers, callees, and paths instead of approximate embeddings.
- **PR guard rails**: Prevent AI models from breaking high-centrality hub functions before staging changes.

---

## Launch Timeline

```mermaid
gantt
    title v2.2.3 Launch Calendar
    dateFormat  YYYY-MM-DD
    section Pre-Launch
    Record Demo Video & GIFs   :a1, 2026-06-29, 3d
    Write Show HN Draft        :a2, after a1, 2d
    section Launch Day
    HN Launch                  :b1, 2026-07-04, 1d
    Reddit / Twitter / Discord :b2, after b1, 2d
    section Post-Launch
    Submit to MCP Registries   :c1, after b2, 3d
    Community Building         :c2, after c1, 7d
```

---

## Step-by-Step Execution Guide

### Phase 1: Launch Assets Preparation (Pre-Launch)

1. **High-Quality Walkthrough Media**:
   - Record a 2-minute demo showing Claude Code or Cursor using Arbor tools (`get_blast_radius`, `explain_symbol`) to refactor code in real-time.
   - Convert key segments into high-compression loop GIFs for the README.
2. **Draft the Show HN Post**:
   - Focus on builder-to-builder tone. No marketing jargon.
   - Emphasize local-first, zero telemetry, sub-ms graph traversals.

---

### Phase 2: The Hacker News Post (Launch Day)

**Title Pattern**: `Show HN: Arbor – Graph-native code intelligence for AI agents (Rust)`

**Draft Introduction Comment**:
> Hey HN,
>
> I built Arbor because feeding entire codebases or raw embeddings to AI coding agents is slow, expensive, and leads to hallucinations. 
> 
> Arbor runs locally, indexes 10,000 lines of code in ~140ms using Tree-sitter, and builds a dependency graph (via petgraph). It exposes a Model Context Protocol (MCP) server so tools like Claude Code or Cursor can query exact execution paths, direct/indirect callers, and centrality ranks.
>
> In v2.2.3, we added:
> 1. **Explain Symbol**: Agent-optimized context slices that describe a function's role, callers, and callees.
> 2. **Blast Radius Analysis**: Lets agents predict what will break before they modify a function.
> 3. **HTTP Transport**: Allows remote MCP client connections.
>
> It's written in Rust, and the visualizer is built in Flutter.
>
> I'd love to hear your feedback on the graph schema and agent tool structures!

---

### Phase 3: Developer Communities & Registries (Launch + 2 Days)

1. **MCP Registries**:
   - Submit to the official MCP Directory.
   - Submit PR to `punkpeye/awesome-mcp-servers`.
   - Submit to `tolkonepiu/best-of-mcp-servers`.
2. **Subreddits**:
   - `r/rust`: Focus on petgraph indexing performance and fast local analysis.
   - `r/programming`: Focus on reducing token usage for LLM-based refactoring.
   - `r/LocalLLaMA`: Focus on local agent toolkits.
3. **Discord Communities**:
   - Post in Cursor, Anthropic Developer, and LangChain Discord tool directories.
