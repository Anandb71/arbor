//! Main application state and UI logic

use arbor_graph::ArborGraph;
use arbor_watcher::{index_directory, IndexOptions};
use eframe::egui;
use std::path::PathBuf;

/// Analysis result for display
#[derive(Default)]
struct AnalysisResult {
    target_name: String,
    target_file: String,
    role: String,
    direct_callers: Vec<String>,
    indirect_callers: Vec<String>,
    downstream: Vec<String>,
    total_affected: usize,
    confidence: String,
}

/// Main application state
pub struct ArborApp {
    /// Current working directory
    cwd: PathBuf,

    /// Symbol input field
    symbol_input: String,

    /// Indexed graph (lazy loaded)
    graph: Option<ArborGraph>,

    /// Current analysis result
    result: Option<AnalysisResult>,

    /// Status message
    status: String,

    /// Is analysis in progress
    loading: bool,

    /// Dark mode toggle
    dark_mode: bool,

    /// Search history
    search_history: Vec<String>,

    /// Show call tree (collapsible)
    show_call_tree: bool,

    /// Show dependencies (collapsible)
    show_dependencies: bool,
}

impl ArborApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            cwd: std::env::current_dir().unwrap_or_default(),
            symbol_input: String::new(),
            graph: None,
            result: None,
            status: "Ready. Enter a symbol name to analyze.".to_string(),
            loading: false,
            dark_mode: true,
            search_history: Vec::new(),
            show_call_tree: true,
            show_dependencies: true,
        }
    }

    fn analyze(&mut self) {
        if self.symbol_input.trim().is_empty() {
            self.status = "Please enter a symbol name.".to_string();
            return;
        }

        // Index if not already done
        if self.graph.is_none() {
            self.status = "Indexing codebase...".to_string();
            match index_directory(&self.cwd, IndexOptions::default()) {
                Ok(result) => {
                    self.graph = Some(result.graph);
                    self.status = format!("Indexed {} nodes.", result.nodes_extracted);
                }
                Err(e) => {
                    self.status = format!("Indexing failed: {}", e);
                    return;
                }
            }
        }

        // Analyze the symbol
        if let Some(graph) = &self.graph {
            let target = self.symbol_input.trim();

            // Find the node
            let node_idx = graph.get_index(target).or_else(|| {
                graph
                    .find_by_name(target)
                    .first()
                    .and_then(|n| graph.get_index(&n.id))
            });

            match node_idx {
                Some(idx) => {
                    let node = graph.get(idx).unwrap();
                    let analysis = graph.analyze_impact(idx, 5);

                    let has_upstream = !analysis.upstream.is_empty();
                    let has_downstream = !analysis.downstream.is_empty();

                    let role = match (has_upstream, has_downstream) {
                        (false, false) => "Isolated",
                        (false, true) => "Entry Point",
                        (true, false) => "Utility",
                        (true, true) => "Core Logic",
                    };

                    let direct: Vec<_> = analysis
                        .all_affected()
                        .into_iter()
                        .filter(|n| n.severity == arbor_graph::ImpactSeverity::Direct)
                        .map(|n| format!("{} ({})", n.node_info.name, n.node_info.kind))
                        .collect();

                    let indirect: Vec<_> = analysis
                        .all_affected()
                        .into_iter()
                        .filter(|n| n.severity != arbor_graph::ImpactSeverity::Direct)
                        .map(|n| format!("{} ({} hops)", n.node_info.name, n.hop_distance))
                        .collect();

                    let downstream: Vec<_> = analysis
                        .downstream
                        .iter()
                        .take(10)
                        .map(|n| format!("{} ({})", n.node_info.name, n.entry_edge))
                        .collect();

                    self.result = Some(AnalysisResult {
                        target_name: node.name.clone(),
                        target_file: node.file.clone(),
                        role: role.to_string(),
                        direct_callers: direct,
                        indirect_callers: indirect,
                        downstream,
                        total_affected: analysis.total_affected,
                        confidence: if analysis.total_affected == 0 {
                            "Low (no edges found)".to_string()
                        } else {
                            "High".to_string()
                        },
                    });

                    self.status = format!("Analyzed '{}'", target);

                    // Add to search history
                    let query = target.to_string();
                    if !self.search_history.contains(&query) {
                        self.search_history.insert(0, query);
                        if self.search_history.len() > 10 {
                            self.search_history.pop();
                        }
                    }
                }
                None => {
                    self.result = None;
                    self.status = format!("Symbol '{}' not found.", target);
                }
            }
        }
    }

    fn copy_as_markdown(&self) -> String {
        if let Some(r) = &self.result {
            let mut md = format!("## Impact Analysis: {}\n\n", r.target_name);
            md += &format!("**File:** `{}`\n", r.target_file);
            md += &format!("**Role:** {}\n", r.role);
            md += &format!("**Confidence:** {}\n\n", r.confidence);

            if !r.direct_callers.is_empty() {
                md += "### Direct Callers (will break immediately)\n";
                for c in &r.direct_callers {
                    md += &format!("- {}\n", c);
                }
                md += "\n";
            }

            if !r.indirect_callers.is_empty() {
                md += "### Indirect Callers (may break)\n";
                for c in r.indirect_callers.iter().take(5) {
                    md += &format!("- {}\n", c);
                }
                md += "\n";
            }

            md += &format!("**Total Affected:** {} nodes\n", r.total_affected);
            md
        } else {
            String::new()
        }
    }
}

