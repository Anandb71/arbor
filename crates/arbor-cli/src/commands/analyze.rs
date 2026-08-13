use super::*;
use arbor_graph::{changed_node_ids, compute_blast_radius};
use arbor_mcp::is_git_repo;
use colored::Colorize;
use std::path::Path;

pub fn refactor(
    target: &str,
    max_depth: usize,
    show_why: bool,
    json_output: bool,
    path: &Path,
) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;
    let graph = load_or_index_graph(&resolved_path)?;

    // Find the target node, preferring the most connected definition when the
    // name is ambiguous — picking the first parsed one reported unrelated
    // methods as dead code.
    let node_idx = match resolve_symbol_ranked(&graph, target) {
        Ok((idx, others)) => {
            if !json_output {
                report_symbol_ambiguity(&graph, target, idx, &others);
            }
            Some(idx)
        }
        Err(_) => None,
    };

    let node_idx = match node_idx {
        Some(idx) => idx,
        None => {
            // Smart fallback: suggest similar symbols
            return suggest_similar_symbols(&graph, target);
        }
    };

    // Get the target node info
    let target_node = graph.get(node_idx).unwrap();

    // Run impact analysis
    let analysis = graph.analyze_impact(node_idx, max_depth);

    if json_output {
        // JSON output (keep existing behavior for automation)
        let output = serde_json::json!({
            "target": {
                "id": analysis.target.id,
                "name": analysis.target.name,
                "kind": analysis.target.kind,
                "file": analysis.target.file
            },
            "upstream": analysis.upstream.iter().map(|n| serde_json::json!({
                "id": n.node_info.id,
                "name": n.node_info.name,
                "severity": n.severity.as_str(),
                "hop_distance": n.hop_distance,
                "entry_edge": n.entry_edge.to_string()
            })).collect::<Vec<_>>(),
            "downstream": analysis.downstream.iter().map(|n| serde_json::json!({
                "id": n.node_info.id,
                "name": n.node_info.name,
                "severity": n.severity.as_str(),
                "hop_distance": n.hop_distance,
                "entry_edge": n.entry_edge.to_string()
            })).collect::<Vec<_>>(),
            "total_affected": analysis.total_affected,
            "query_time_ms": analysis.query_time_ms
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    // === WARM, OPINIONATED OUTPUT ===
    println!();
    println!(
        "{} {}",
        "🔍 Analyzing".cyan().bold(),
        target_node.name.cyan().bold()
    );
    println!();

    // Compute and display confidence
    let confidence = arbor_graph::ConfidenceExplanation::from_analysis(&analysis);
    let role = arbor_graph::NodeRole::from_analysis(&analysis);

    let confidence_color = match confidence.level {
        arbor_graph::ConfidenceLevel::High => "green",
        arbor_graph::ConfidenceLevel::Medium => "yellow",
        arbor_graph::ConfidenceLevel::Low => "red",
    };

    println!(
        "{}  {} | {}",
        match confidence.level {
            arbor_graph::ConfidenceLevel::High => "🟢",
            arbor_graph::ConfidenceLevel::Medium => "🟡",
            arbor_graph::ConfidenceLevel::Low => "🔴",
        },
        format!("Confidence: {}", confidence.level).color(confidence_color),
        format!("Role: {}", role).dimmed()
    );

    for reason in &confidence.reasons {
        println!("   • {}", reason.dimmed());
    }
    println!();

    // ========== --why VERBOSE OUTPUT ==========
    if show_why {
        println!("{}", "═══ Detailed Analysis (--why) ═══".cyan().bold());
        println!();

        // 1. Why this confidence level?
        println!("{}", "📊 Why this confidence level?".cyan());
        match confidence.level {
            arbor_graph::ConfidenceLevel::High => {
                println!("   • High caller count indicates well-integrated code");
                println!("   • Clear static call graph with minimal uncertainty");
            }
            arbor_graph::ConfidenceLevel::Medium => {
                println!("   • Moderate caller count or some uncertain edges");
                println!("   • May have dynamic dispatch or callback patterns");
            }
            arbor_graph::ConfidenceLevel::Low => {
                println!("   • Few or no callers detected statically");
                println!("   • May be called via reflection, DI, or externally");
            }
        }
        println!();

        // 2. Check for heuristics fired
        let _all_nodes: Vec<_> = analysis
            .all_affected()
            .iter()
            .map(|a| &a.node_info)
            .collect();
        let all_node_refs: Vec<_> = graph.nodes().take(100).collect(); // Sample for heuristics

        let callbacks: Vec<_> = all_node_refs
            .iter()
            .filter(|n| arbor_graph::HeuristicsMatcher::is_callback_style(n))
            .take(3)
            .collect();
        let event_handlers: Vec<_> = all_node_refs
            .iter()
            .filter(|n| arbor_graph::HeuristicsMatcher::is_event_handler(n))
            .take(3)
            .collect();
        let widgets: Vec<_> = all_node_refs
            .iter()
            .filter(|n| arbor_graph::HeuristicsMatcher::is_flutter_widget(n))
            .take(3)
            .collect();
        let di_nodes: Vec<_> = all_node_refs
            .iter()
            .filter(|n| arbor_graph::HeuristicsMatcher::is_dependency_injection(n))
            .take(3)
            .collect();

        println!("{}", "🔍 Heuristics detected in codebase:".cyan());
        if callbacks.is_empty()
            && event_handlers.is_empty()
            && widgets.is_empty()
            && di_nodes.is_empty()
        {
            println!("   • None detected (clean static analysis)");
        } else {
            if !callbacks.is_empty() {
                println!(
                    "   • {} callback-style nodes (may be invoked dynamically)",
                    callbacks.len()
                );
                for cb in &callbacks {
                    println!("     └─ {}", cb.name.dimmed());
                }
            }
            if !event_handlers.is_empty() {
                println!(
                    "   • {} event handlers (connected at runtime)",
                    event_handlers.len()
                );
                for eh in &event_handlers {
                    println!("     └─ {}", eh.name.dimmed());
                }
            }
            if !widgets.is_empty() {
                println!(
                    "   • {} Flutter widgets (tree determined at runtime)",
                    widgets.len()
                );
            }
            if !di_nodes.is_empty() {
                println!(
                    "   • {} DI/factory patterns (may bypass static calls)",
                    di_nodes.len()
                );
            }
        }
        println!();

        // 3. Why were callers included/excluded?
        println!("{}", "📥 Why callers were included:".cyan());
        if analysis.upstream.is_empty() {
            println!("   • No static callers found in indexed files");
            println!("   • Check: external entry points, tests, or dynamic invocation");
        } else {
            println!(
                "   • {} nodes call this directly or transitively",
                analysis.upstream.len()
            );
            for caller in analysis.upstream.iter().take(3) {
                println!(
                    "     └─ {} via {}",
                    caller.node_info.name,
                    caller.entry_edge.to_string().dimmed()
                );
            }
        }
        println!();

        println!("{}", "📤 Why dependencies were included:".cyan());
        if analysis.downstream.is_empty() {
            println!("   • This is a leaf node (no outgoing calls)");
        } else {
            println!(
                "   • {} nodes are called by this function",
                analysis.downstream.len()
            );
        }
        println!();

        println!("{}", "════════════════════════════════".dimmed());
        println!();
    }

    // Determine the node's role
    let has_upstream = !analysis.upstream.is_empty();
    let has_downstream = !analysis.downstream.is_empty();

    match (has_upstream, has_downstream) {
        (false, false) => {
            // Isolated node
            println!("{}", "This node appears isolated.".yellow());
            println!("  • No callers found in the codebase");
            println!("  • No dependencies detected");
            println!();
            println!("{}", "Possible reasons:".dimmed());
            println!("  • It's an entry point called externally (CLI, HTTP, tests)");
            println!("  • It's dynamically invoked (reflection, callbacks)");
            println!("  • It may be dead code");
            println!();
            println!("{} Safe to change, but verify external usage.", "→".green());
        }
        (false, true) => {
            // Entry point (no callers, but calls others)
            println!("{}", "This is an entry point.".green());
            println!("  Nothing in your codebase calls it directly.");
            println!();
            println!("{}", "However, changing it may affect:".yellow());
            for node in analysis.downstream.iter().take(5) {
                println!(
                    "  └─ {} ({})",
                    node.node_info.name.cyan(),
                    node.entry_edge.to_string().dimmed()
                );
            }
            if analysis.downstream.len() > 5 {
                println!("  └─ ... and {} more", analysis.downstream.len() - 5);
            }
            println!();
            println!(
                "{} Low risk upstream, {} downstream dependencies.",
                "→".green(),
                analysis.downstream.len().to_string().yellow()
            );
        }
        (true, false) => {
            // Leaf/utility node (has callers, but doesn't call anything)
            println!("{}", "This is a utility function.".cyan());
            println!("  Called by others, but doesn't depend on much.");
            println!();
            println!("{}", "Called by:".yellow());
            for node in analysis.upstream.iter().take(5) {
                println!(
                    "  • {} ({} hop{})",
                    node.node_info.name.cyan(),
                    node.hop_distance,
                    if node.hop_distance == 1 { "" } else { "s" }
                );
            }
            if analysis.upstream.len() > 5 {
                println!("  • ... and {} more", analysis.upstream.len() - 5);
            }
            println!();
            println!(
                "{} Changes here ripple up to {} caller{}.",
                "→".yellow(),
                analysis.upstream.len(),
                if analysis.upstream.len() == 1 {
                    ""
                } else {
                    "s"
                }
            );
        }
        (true, true) => {
            // Connected node (has both callers and dependencies)
            println!("{}", "This node sits in the middle of the graph.".cyan());
            println!(
                "  {} caller{}, {} dependenc{}.",
                analysis.upstream.len(),
                if analysis.upstream.len() == 1 {
                    ""
                } else {
                    "s"
                },
                analysis.downstream.len(),
                if analysis.downstream.len() == 1 {
                    "y"
                } else {
                    "ies"
                }
            );
            println!();

            // Count by severity
            let direct: Vec<_> = analysis
                .all_affected()
                .into_iter()
                .filter(|n| n.severity == arbor_graph::ImpactSeverity::Direct)
                .collect();
            let transitive: Vec<_> = analysis
                .all_affected()
                .into_iter()
                .filter(|n| n.severity == arbor_graph::ImpactSeverity::Transitive)
                .collect();

            println!(
                "{} {} nodes affected ({}  direct, {} transitive)",
                "⚠️ ".yellow(),
                analysis.total_affected.to_string().bold(),
                direct.len().to_string().red(),
                transitive.len().to_string().yellow()
            );
            println!();

            if !direct.is_empty() {
                println!("{}", "Will break immediately:".red());
                for node in direct.iter().take(5) {
                    print!("  • {} ({})", node.node_info.name, node.node_info.kind);
                    if show_why {
                        print!(
                            " — {} {}",
                            node.entry_edge.to_string().dimmed(),
                            target_node.name
                        );
                    }
                    println!();
                }
                if direct.len() > 5 {
                    println!("  • ... and {} more", direct.len() - 5);
                }
                println!();
            }

            if !transitive.is_empty() && show_why {
                println!("{}", "May break indirectly:".yellow());
                for node in transitive.iter().take(3) {
                    println!(
                        "  • {} ({} hops away)",
                        node.node_info.name, node.hop_distance
                    );
                }
                if transitive.len() > 3 {
                    println!("  • ... and {} more", transitive.len() - 3);
                }
                println!();
            }

            println!("{} Proceed carefully. Test affected callers.", "→".red());
        }
    }

    println!();
    println!("{}", format!("File: {}", target_node.file).dimmed());

    Ok(())
}

pub fn explain(
    question: &str,
    max_tokens: usize,
    show_why: bool,
    json_output: bool,
    path: &Path,
) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;
    let graph = load_or_index_graph(&resolved_path)?;

    // Try to find a node matching the question (could be a function name)
    let node_idx = graph.get_index(question).or_else(|| {
        graph
            .find_by_name(question)
            .first()
            .and_then(|n| graph.get_index(&n.id))
    });

    let node_idx = match node_idx {
        Some(idx) => idx,
        None => {
            return Err(format!("Node '{}' not found in graph", question).into());
        }
    };

    // Slice context around the node
    let slice = graph.slice_context(node_idx, max_tokens, 2, &[]);

    // Warn if context was truncated
    if slice.truncation_reason != arbor_graph::TruncationReason::Complete {
        eprintln!(
            "\n{} Context truncated: {} (limit: {} tokens)",
            "⚠".yellow(),
            slice.truncation_reason,
            max_tokens
        );
        eprintln!("  Some nodes were excluded to fit token budget.");
        eprintln!("  Use --tokens to increase limit, or use pinning for critical nodes.");
    }

    if json_output {
        let output = serde_json::json!({
            "target": {
                "id": slice.target.id,
                "name": slice.target.name,
                "kind": slice.target.kind,
                "file": slice.target.file
            },
            "context_nodes": slice.nodes.iter().map(|n| serde_json::json!({
                "id": n.node_info.id,
                "name": n.node_info.name,
                "kind": n.node_info.kind,
                "file": n.node_info.file,
                "depth": n.depth,
                "token_estimate": n.token_estimate,
                "pinned": n.pinned
            })).collect::<Vec<_>>(),
            "total_tokens": slice.total_tokens,
            "max_tokens": slice.max_tokens,
            "truncation_reason": slice.truncation_reason.to_string()
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}", "📖 Graph-Backed Context".cyan().bold());
        println!(
            "Target: {} ({})",
            slice.target.name.cyan(),
            slice.target.kind
        );
        println!();

        println!("{}", slice.summary());
        println!();

        if show_why {
            println!("{}", "Path traced:".dimmed());
            for node in slice.nodes.iter().take(10) {
                let pinned_marker = if node.pinned { " [pinned]" } else { "" };
                println!(
                    "  {} {} ({}) — ~{} tokens{}",
                    "→".dimmed(),
                    node.node_info.name,
                    node.node_info.kind,
                    node.token_estimate,
                    pinned_marker.cyan()
                );
            }
            if slice.nodes.len() > 10 {
                println!("  ... and {} more nodes", slice.nodes.len() - 10);
            }
            println!();
        }

        println!(
            "Truncation: {} | Query time: {}ms",
            slice.truncation_reason.to_string().yellow(),
            slice.query_time_ms
        );
    }

    Ok(())
}

/// Perform a security audit to find paths to a sensitive sink.
pub fn audit(sink: &str, depth: usize, format: &str, path: &Path) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;

    // 1. Load the graph
    let graph = load_or_index_graph(&resolved_path)?;
    println!(
        "{} Auditing security paths to sink: {}",
        "🔍".cyan(),
        sink.yellow().bold()
    );

    // 2. Configure audit
    let config = crate::audit::AuditConfig {
        max_depth: depth,
        ignore_tests: true,
    };

    // 3. Run audit
    let start = std::time::Instant::now();
    let result = crate::audit::run_audit(&graph, sink, &config).map_err(|e| e.to_string())?;
    let duration = start.elapsed();

    // 4. Output results
    match format {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }
        "csv" => {
            println!("severity,entry_point,entry_file,path_length,trace");
            for audit_path in &result.paths {
                let trace_str: Vec<&str> =
                    audit_path.trace.iter().map(|n| n.name.as_str()).collect();
                println!(
                    "{},{},{},{},\"{}\"",
                    audit_path.severity.label(),
                    audit_path.source.name,
                    audit_path.source.file,
                    audit_path.trace.len(),
                    trace_str.join(" -> ")
                );
            }
            return Ok(());
        }
        _ => {} // text format below
    }

    // Text output
    println!(
        "\n{} Found {} paths to sink in {:.2?}",
        if result.path_count > 0 {
            "⚠️".yellow()
        } else {
            "✓".green()
        },
        result.path_count,
        duration
    );

    if result.path_count == 0 {
        println!(
            "\nNo public entry points found leading to '{}'.",
            sink.dimmed()
        );
        return Ok(());
    }

    // Summary box
    println!("\n{}", "┌─ Audit Summary ─────────────────────┐".dimmed());
    println!(
        "│  🔴 Critical: {}  🟠 High: {}  🟡 Medium: {}  🟢 Low: {}",
        result.summary.critical_count,
        result.summary.high_count,
        result.summary.medium_count,
        result.summary.low_count
    );
    println!(
        "│  Entry Points: {}  Files Touched: {}",
        result.summary.unique_entry_points, result.summary.unique_files
    );
    println!("{}", "└─────────────────────────────────────┘".dimmed());

    // Detailed paths
    println!("\n{}", "Exploit Paths:".red().bold());
    println!("{}", "═".repeat(50).dimmed());

    for (i, audit_path) in result.paths.iter().take(15).enumerate() {
        println!(
            "\n{} {}. {} → {}",
            audit_path.severity.emoji(),
            i + 1,
            audit_path.source.name.green().bold(),
            sink.red().bold()
        );
        println!(
            "   {} {}  Depth: {}",
            "File:".dimmed(),
            audit_path.source.file.dimmed(),
            audit_path.trace.len()
        );

        println!("   {}", "Trace:".dimmed());
        for (j, step) in audit_path.trace.iter().enumerate() {
            let is_last = j == audit_path.trace.len() - 1;
            let prefix = if is_last { "└─" } else { "├─" };
            let name = if j == 0 {
                step.name.green().to_string()
            } else if is_last {
                step.name.red().to_string()
            } else {
                step.name.white().to_string()
            };
            println!("     {} {}", prefix.dimmed(), name);
        }

        if !audit_path.uncertainty.is_empty() {
            println!(
                "   {} {}",
                "⚠ Heuristic:".yellow(),
                audit_path.uncertainty.join(", ")
            );
        }
    }

    if result.path_count > 15 {
        println!(
            "\n{} ... and {} more paths. Use --format json for full export.",
            "→".dimmed(),
            result.path_count - 15
        );
    }

    // Remediation
    println!("\n{}", "Recommended Actions:".cyan().bold());
    println!(
        "  1. Review direct callers of '{}' for input validation.",
        sink
    );
    println!("  2. Add sanitization at entry points marked CRITICAL/HIGH.");
    println!(
        "  3. Export full report: {} {} --format csv",
        "arbor audit".bold(),
        sink
    );

    Ok(())
}

