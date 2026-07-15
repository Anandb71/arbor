# Arbor v2.5.0 — "The Last Excuse" Release Notes

**Release date:** July 2026
**Theme:** The last excuse for letting your agent grep was "indexing is slow." It's gone.

---

## TL;DR

| Measurement | v2.4.0 | v2.5.0 | Delta |
|---|---|---|---|
| PageRank, 10k-node graph (20 iter) | 149.8ms | **6.6ms** | **23x** |
| Full index, Arbor repo (123 files) | 253ms | **95ms** | **2.7x** |
| Full index, tokio (815 files, 178k LOC) | 2.7s | **1.6s** | **1.7x** |
| Watcher recompute after 1-file patch | full 20 iterations | **converges in ~2 rounds** | warm-start |

Also stress-tested against VS Code (12,406 files, 366k extracted nodes): indexing completes, but at that graph size the single-threaded final assembly dominates end-to-end time. That bottleneck is measured, documented in [BENCHMARKS.md](BENCHMARKS.md), and first on the v2.6.0 list.

Every number is reproducible: `cargo bench -p arbor-graph` for the graph engine, `arbor index . --no-cache` (with `RAYON_NUM_THREADS=1` for the sequential baseline) for indexing. Methodology in [BENCHMARKS.md](BENCHMARKS.md).

---

## Highlights

### Parallel Indexing (rayon)

The cache-check/parse phase now fans out across every core. Tree-sitter parsers are created per task and sled handles concurrent access, so there are no locks in the hot path — and results assemble in walk order, so the graph you get is byte-for-byte identical to the sequential build.

```bash
arbor index . --no-cache          # all cores (default)
RAYON_NUM_THREADS=4 arbor index . # or pin it
```

Small repos gain ~3x; the bigger the repo, the closer you get to core-count scaling, because parse time dominates the serial walk.

### 23x Faster PageRank

`compute_centrality` was spending its life in per-iteration `get_callers()` calls and string-ID hash lookups. It now builds a flat call-graph adjacency once and iterates over dense vectors. Same semantics — Calls edges only, 10% weight for test-file callers, [0,1] normalization — measured against the old implementation running side-by-side in the same Criterion run.

### Warm-Start Centrality

`compute_centrality_warm` seeds iteration from the previous scores instead of a uniform start. Stored scores are max-normalized, so a naive warm start converges no faster — the fix rescales them back to fixed-point scale analytically (one pass over the edges). The sync server's file-change and delete paths now warm-start, so watcher-driven recomputes converge in a couple of rounds instead of burning the full iteration budget.

### Convergence Early-Exit

Centrality iteration stops as soon as no score moves more than 1e-9 between rounds — the iteration budget is now a ceiling, not a sentence.

---

## Why this matters for agents

`arbor map` is the recommended first call for every MCP agent session. Map cost = index cost + rank cost, and both just dropped: sub-2-second cold index on a 178k-LOC codebase, single-digit-millisecond ranking. Cold-starting an agent on a codebase it has never seen is fast enough that there is no latency argument left for `grep -r` as a navigation strategy — and the token argument was never close ([~95% reduction](BENCHMARKS.md#token-savings-methodology)).

---

## Upgrade Guide

No breaking changes. No config changes. Install and everything is faster:

```bash
cargo install arbor-graph-cli --force
```

MCP clients: unchanged. `graph.bin` caches from v2.4.0 load fine and their centrality scores are used to warm-start the first recompute.

---

## Deferred to v2.6.0

- Unified parser pipelines
- Process-level graph daemon
- Incremental PageRank over graph deltas (warm-start covers the common case; true delta-updates remain)
