# Arbor Performance & Token Benchmarks

This document records reproducible performance characteristics of Arbor's graph engine and token reduction when integrated with AI coding agents.

> **v2.5.0** parallelizes indexing and rewrites PageRank. Every number below is reproducible from this repo — if you think one is wrong, run it and file an issue with your output.

## v2.5.0 Measured Results

Hardware: 16-core Windows 11 dev machine, warm filesystem cache, median of 3 runs. Sequential baseline is the identical binary pinned to one thread (`RAYON_NUM_THREADS=1`), so the comparison isolates the parallel fan-out.

### Real-repo indexing (`arbor index . --no-cache`)

| Repo | Files | Sequential | Parallel (default) | Speedup |
|------|-------|-----------:|-------------------:|--------:|
| Arbor (this repo) | 123 | 253ms | **95ms** | 2.7x |
| tokio (178k LOC) | 815 | 2.7s | **1.6s** | 1.7x |

The parse phase scales with cores; the walk and final graph assembly are serial, which caps the end-to-end gain on parse-light repos (Amdahl's law). Repos with heavier files spend proportionally more time in the parallel phase.

**Known limit:** stress-testing against VS Code (12,406 files → 366k extracted nodes) completes in ~7 minutes (405s measured, cold cache, 16 cores) — at that graph size the single-threaded assembly dominates and parallel parsing can't save it. This is the top optimization target for v2.6.0; we publish the limit rather than hide it.

### PageRank (`cargo bench -p arbor-graph`)

| Benchmark | v2.4.0 | v2.5.0 | Speedup |
|-----------|-------:|-------:|--------:|
| `compute_centrality_10k` (10,200 nodes, 20 iter) | 149.8ms | **6.6ms** | **23x** |

Measured side-by-side in one Criterion run: the v2.4.0 implementation (per-iteration `get_callers()` + string-ID lookups) was kept as a reference function during measurement. The rewrite builds a flat call-adjacency once and iterates dense vectors; semantics are identical (Calls edges only, 10% test-caller weight, [0,1] normalization) and covered by unit tests.

`compute_centrality_10k_warm` additionally measures warm-starting from previous scores — at 10k nodes the adjacency precompute dominates so cold and warm are within noise; warm-start pays off on much larger graphs and in watcher-driven recomputes where the previous scores are already near the fixed point.

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
| `compute_centrality_10k` | PageRank (20 iter) on ~10k-node fan-in graph |
| `compute_centrality_10k_warm` | Same graph, warm-started from previous scores |
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

Prefer the measured real-repo table at the top of this document over extrapolations. Rules of thumb from those measurements (16 cores, warm cache):

| Codebase | Measured |
|----------|----------|
| ~30k LOC (Arbor) | ~95ms |
| ~180k LOC (tokio) | ~1.6s |
| Very large graphs (VS Code-scale, 300k+ nodes) | minutes — graph assembly dominates; optimization tracked for v2.6.0 |
| Incremental (cache hit) | ~22ms |

Indexing cost is driven more by extracted node/edge count than by raw LOC: parse fans out across cores (v2.5.0), while final graph assembly is single-threaded and grows with graph size.

## CI Regression Gate

The `benchmarks.yml` workflow runs `cargo bench -p arbor-graph` on every PR to main and fails on >20% regression (when baseline is established).
