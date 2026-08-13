use super::*;
use arbor_graph::{
    compress_path, compute_centrality, is_minified_or_generated, is_test_file, make_relative,
    shorten_signature, HeuristicsMatcher,
};
use arbor_mcp::is_git_repo;
use colored::Colorize;
use std::path::Path;

pub fn query(query: &str, limit: usize, path: &Path, exclude_test: bool) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;
    let graph = load_or_index_graph(&resolved_path)?;

    let terms: Vec<&str> = query
        .split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    let mut seen_ids = std::collections::HashSet::new();
    let mut matches = Vec::new();

    for term in &terms {
        for node in graph.search(term) {
            if exclude_test && is_test_file(&node.file) {
                continue;
            }
            if seen_ids.insert(&node.id) {
                matches.push(node);
            }
            if matches.len() >= limit {
                break;
            }
        }
        if matches.len() >= limit {
            break;
        }
    }

    if matches.is_empty() {
        if exclude_test {
            println!("No matches found for \"{}\" (excluding test files)", query);
        } else {
            println!("No matches found for \"{}\"", query);
        }
        return Ok(());
    }

    println!("Found {} matches:\n", matches.len());

    for node in matches {
        println!(
            "  {} {} {}",
            node.kind.to_string().yellow(),
            node.qualified_name.cyan(),
            format!("({}:{})", node.file, node.line_start).dimmed()
        );
        if let Some(ref sig) = node.signature {
            println!("    {}", sig.dimmed());
        }
    }

    Ok(())
}

pub fn callers(symbol: &str, path: &Path, json_output: bool) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let graph = load_or_index_graph(&resolved_path)?;

    let (idx, ambiguous_with) = resolve_symbol_ranked(&graph, symbol)?;
    if !json_output {
        report_symbol_ambiguity(&graph, symbol, idx, &ambiguous_with);
    }
    let callers = graph.get_callers(idx);

    if json_output {
        let items: Vec<serde_json::Value> = callers
            .iter()
            .map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "name": n.name,
                    "kind": n.kind.to_string(),
                    "file": n.file,
                    "line": n.line_start
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "symbol": symbol,
                "callers": items
            }))?
        );
    } else if callers.is_empty() {
        println!("No callers found for '{}'", symbol);
    } else {
        println!("Callers of '{}' ({}):\n", symbol, callers.len());
        for n in &callers {
            println!(
                "  {} {} {}",
                n.kind.to_string().yellow(),
                n.qualified_name.cyan(),
                format!("({}:{})", n.file, n.line_start).dimmed()
            );
        }
    }

    Ok(())
}

pub fn callees(symbol: &str, path: &Path, json_output: bool) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let graph = load_or_index_graph(&resolved_path)?;

    let (idx, ambiguous_with) = resolve_symbol_ranked(&graph, symbol)?;
    if !json_output {
        report_symbol_ambiguity(&graph, symbol, idx, &ambiguous_with);
    }
    let callees = graph.get_callees(idx);

    if json_output {
        let items: Vec<serde_json::Value> = callees
            .iter()
            .map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "name": n.name,
                    "kind": n.kind.to_string(),
                    "file": n.file,
                    "line": n.line_start
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "symbol": symbol,
                "callees": items
            }))?
        );
    } else if callees.is_empty() {
        println!("No callees found for '{}'", symbol);
    } else {
        println!("Callees of '{}' ({}):\n", symbol, callees.len());
        for n in &callees {
            println!(
                "  {} {} {}",
                n.kind.to_string().yellow(),
                n.qualified_name.cyan(),
                format!("({}:{})", n.file, n.line_start).dimmed()
            );
        }
    }

    Ok(())
}

pub fn entry_points(path: &Path, json_output: bool) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let graph = load_or_index_graph(&resolved_path)?;

    let eps = graph.list_entry_points();

    if json_output {
        let items: Vec<serde_json::Value> = eps
            .iter()
            .map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "name": n.name,
                    "kind": n.kind.to_string(),
                    "file": n.file,
                    "line": n.line_start
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "entry_points": items
            }))?
        );
    } else if eps.is_empty() {
        println!("No entry points detected.");
    } else {
        println!("Entry points ({}):\n", eps.len());
        for n in &eps {
            println!(
                "  {} {} {}",
                n.kind.to_string().yellow(),
                n.qualified_name.cyan(),
                format!("({}:{})", n.file, n.line_start).dimmed()
            );
        }
    }

    Ok(())
}