/// Generate a PR summary for refactored symbols.
pub fn pr_summary(symbols: &str, path: &Path) -> Result<()> {
    println!("{}", "📝 PR Summary Generator".cyan().bold());
    println!();

    // Index the codebase
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;
    let graph = load_or_index_graph(&resolved_path)?;

    let symbol_list: Vec<&str> = symbols.split(',').map(|s| s.trim()).collect();

    println!("## Impact Analysis\n");
    println!("The following symbols were modified:\n");

    for symbol in &symbol_list {
        // Find the node
        let node_idx = graph.get_index(symbol).or_else(|| {
            graph
                .find_by_name(symbol)
                .first()
                .and_then(|n| graph.get_index(&n.id))
        });

        if let Some(idx) = node_idx {
            let node = graph.get(idx).unwrap();
            let analysis = graph.analyze_impact(idx, 3);
            let confidence = arbor_graph::ConfidenceExplanation::from_analysis(&analysis);
            let role = arbor_graph::NodeRole::from_analysis(&analysis);

            println!("### `{}`", node.name);
            println!();
            println!("- **File:** `{}`", node.file);
            println!("- **Role:** {}", role);
            println!("- **Confidence:** {}", confidence.level);
            println!("- **Total Affected:** {} nodes", analysis.total_affected);

            if !analysis.upstream.is_empty() {
                println!("\n**Callers that may be affected:**");
                for caller in analysis.upstream.iter().take(5) {
                    println!("- `{}`", caller.node_info.name);
                }
            }
            println!();
        } else {
            println!("### `{}` (not found in graph)\n", symbol);
        }
    }

    println!("---");
    println!("*Generated by Arbor*");

    Ok(())
}

