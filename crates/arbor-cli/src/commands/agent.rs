use super::*;
use arbor_graph::{changed_node_ids, compute_blast_radius};
use arbor_mcp::is_git_repo;
use colored::Colorize;
use std::path::Path;

pub fn agent_review(path: &Path, json: bool) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;

    if !is_git_repo(&resolved_path) {
        return Err("arbor agent review requires a git repository".into());
    }

    let changed_files = git_changed_files(&resolved_path)?;
    if changed_files.is_empty() {
        if json {
            println!("{{}}");
        } else {
            println!("{} No modified files detected against HEAD.", "✓".green());
        }
        return Ok(());
    }

    let graph = load_or_index_graph(&resolved_path)?;
    let changed_nodes = changed_node_ids(&graph, &changed_files, &resolved_path);
    let summary = compute_blast_radius(
        &graph,
        changed_files.clone(),
        changed_nodes.clone(),
        5,
        &resolved_path,
    );

    let mut high_risk_changes = Vec::new();
    let mut recommendations = Vec::new();

    for node_id in changed_nodes {
        if let Some(node) = graph.get(node_id) {
            let centrality = graph.centrality(node_id);
            let callers = graph.get_callers(node_id);
            let is_entry = arbor_graph::HeuristicsMatcher::is_likely_entry_point(node);

            let risk = if centrality > 0.3 || is_entry || callers.len() > 10 {
                "🔴 High"
            } else if centrality > 0.1 || callers.len() > 3 {
                "🟡 Medium"
            } else {
                "🟢 Low"
            };

            if risk == "🔴 High" || risk == "🟡 Medium" {
                high_risk_changes.push(serde_json::json!({
                    "symbol": node.name.clone(),
                    "file": node.file.clone(),
                    "centrality": centrality,
                    "caller_count": callers.len(),
                    "risk": risk
                }));

                if is_entry {
                    recommendations.push(format!("⚠️ `{}` in `{}` is a public entry point. Ensure input validation is robust.", node.name, node.file));
                } else if centrality > 0.3 {
                    recommendations.push(format!("⚠️ `{}` in `{}` is a highly connected hotspot (centrality {:.2}). Run all integration tests.", node.name, node.file, centrality));
                } else {
                    recommendations.push(format!(
                        "ℹ️ `{}` in `{}` has {} callers. Check for call-site breakages.",
                        node.name,
                        node.file,
                        callers.len()
                    ));
                }
            }
        }
    }

    let risk_level = if high_risk_changes.iter().any(|c| c["risk"] == "🔴 High") {
        "🔴 High"
    } else if !high_risk_changes.is_empty() {
        "🟡 Medium"
    } else {
        "🟢 Low"
    };

    if json {
        let output = serde_json::json!({
            "risk_level": risk_level,
            "changed_symbols": summary.changed_symbols,
            "blast_radius": summary.blast_radius_nodes,
            "high_risk_changes": high_risk_changes,
            "recommendations": recommendations
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("# 🌳 Arbor Agent: PR Review Report\n");
        println!("## Risk Summary");
        println!("- **Risk Level**: {}", risk_level);
        println!("- **Changed Symbols**: {}", summary.changed_symbols);
        println!(
            "- **Blast Radius**: {} nodes affected\n",
            summary.blast_radius_nodes
        );

        if !high_risk_changes.is_empty() {
            println!("## High-Risk Changes");
            println!("| Symbol | File | Centrality | Callers | Risk |");
            println!("|--------|------|------------|---------|------|");
            for c in &high_risk_changes {
                println!(
                    "| `{}` | `{}` | {:.2} | {} | {} |",
                    c["symbol"].as_str().unwrap(),
                    c["file"].as_str().unwrap(),
                    c["centrality"].as_f64().unwrap(),
                    c["caller_count"].as_u64().unwrap(),
                    c["risk"].as_str().unwrap()
                );
            }
            println!();
        }

        if !recommendations.is_empty() {
            println!("## Recommendations");
            for r in &recommendations {
                println!("- {}", r);
            }
        } else {
            println!("## Recommendations");
            println!("- ✅ No high-risk or architectural anomalies detected. Safe to merge.");
        }
    }

    Ok(())
}

pub fn agent_onboard(path: &Path, json: bool) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;
    let graph = load_or_index_graph(&resolved_path)?;

    let node_count = graph.node_count();
    let edge_count = graph.edge_count();

    let mut extensions = std::collections::HashSet::new();
    for node_idx in graph.node_indexes() {
        if let Some(node) = graph.get(node_idx) {
            let path = std::path::Path::new(&node.file);
            if let Some(ext) = path.extension() {
                if let Some(ext_str) = ext.to_str() {
                    extensions.insert(ext_str.to_string());
                }
            }
        }
    }
    let languages: Vec<String> = extensions.into_iter().collect();

    let mut entry_points = graph.list_entry_points();
    entry_points.sort_by(|a, b| a.name.cmp(&b.name));

    let mut nodes_with_centrality = Vec::new();
    for node_idx in graph.node_indexes() {
        if let Some(node) = graph.get(node_idx) {
            let centrality = graph.centrality(node_idx);
            nodes_with_centrality.push((node, centrality));
        }
    }
    nodes_with_centrality
        .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    if json {
        let hotspots_json: Vec<serde_json::Value> = nodes_with_centrality
            .iter()
            .take(20)
            .map(|(node, centrality)| {
                serde_json::json!({
                    "symbol": node.name.clone(),
                    "centrality": centrality,
                    "file": node.file.clone()
                })
            })
            .collect();

        let entries_json: Vec<serde_json::Value> = entry_points
            .iter()
            .take(20)
            .map(|node| {
                serde_json::json!({
                    "symbol": node.name.clone(),
                    "kind": node.kind.to_string(),
                    "file": node.file.clone()
                })
            })
            .collect();

        let output = serde_json::json!({
            "total_symbols": node_count,
            "total_connections": edge_count,
            "languages": languages,
            "entry_points": entries_json,
            "hotspots": hotspots_json
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("# 🌳 Arbor Agent: Codebase Guide\n");
        println!("## Architecture Overview");
        println!("- **Total Symbols**: {}", node_count);
        println!("- **Total Connections**: {}", edge_count);
        println!("- **Languages Detected**: {}", languages.join(", "));
        println!();

        println!("## Entry Points (Start Here)");
        println!("| Name | Type | File |");
        println!("|------|------|------|");
        for ep in entry_points.iter().take(10) {
            println!("| `{}` | {} | `{}` |", ep.name, ep.kind, ep.file);
        }
        println!();

        println!("## Core Components (Hotspots)");
        println!("| Rank | Symbol | Centrality | File |");
        println!("|------|--------|------------|------|");
        for (i, (node, centrality)) in nodes_with_centrality.iter().take(15).enumerate() {
            println!(
                "| {} | `{}` | {:.4} | `{}` |",
                i + 1,
                node.name,
                centrality,
                node.file
            );
        }
        println!();

        println!("## Suggested Reading Order");
        println!("1. Start with **Entry Points** listed above to understand execution flow.");
        println!("2. Reference **Core Components** to study the system's shared utility hubs.");
        println!("3. Dive into directory sub-modules as needed for feature development.");
    }

    Ok(())
}

pub fn agent_guard(path: &Path, max_blast_radius: usize) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;

    if !is_git_repo(&resolved_path) {
        return Err("arbor agent guard requires a git repository".into());
    }

    let changed_files = git_changed_files(&resolved_path)?;
    if changed_files.is_empty() {
        println!(
            "{} No changes detected. Architecture guard PASS.",
            "✓".green()
        );
        return Ok(());
    }

    let graph = load_or_index_graph(&resolved_path)?;
    let changed_nodes = changed_node_ids(&graph, &changed_files, &resolved_path);
    let summary = compute_blast_radius(
        &graph,
        changed_files.clone(),
        changed_nodes.clone(),
        5,
        &resolved_path,
    );

    let mut failed = false;
    let mut checks = Vec::new();

    if summary.blast_radius_nodes > max_blast_radius {
        checks.push(format!(
            "❌ Blast radius of {} nodes exceeds limit of {}",
            summary.blast_radius_nodes, max_blast_radius
        ));
        failed = true;
    } else {
        checks.push(format!(
            "✅ Blast radius within limit ({} / {})",
            summary.blast_radius_nodes, max_blast_radius
        ));
    }

    let mut changed_entries = Vec::new();
    for node_id in &changed_nodes {
        if let Some(node) = graph.get(*node_id) {
            if arbor_graph::HeuristicsMatcher::is_likely_entry_point(node) {
                changed_entries.push(node.name.clone());
            }
        }
    }

    if !changed_entries.is_empty() {
        checks.push(format!(
            "❌ Modified public entry point(s): {}",
            changed_entries.join(", ")
        ));
        failed = true;
    } else {
        checks.push("✅ No public entry points modified".to_string());
    }

    let mut changed_hubs = Vec::new();
    for node_id in &changed_nodes {
        if graph.centrality(*node_id) > 0.4 {
            if let Some(node) = graph.get(*node_id) {
                changed_hubs.push(node.name.clone());
            }
        }
    }

    if !changed_hubs.is_empty() {
        checks.push(format!(
            "❌ Modified high-centrality hub(s): {}",
            changed_hubs.join(", ")
        ));
        failed = true;
    } else {
        checks.push("✅ No high-centrality hubs modified".to_string());
    }

    println!("# 🌳 Arbor Agent: Architecture Guard\n");
    if failed {
        println!("## Status: ❌ FAIL\n");
    } else {
        println!("## Status: ✅ PASS\n");
    }

    println!("## Checks");
    for check in checks {
        println!("- {}", check);
    }

    if failed {
        return Err("Architecture guard failed".into());
    }

    Ok(())
}