pub fn file_graph(file: &str, path: &Path, json_output: bool) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let graph = load_or_index_graph(&resolved_path)?;

    let candidates = [
        file.to_string(),
        file.replace('\\', "/"),
        resolved_path.join(file).to_string_lossy().to_string(),
        resolved_path
            .join(file.replace('\\', "/"))
            .to_string_lossy()
            .to_string(),
    ];

    for candidate in &candidates {
        let (nodes, edges) = graph.nodes_in_file_with_edges(candidate);
        if !nodes.is_empty() {
            return print_file_graph_output(candidate, &nodes, &edges, json_output);
        }
    }

    Err(format!("No symbols found in file '{}'", file).into())
}

fn print_file_graph_output(
    file: &str,
    nodes: &[&arbor_core::CodeNode],
    edges: &[(String, String, String)],
    json_output: bool,
) -> Result<()> {
    if json_output {
        let node_items: Vec<serde_json::Value> = nodes
            .iter()
            .map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "name": n.name,
                    "kind": n.kind.to_string(),
                    "line": n.line_start
                })
            })
            .collect();
        let edge_items: Vec<serde_json::Value> = edges
            .iter()
            .map(|(from, to, kind)| {
                serde_json::json!({
                    "from": from,
                    "to": to,
                    "kind": kind
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "file": file,
                "nodes": node_items,
                "edges": edge_items
            }))?
        );
    } else {
        println!("Symbols in '{}' ({}):\n", file, nodes.len());
        for n in nodes {
            println!(
                "  {} {} {}",
                n.kind.to_string().yellow(),
                n.qualified_name.cyan(),
                format!("(L{}–{})", n.line_start, n.line_end).dimmed()
            );
        }
        if !edges.is_empty() {
            println!("\nInternal edges ({}):\n", edges.len());
            for (from, to, kind) in edges {
                println!(
                    "  {} {} {}",
                    from.cyan(),
                    "→".dimmed(),
                    format!("{} ({})", to, kind).dimmed()
                );
            }
        }
    }

    Ok(())
}

pub fn inspect(symbol: &str, path: &Path, json_output: bool) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let graph = load_or_index_graph(&resolved_path)?;

    let (idx, ambiguous_with) = resolve_symbol_ranked(&graph, symbol)?;
    if !json_output {
        report_symbol_ambiguity(&graph, symbol, idx, &ambiguous_with);
    }
    let node = graph
        .get(idx)
        .ok_or_else(|| format!("Node index invalid for '{}'", symbol))?;
    let centrality = graph.centrality(idx);
    let callers = graph.get_callers(idx);
    let callees = graph.get_callees(idx);
    let is_entry = HeuristicsMatcher::is_likely_entry_point(node);
    let role = if is_entry {
        "entry_point"
    } else if callers.is_empty() {
        "unreachable"
    } else if callees.is_empty() {
        "utility"
    } else {
        "internal"
    };

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "id": node.id,
                "name": node.name,
                "kind": node.kind.to_string(),
                "file": node.file,
                "line_start": node.line_start,
                "line_end": node.line_end,
                "signature": node.signature,
                "centrality": centrality,
                "role": role,
                "caller_count": callers.len(),
                "callee_count": callees.len(),
                "is_entry_point": is_entry
            }))?
        );
    } else {
        println!("  {}:    {}", "Name".bold(), node.name);
        println!("  {}:    {}", "Kind".bold(), node.kind.to_string().yellow());
        println!(
            "  {}:    {}:{}-{}",
            "File".bold(),
            node.file,
            node.line_start,
            node.line_end
        );
        if let Some(ref sig) = node.signature {
            println!("  {}:     {}", "Sig".bold(), sig.dimmed());
        }
        println!("  {}:    {}", "Role".bold(), role.cyan());
        println!("  {}:     {:.4}", "Rank".bold(), centrality);
        println!("  {}:  {}", "Callers".bold(), callers.len());
        println!("  {}:  {}", "Callees".bold(), callees.len());
    }

    Ok(())
}