/// Generate an auto-description for a PR based on graph changes.
pub fn summary(path: &Path) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;

    if !is_git_repo(&resolved_path) {
        return Err("arbor summary requires a git repository".into());
    }

    let changed_files = git_changed_files(&resolved_path)?;
    if changed_files.is_empty() {
        println!("## 🌳 Arbor PR Summary\n");
        println!("No changes detected in git. Working tree is clean.");
        return Ok(());
    }

    let graph = load_or_index_graph(&resolved_path)?;
    let changed_nodes = changed_node_ids(&graph, &changed_files, &resolved_path);

    // Reuse our depth=5 summary computation
    let summary = compute_blast_radius(
        &graph,
        changed_files.clone(),
        changed_nodes,
        5,
        &resolved_path,
    );

    // Classify changes
    let mut code_changes = 0;
    let mut test_changes = 0;
    let mut doc_changes = 0;
    let mut infra_changes = 0;

    for f in &changed_files {
        let fl = f.to_lowercase();
        if fl.contains("test") || fl.contains("spec") {
            test_changes += 1;
        } else if fl.ends_with(".md") || fl.contains("docs/") {
            doc_changes += 1;
        } else if fl.contains("cargo.toml") || fl.contains("dockerfile") || fl.contains(".github/")
        {
            infra_changes += 1;
        } else {
            code_changes += 1;
        }
    }

    let primary_type = if code_changes > 0 {
        "Code/Feature Implementation"
    } else if test_changes > 0 {
        "Testing & Coverage Updates"
    } else if doc_changes > 0 {
        "Documentation Improvements"
    } else if infra_changes > 0 {
        "Infrastructure & Dependency Updates"
    } else {
        "General Maintenance"
    };

    println!("## 🌳 Arbor PR Auto-Description\n");
    println!("### 📝 Overview");
    println!(
        "This PR introduces changes primarily categorized as **{}** across {} modified file(s).\n",
        primary_type,
        changed_files.len()
    );

    println!("### 🔍 Scope of Changes");
    println!("| File | Focus Area |");
    println!("|------|------------|");
    for f in &summary.changed_files {
        let focus = if f.contains("crates/arbor-cli") {
            "CLI Interface"
        } else if f.contains("crates/arbor-core") {
            "Core Intelligence Layer"
        } else if f.contains("crates/arbor-graph") {
            "Graph Modeling Engine"
        } else if f.contains("crates/arbor-server") {
            "LSP/Server Host"
        } else if f.contains("crates/arbor-mcp") {
            "Model Context Protocol integration"
        } else if f.contains(".github/") {
            "CI Workflows"
        } else {
            "General Codebase"
        };
        println!("| `{}` | {} |", f, focus);
    }
    println!();

    if let Some(ref diagram) = summary.mermaid_diagram {
        println!("### 📊 Visual Impact Graph\n");
        println!("```mermaid");
        println!("{}", diagram);
        println!("```\n");
    }

    println!("### ⚡ Impact & Blast Radius");
    println!("Our graph analysis resolved **{}** specific symbol changes with the following downstream impact:", summary.changed_symbols);
    println!(
        "- **Direct Callers Affected:** {} callers will need direct integration review.",
        summary.direct_callers
    );
    println!("- **Indirect Callers Affected:** {} secondary callers are in the downstream dependency path.", summary.indirect_callers);
    println!(
        "- **API Entrypoints Affected:** {} public-facing entrypoints are impacted.",
        summary.entrypoints_affected
    );
    println!(
        "- **Total Blast Radius:** {} nodes total in the impact graph.",
        summary.blast_radius_nodes
    );
    println!();

    // Risk classification
    let risk_emoji = if summary.blast_radius_nodes > 50 {
        "🔴 Critical Impact risk"
    } else if summary.blast_radius_nodes > 25 {
        "🟠 High Impact risk"
    } else if summary.blast_radius_nodes > 10 {
        "🟡 Medium Impact risk"
    } else {
        "🟢 Low Impact risk"
    };
    println!("**Risk Classification:** {}\n", risk_emoji);

    // Suggested reviewers based on files modified
    println!("### 👥 Suggested Reviewers");
    let mut reviewers = std::collections::HashSet::new();
    for f in &changed_files {
        if f.contains("crates/arbor-core") || f.contains("crates/arbor-graph") {
            reviewers.insert("@Anandb71 (Core Engine)");
        }
        if f.contains("crates/arbor-cli") || f.contains("crates/arbor-mcp") {
            reviewers.insert("@Anandb71 (CLI / MCP)");
        }
        if f.contains(".github/") || f.contains("Cargo.toml") {
            reviewers.insert("@Anandb71 (DevOps / Build)");
        }
    }
    if reviewers.is_empty() {
        reviewers.insert("@Anandb71 (Maintainer)");
    }
    for r in reviewers {
        println!("- {}", r);
    }
    println!();

    // Verification Checklist
    println!("### ✅ Recommended Verification Checklist");
    println!("- [ ] Execute `cargo test --workspace` to ensure all 185+ unit and integration tests pass.");
    if summary.blast_radius_nodes > 0 {
        println!(
            "- [ ] Manually verify the blast radius of {} impacted nodes.",
            summary.blast_radius_nodes
        );
    }
    if summary.entrypoints_affected > 0 {
        println!(
            "- [ ] Run end-to-end integration tests for the {} affected API entrypoint(s).",
            summary.entrypoints_affected
        );
    }
    println!("- [ ] Run `cargo clippy --workspace --all-targets` to catch any lint warnings.");
    println!();

    println!("---");
    println!("*Generated automatically by [Arbor](https://github.com/Anandb71/arbor) v{} — Graph-Native Code Intelligence*", env!("CARGO_PKG_VERSION"));

    Ok(())
}
