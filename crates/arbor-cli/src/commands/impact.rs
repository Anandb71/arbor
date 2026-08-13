use super::*;
use arbor_graph::{changed_node_ids, compute_blast_radius, BlastRadiusSummary};
use arbor_mcp::is_git_repo;
use colored::Colorize;
use std::path::Path;

fn print_diff_summary(summary: &BlastRadiusSummary) {
    println!("{}", "Change Impact Preview".cyan().bold());
    println!();
    println!("Modified files:");
    for f in &summary.changed_files {
        println!("  • {}", f);
    }
    println!();
    println!("Impact:");
    println!("  • {} direct callers", summary.direct_callers);
    println!("  • {} indirect callers", summary.indirect_callers);
    println!(
        "  • {} API entrypoints affected",
        summary.entrypoints_affected
    );
    println!(
        "  • {} files likely require updates",
        summary.files_likely_updates
    );
    println!("  • {} impacted nodes total", summary.blast_radius_nodes);
    println!("  • {} changed symbols resolved", summary.changed_symbols);
}

fn print_diff_markdown(summary: &BlastRadiusSummary) {
    let risk = if summary.blast_radius_nodes > 50 {
        ("🔴", "Critical")
    } else if summary.blast_radius_nodes > 25 {
        ("🟠", "High")
    } else if summary.blast_radius_nodes > 10 {
        ("🟡", "Medium")
    } else {
        ("🟢", "Low")
    };

    println!("## 🌳 Arbor Impact Report\n");
    println!(
        "**Risk Level:** {} {} | **Blast Radius:** {} nodes | **Changed Symbols:** {}\n",
        risk.0, risk.1, summary.blast_radius_nodes, summary.changed_symbols
    );

    // Changed files table
    println!("### Changed Files\n");
    println!("| File | Status |");
    println!("|------|--------|");
    for f in &summary.changed_files {
        println!("| `{}` | Modified |", f);
    }

    if let Some(ref diagram) = summary.mermaid_diagram {
        println!("\n### 📊 Visual Impact Graph\n");
        println!("```mermaid");
        println!("{}", diagram);
        println!("```");
    }

    // Impact summary
    println!("\n### Impact Summary\n");
    println!("| Metric | Count |");
    println!("|--------|-------|");
    println!("| Direct callers affected | {} |", summary.direct_callers);
    println!(
        "| Indirect callers affected | {} |",
        summary.indirect_callers
    );
    println!(
        "| API entrypoints impacted | {} |",
        summary.entrypoints_affected
    );
    println!(
        "| Files likely requiring updates | {} |",
        summary.files_likely_updates
    );
    println!("| Total blast radius | {} |", summary.blast_radius_nodes);

    // Recommendations
    if summary.entrypoints_affected > 0 {
        println!(
            "\n> ⚠️ **Warning:** {} API entrypoints are affected. Integration tests recommended.",
            summary.entrypoints_affected
        );
    }
    if summary.blast_radius_nodes > 25 {
        println!("\n> 🔍 **Suggestion:** Consider breaking this change into smaller PRs.");
    }

    println!("\n---");
    println!("*Powered by [Arbor](https://github.com/Anandb71/arbor) v{} — graph-native code intelligence*", env!("CARGO_PKG_VERSION"));
}