impl eframe::App for ArborApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply theme
        if self.dark_mode {
            ctx.set_visuals(egui::Visuals::dark());
        } else {
            ctx.set_visuals(egui::Visuals::light());
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            // Header
            ui.horizontal(|ui| {
                ui.heading("🌲 Arbor");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(if self.dark_mode { "☀" } else { "🌙" }).clicked() {
                        self.dark_mode = !self.dark_mode;
                    }
                });
            });

            ui.separator();

            // Input section
            ui.horizontal(|ui| {
                ui.label("Symbol:");
                let response = ui.text_edit_singleline(&mut self.symbol_input);
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.analyze();
                }
                if ui.button("🔍 Analyze").clicked() {
                    self.analyze();
                }
            });

            // Search history
            let mut clicked_query: Option<String> = None;
            if !self.search_history.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("History:").small().weak());
                    for query in self.search_history.iter().take(5) {
                        if ui.small_button(query).clicked() {
                            clicked_query = Some(query.clone());
                        }
                    }
                });
            }
            if let Some(query) = clicked_query {
                self.symbol_input = query;
                self.analyze();
            }

            ui.label(egui::RichText::new(&self.status).small().weak());

            ui.separator();

            // Results section
            if let Some(r) = &self.result {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading(&r.target_name);
                    ui.label(format!("File: {}", r.target_file));
                    ui.horizontal(|ui| {
                        ui.label("Role:");
                        ui.label(egui::RichText::new(&r.role).strong());
                    });
                    ui.horizontal(|ui| {
                        ui.label("Confidence:");
                        ui.label(&r.confidence);
                    });

                    ui.add_space(10.0);

                    // Direct callers
                    if !r.direct_callers.is_empty() {
                        ui.label(egui::RichText::new("⚠️ Will break immediately:").strong().color(egui::Color32::RED));
                        for c in &r.direct_callers {
                            ui.label(format!("  • {}", c));
                        }
                    }

                    ui.add_space(5.0);

                    // Indirect callers
                    if !r.indirect_callers.is_empty() {
                        ui.label(egui::RichText::new("May break indirectly:").strong().color(egui::Color32::YELLOW));
                        for c in r.indirect_callers.iter().take(5) {
                            ui.label(format!("  • {}", c));
                        }
                        if r.indirect_callers.len() > 5 {
                            ui.label(format!("  ... and {} more", r.indirect_callers.len() - 5));
                        }
                    }

                    ui.add_space(5.0);

                    // Downstream
                    if !r.downstream.is_empty() {
                        ui.label(egui::RichText::new("Dependencies:").strong());
                        for d in &r.downstream {
                            ui.label(format!("  └─ {}", d));
                        }
                    }

                    ui.add_space(10.0);

                    ui.label(format!("Total affected: {} nodes", r.total_affected));

                    ui.add_space(10.0);

                    // Copy button
                    if ui.button("📋 Copy as Markdown").clicked() {
                        let md = self.copy_as_markdown();
                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                            let _ = clipboard.set_text(md);
                        }
                    }
                });
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Enter a function or class name above to analyze its impact.");
                });
            }
        });
    }
}
