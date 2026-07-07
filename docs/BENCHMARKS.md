# Arbor Performance & Token Benchmarks

This document records reproducible performance characteristics of Arbor's graph engine and token reduction when integrated with AI coding agents.

> **v2.4.0** adds a Criterion benchmark suite. Run `cargo bench -p arbor-graph` for machine-measured numbers.

## Measured Benchmarks (Criterion)

Run on developer hardware via:

```bash
cargo bench -p arbor-graph --bench graph_bench
```

| Benchmark | What it measures |
|-----------|------------------|
| `search_symbols_500` | N-gram symbol search on 500-node chain graph |
| `analyze_impact_depth5_200` | BFS blast-radius on 200-node chain |
| `compute_centrality_100` | PageRank (20 iter) on 100-node graph |
| `graph_query_bundle` | Combined search + impact (simulates one MCP agent turn) |

## Expected Performance Claims

| Metric | Arbor | Grep / Ripgrep | Embedding RAG | Sourcegraph (SCIP) |
|--------|-------|----------------|---------------|---------------------|
| **10k LOC Indexing** | **~144ms** | N/A | ~12 seconds | ~3 seconds |
| **Call Path Resolution** | **< 1ms** | Heuristic/Slow | Probabilistic | ~50ms (remote) |
| **Token Usage (Refactor)** | **~400 tokens** | ~2,500 tokens | ~8,000 tokens | ~2,000 tokens |
| **Precision** | **100% Deterministic** | Heuristic | Probabilistic | 100% Deterministic |

## Token Savings Methodology

When an AI agent is asked: *"What functions will be affected if we change `parse_file`?"*

### Standard File-Reading Agent
1. Read function signature (~150 tokens)
2. Grep codebase for references (~1,500 tokens)
3. Read 5 calling files for transitives (~8,000 tokens)
**Total: ~9,650 tokens**

### Arbor Graph-Enabled Agent (MCP `analyze_impact`)
1. Single tool call with node_id (~100 tokens)
2. Deterministic upstream/downstream summary (~300 tokens)
**Total: ~400 tokens (95.8% reduction)**

The `graph_query_bundle` Criterion benchmark measures the CPU cost of the graph path; token savings come from avoiding file reads entirely.

## Indexing Performance

```bash
rm -rf .arbor/
arbor index --no-cache
```

| Codebase size | Expected time |
|---------------|---------------|
| 10k LOC | ~144ms |
| 100k LOC | ~1.2s |
| 500k LOC | ~5.8s |
| Incremental | ~22ms |

## CI Regression Gate

The `benchmarks.yml` workflow runs `cargo bench -p arbor-graph` on every PR to main and fails on >20% regression (when baseline is established).