pub fn find_path_cmd(start: &str, end: &str, path: &Path, json_output: bool) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let graph = load_or_index_graph(&resolved_path)?;

    let start_idx = resolve_symbol(&graph, start)?;
    let end_idx = resolve_symbol(&graph, end)?;

    let found = graph.find_path(start_idx, end_idx);

    if json_output {
        match &found {
            Some(nodes) => {
                let items: Vec<serde_json::Value> = nodes
                    .iter()
                    .map(|n| {
                        serde_json::json!({
                            "id": n.id,
                            "name": n.name,
                            "kind": n.kind.to_string(),
                            "file": n.file,
                            "line": n.line_start
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "start": start,
                        "end": end,
                        "path": items,
                        "hops": items.len().saturating_sub(1)
                    }))?
                );
            }
            None => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "start": start,
                        "end": end,
                        "path": null,
                        "message": "No path found"
                    }))?
                );
            }
        }
    } else {
        match &found {
            Some(nodes) => {
                println!("Path ({} hops):\n", nodes.len().saturating_sub(1));
                for (i, n) in nodes.iter().enumerate() {
                    if i > 0 {
                        println!("    {}", "↓".dimmed());
                    }
                    println!(
                        "  {} {} {}",
                        n.kind.to_string().yellow(),
                        n.qualified_name.cyan(),
                        format!("({}:{})", n.file, n.line_start).dimmed()
                    );
                }
            }
            None => {
                println!("No path found between '{}' and '{}'", start, end);
            }
        }
    }

    Ok(())
}

pub fn open(symbol: &str, path: &Path) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;
    let graph = load_or_index_graph(&resolved_path)?;

    let (file, line) = resolve_node_or_file_target(&graph, symbol, &resolved_path)
        .ok_or_else(|| format!("Could not resolve symbol or file '{}'.", symbol))?;

    open_in_editor(&file, line)?;
    println!("{} Opened {}:{}", "✓".green(), file, line);
    Ok(())
}

