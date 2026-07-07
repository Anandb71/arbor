use arbor_core::{CodeNode, NodeKind};
use arbor_graph::{compute_centrality, ArborGraph, Edge, EdgeKind};
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn build_chain_graph(n: usize) -> ArborGraph {
    let mut graph = ArborGraph::new();
    let mut prev = None;
    for i in 0..n {
        let name = format!("fn_{}", i);
        let node = CodeNode::new(&name, &name, NodeKind::Function, "src/lib.rs");
        let idx = graph.add_node(node);
        if let Some(p) = prev {
            graph.add_edge(p, idx, Edge::new(EdgeKind::Calls));
        }
        prev = Some(idx);
    }
    graph.rebuild_search_index();
    graph
}

fn bench_search(c: &mut Criterion) {
    let graph = build_chain_graph(500);
    c.bench_function("search_symbols_500", |b| {
        b.iter(|| black_box(graph.search("fn_25")))
    });
}

fn bench_impact(c: &mut Criterion) {
    let graph = build_chain_graph(200);
    let binding = graph.find_by_name("fn_100");
    let mid = binding.first().unwrap();
    let idx = graph.get_index(&mid.id).unwrap();
    c.bench_function("analyze_impact_depth5_200", |b| {
        b.iter(|| black_box(graph.analyze_impact(idx, 5)))
    });
}

fn bench_centrality(c: &mut Criterion) {
    let graph = build_chain_graph(100);
    c.bench_function("compute_centrality_100", |b| {
        b.iter(|| black_box(compute_centrality(&graph, 20, 0.85)))
    });
}

fn bench_token_savings_estimate(c: &mut Criterion) {
    let graph = build_chain_graph(300);
    let binding = graph.find_by_name("fn_150");
    let target = binding.first().unwrap();
    let idx = graph.get_index(&target.id).unwrap();
    c.bench_function("graph_query_bundle", |b| {
        b.iter(|| {
            let _ = graph.search("fn_150");
            black_box(graph.analyze_impact(idx, 3))
        })
    });
}

criterion_group!(
    benches,
    bench_search,
    bench_impact,
    bench_centrality,
    bench_token_savings_estimate
);
criterion_main!(benches);