fn print_check_markdown(summary: &BlastRadiusSummary, risky: bool, max_blast_radius: usize) {
    let status = if risky {
        ("🔴", "FAIL", "High-risk change detected")
    } else {
        ("🟢", "PASS", "Change is within safe thresholds")
    };

    println!("## 🌳 Arbor Safety Check\n");
    println!("**Status:** {} **{}** — {}\n", status.0, status.1, status.2);
    println!(
        "**Threshold:** max blast radius = {} | **Actual:** {}\n",
        max_blast_radius, summary.blast_radius_nodes
    );

    // Changed files
    println!("### Changed Files\n");
    println!("| File | Status |");
    println!("|------|--------|");
    for f in &summary.changed_files {
        println!("| `{}` | Modified |", f);
    }

    if let Some(ref diagram) = summary.mermaid_diagram {
        println!("\n### 📊 Visual Impact Graph\n");
        println!("```mermaid");
        println!("{}", diagram);
        println!("```");
    }

    // Impact table
    println!("\n### Impact Summary\n");
    println!("| Metric | Count | Status |");
    println!("|--------|-------|--------|");
    let br_status = if summary.blast_radius_nodes > max_blast_radius {
        "🔴"
    } else {
        "🟢"
    };
    let ep_status = if summary.entrypoints_affected > 0 {
        "🟡"
    } else {
        "🟢"
    };
    println!(
        "| Blast radius | {} | {} |",
        summary.blast_radius_nodes, br_status
    );
    println!("| Direct callers | {} | |", summary.direct_callers);
    println!("| Indirect callers | {} | |", summary.indirect_callers);
    println!(
        "| API entrypoints | {} | {} |",
        summary.entrypoints_affected, ep_status
    );
    println!(
        "| Files needing updates | {} | |",
        summary.files_likely_updates
    );

    if risky {
        println!("\n> 🚨 **Action Required:** This PR exceeds the blast radius threshold. Review the impact carefully before merging.");
    }

    println!("\n---");
    println!("*Powered by [Arbor](https://github.com/Anandb71/arbor) v{} — graph-native code intelligence*", env!("CARGO_PKG_VERSION"));
}

pub fn diff(path: &Path, depth: usize, json_output: bool, markdown: bool) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;

    if !is_git_repo(&resolved_path) {
        return Err("arbor diff requires a git repository".into());
    }

    let changed_files = git_changed_files(&resolved_path)?;
    if changed_files.is_empty() {
        println!("{} No modified files detected against HEAD.", "✓".green());
        return Ok(());
    }

    let graph = load_or_index_graph(&resolved_path)?;
    let changed_nodes = changed_node_ids(&graph, &changed_files, &resolved_path);
    let summary = compute_blast_radius(&graph, changed_files, changed_nodes, depth, &resolved_path);

    if markdown {
        print_diff_markdown(&summary);
        return Ok(());
    }

    if json_output {
        let output = serde_json::json!({
            "changed_files": summary.changed_files,
            "changed_symbols": summary.changed_symbols,
            "impact": {
                "direct_callers": summary.direct_callers,
                "indirect_callers": summary.indirect_callers,
                "api_entrypoints_affected": summary.entrypoints_affected,
                "files_likely_require_updates": summary.files_likely_updates,
                "blast_radius_nodes": summary.blast_radius_nodes
            }
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    print_diff_summary(&summary);
    Ok(())
}

pub fn check(
    path: &Path,
    depth: usize,
    max_blast_radius: usize,
    no_fail: bool,
    json_output: bool,
    markdown: bool,
) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;

    if !is_git_repo(&resolved_path) {
        return Err("arbor check requires a git repository".into());
    }

    let changed_files = git_changed_files(&resolved_path)?;
    let graph = load_or_index_graph(&resolved_path)?;
    let changed_nodes = changed_node_ids(&graph, &changed_files, &resolved_path);
    let summary = compute_blast_radius(&graph, changed_files, changed_nodes, depth, &resolved_path);

    let risky = summary.blast_radius_nodes > max_blast_radius
        || summary.entrypoints_affected > 0
        || summary.indirect_callers > max_blast_radius / 2;

    if markdown {
        print_check_markdown(&summary, risky, max_blast_radius);
        if risky && !no_fail {
            return Err("risky change set detected".into());
        }
        return Ok(());
    }

    if json_output {
        let output = serde_json::json!({
            "risky": risky,
            "thresholds": {
                "max_blast_radius": max_blast_radius
            },
            "summary": {
                "changed_files": summary.changed_files,
                "changed_symbols": summary.changed_symbols,
                "direct_callers": summary.direct_callers,
                "indirect_callers": summary.indirect_callers,
                "api_entrypoints_affected": summary.entrypoints_affected,
                "files_likely_require_updates": summary.files_likely_updates,
                "blast_radius_nodes": summary.blast_radius_nodes
            }
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else if risky {
        println!("{}", "High risk refactor detected.".red().bold());
        println!();
        print_diff_summary(&summary);
        println!();
        println!("Recommendation: run integration tests.");
    } else {
        println!("{}", "Safe change window detected.".green().bold());
        print_diff_summary(&summary);
    }

    if risky && !no_fail {
        return Err("risky change set detected".into());
    }

    Ok(())
}