pub fn map(
    path: &Path,
    token_budget: usize,
    exclude_test: bool,
    json_output: bool,
    verbose: bool,
    focus_changed: bool,
    focus_glob: Option<&str>,
) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;
    let mut graph = load_or_index_graph(&resolved_path)?;

    // Compute centrality if not already present, then persist for future calls
    let has_centrality = graph.node_indexes().any(|idx| graph.centrality(idx) > 0.0);
    if !has_centrality {
        eprintln!("Computing centrality...");
        let scores = compute_centrality(&graph, 20, 0.85);
        graph.set_centrality_scores(scores);
        let _ = save_graph_binary(&resolved_path, &graph);
    }

    // Build set of changed files for --focus-changed
    let changed_files: std::collections::HashSet<String> =
        if focus_changed && is_git_repo(&resolved_path) {
            git_changed_files(&resolved_path)
                .unwrap_or_default()
                .into_iter()
                .map(|f| resolved_path.join(&f).to_string_lossy().to_string())
                .collect()
        } else {
            std::collections::HashSet::new()
        };

    struct ScoredNode {
        name: String,
        kind: String,
        file: String,
        line_start: u32,
        line_end: u32,
        signature: Option<String>,
        score: f64,
        is_entry_point: bool,
        callers: usize,
    }

    let mut scored: Vec<ScoredNode> = Vec::new();
    for idx in graph.node_indexes() {
        let node = match graph.get(idx) {
            Some(n) => n,
            None => continue,
        };

        if exclude_test && is_test_file(&node.file) {
            continue;
        }

        // Skip minified/generated files
        if is_minified_or_generated(&node.file) {
            continue;
        }

        let kind_str = node.kind.to_string();
        if kind_str == "import" || kind_str == "export" || kind_str == "module" {
            continue;
        }

        let centrality = graph.centrality(idx);
        let is_entry = HeuristicsMatcher::is_likely_entry_point(node);
        let caller_count = graph.get_callers(idx).len();

        let kind_boost = match kind_str.as_str() {
            "class" | "interface" | "struct" => 0.1,
            "constructor" => -0.1,
            "field" | "constant" => -0.2,
            _ => 0.0,
        };
        let entry_boost = if is_entry { 0.3 } else { 0.0 };
        let changed_boost = if changed_files.contains(&node.file) {
            0.3
        } else {
            0.0
        };
        let glob_boost = match focus_glob {
            Some(pattern) if node.file.contains(pattern.trim_matches('*')) => 0.3,
            _ => 0.0,
        };
        let score = centrality + entry_boost + kind_boost + changed_boost + glob_boost;

        scored.push(ScoredNode {
            name: node.name.clone(),
            kind: kind_str,
            file: node.file.clone(),
            line_start: node.line_start,
            line_end: node.line_end,
            signature: node.signature.clone(),
            score,
            is_entry_point: is_entry,
            callers: caller_count,
        });
    }

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let total_symbols = scored.len();

    // Group by file, preserving rank order of first appearance.
    // Cap symbols per file to force breadth across the project.
    let max_per_file: usize = if token_budget <= 1024 {
        5
    } else if token_budget <= 2048 {
        8
    } else {
        12
    };

    let mut file_order: Vec<String> = Vec::new();
    let mut file_groups: std::collections::HashMap<String, Vec<&ScoredNode>> =
        std::collections::HashMap::new();
    for node in &scored {
        let group = file_groups.entry(node.file.clone()).or_default();
        if group.len() >= max_per_file {
            continue;
        }
        if group.is_empty() {
            file_order.push(node.file.clone());
        }
        group.push(node);
    }

    let root_str = resolved_path.to_string_lossy().to_string();
    let budget_chars = token_budget * 4;

    if json_output {
        let mut entries: Vec<serde_json::Value> = Vec::new();
        let mut json_symbols_shown = 0;
        let mut json_chars = 0;

        for file_path in &file_order {
            let symbols = match file_groups.get(file_path) {
                Some(s) => s,
                None => continue,
            };

            let rel_path = make_relative(file_path, &root_str);
            let short_path = compress_path(file_path, &root_str);

            let mut sym_items: Vec<serde_json::Value> = Vec::new();
            for node in symbols {
                let sig_short = node
                    .signature
                    .as_deref()
                    .map(shorten_signature)
                    .unwrap_or_else(|| node.name.clone());

                let item_cost = sig_short.len() + 50;
                if json_chars + item_cost > budget_chars && json_symbols_shown > 0 {
                    break;
                }

                sym_items.push(serde_json::json!({
                    "name": node.name,
                    "kind": node.kind,
                    "line": node.line_start,
                    "centrality": (node.score * 100.0).round() / 100.0,
                    "callers": node.callers,
                    "is_entry_point": node.is_entry_point,
                    "signature_short": sig_short,
                }));
                json_symbols_shown += 1;
                json_chars += item_cost;
            }

            if !sym_items.is_empty() {
                entries.push(serde_json::json!({
                    "file": rel_path,
                    "file_short": short_path,
                    "symbols": sym_items,
                }));
            }

            if json_chars >= budget_chars {
                break;
            }
        }

        let output = serde_json::json!({
            "schema": "arbor.map.v1",
            "token_estimate": json_chars / 4,
            "symbols_shown": json_symbols_shown,
            "symbols_total": total_symbols,
            "files_shown": entries.len(),
            "files_total": file_order.len(),
            "entries": entries,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        let mut output_lines: Vec<String> = Vec::new();
        let mut token_chars: usize = 0;
        let mut symbols_shown: usize = 0;
        let mut files_shown: usize = 0;
        let mut budget_hit = false;

        for file_path in &file_order {
            if budget_hit {
                break;
            }

            let symbols = match file_groups.get(file_path) {
                Some(s) => s,
                None => continue,
            };

            let display_path = if verbose {
                make_relative(file_path, &root_str)
            } else {
                compress_path(file_path, &root_str)
            };

            let header = format!("{}:", display_path);
            let header_cost = header.len() + 1;

            if token_chars + header_cost > budget_chars && files_shown > 0 {
                break;
            }

            output_lines.push(header);
            token_chars += header_cost;
            files_shown += 1;

            for node in symbols {
                let entry_marker = if node.is_entry_point { " ★" } else { "" };
                let line_info =
                    if node.kind == "class" || node.kind == "interface" || node.kind == "struct" {
                        format!("[L{}-{}]", node.line_start, node.line_end)
                    } else {
                        format!("L{}", node.line_start)
                    };

                let line =
                    if node.kind == "class" || node.kind == "interface" || node.kind == "struct" {
                        format!(
                            "  {} {} {}{}",
                            node.kind, node.name, line_info, entry_marker
                        )
                    } else {
                        let sig_short = node
                            .signature
                            .as_deref()
                            .map(shorten_signature)
                            .unwrap_or_else(|| node.name.clone());
                        format!("    {}  {}{}", sig_short, line_info, entry_marker)
                    };

                let line_cost = line.len() + 1;
                if token_chars + line_cost > budget_chars && symbols_shown > 0 {
                    let remaining_symbols = total_symbols - symbols_shown;
                    let remaining_files = file_order.len() - files_shown;
                    output_lines.push(format!(
                        "\n⋮... {} more symbols across {} files (use --tokens {} to see more)",
                        remaining_symbols,
                        remaining_files,
                        token_budget * 2
                    ));
                    budget_hit = true;
                    break;
                }

                output_lines.push(line);
                token_chars += line_cost;
                symbols_shown += 1;
            }

            if !budget_hit {
                output_lines.push(String::new());
                token_chars += 1;
            }
        }

        println!(
            "# arbor map ({} symbols, {} files, budget: {} tokens)\n",
            symbols_shown, files_shown, token_budget,
        );
        for line in &output_lines {
            println!("{}", line);
        }
    }
    Ok(())
}
