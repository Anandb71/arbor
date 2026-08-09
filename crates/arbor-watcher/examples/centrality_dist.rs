//! Compares the old max-normalized centrality distribution against percentile
//! rank, to calibrate risk thresholds against evidence rather than intuition.
//!
//! Usage: cargo run -p arbor-watcher --example centrality_dist -- <dir>

use arbor_graph::compute_centrality;
use arbor_watcher::{index_directory, IndexOptions};
use std::path::Path;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let result = index_directory(Path::new(&dir), IndexOptions::default()).expect("index");
    let graph = result.graph;

    let scores = compute_centrality(&graph, 20, 0.85);
    let nodes: Vec<_> = graph.node_indexes().collect();
    let n = nodes.len();
    if n == 0 {
        println!("empty graph");
        return;
    }

    // Raw PageRank, then the old max-normalization: score / max.
    let raw: Vec<f64> = nodes.iter().map(|&i| scores.get_raw(i)).collect();
    let max = raw.iter().cloned().fold(0.0f64, f64::max);
    let max_norm: Vec<f64> = raw.iter().map(|r| if max > 0.0 { r / max } else { 0.0 }).collect();
    let pct: Vec<f64> = nodes.iter().map(|&i| scores.get(i)).collect();

    let frac_above = |v: &[f64], t: f64| v.iter().filter(|x| **x > t).count() as f64 / n as f64;

    println!("\n{dir}");
    println!("  nodes {n}");
    println!("\n  threshold   max-normalized   percentile");
    for t in [0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 0.95, 0.98] {
        println!(
            "  > {t:<9.2} {:>12.2}%  {:>10.2}%",
            frac_above(&max_norm, t) * 100.0,
            frac_above(&pct, t) * 100.0
        );
    }

    // What percentile selects the same population the old 0.7 / 0.4 bars did?
    let target_high = frac_above(&max_norm, 0.7);
    let target_med = frac_above(&max_norm, 0.4);
    println!(
        "\n  old HIGH bar (max-norm > 0.70) selected {:.2}% of nodes -> percentile {:.3}",
        target_high * 100.0,
        1.0 - target_high
    );
    println!(
        "  old MED  bar (max-norm > 0.40) selected {:.2}% of nodes -> percentile {:.3}",
        target_med * 100.0,
        1.0 - target_med
    );
}
