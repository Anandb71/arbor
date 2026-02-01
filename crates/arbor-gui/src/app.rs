// Full file skipped by rustfmt manually via block-level attributes to avoid unstable inner attributes
//! Main application state and UI logic

use arbor_graph::ArborGraph;
use arbor_watcher::{index_directory, IndexOptions};
use eframe::egui;
use std::path::PathBuf;

/// Analysis result for display
#[rustfmt::skip]
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
#[rustfmt::skip]
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

    /// Show file path (spoiler mode - click to reveal)
    show_file_path: bool,
}

#[rustfmt::skip]
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
            show_file_path: false, // Hidden by default (spoiler mode)
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
                    self.status = format!(
                        "Symbol '{}' not found. Try: arbor status --files to see indexed files.",
                        target
                    );
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

#[rustfmt::skip]
impl eframe::App for ArborApp {
    #[rustfmt::skip]
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
                // ui.heading("🌲 Arbor");
                ui.image(egui::include_image!("../../../docs/assets/arbor-logo.svg"));
                ui.heading("Arbor");
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

            // Results section - extract values to avoid borrow issues
            let result_data = self.result.as_ref().map(|r| {
                (r.target_name.clone(), r.target_file.clone(), r.role.clone(), 
                 r.confidence.clone(), r.direct_callers.clone(), r.indirect_callers.clone(),
                 r.downstream.clone(), r.total_affected)
            });
            
            let mut toggle_file_path = false;
            let mut toggle_hide_path = false;

            if let Some((target_name, target_file, role, confidence, direct_callers, indirect_callers, downstream, total_affected)) = result_data {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading(&target_name);
                    
                    // File path with Discord-style spoiler
                    ui.horizontal(|ui| {
                        ui.label("File:");
                        if self.show_file_path {
                            ui.label(egui::RichText::new(&target_file).monospace());
                            if ui.small_button("🙈 Hide").clicked() {
                                toggle_hide_path = true;
                            }
                        } else {
                            // Spoiler box - click to reveal
                            let spoiler_text = "████████████████";
                            let button = egui::Button::new(
                                egui::RichText::new(spoiler_text)
                                    .background_color(egui::Color32::DARK_GRAY)
                                    .color(egui::Color32::DARK_GRAY),
                            )
                            .frame(false);

                            if ui
                                .add(button)
                                .on_hover_text("Click to reveal file path")
                                .clicked()
                            {
                                toggle_file_path = true;
                            }
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Role:");
                        ui.label(egui::RichText::new(&role).strong());
                    });
                    ui.horizontal(|ui| {
                        ui.label("Confidence:");
                        ui.label(&confidence);
                    });
                    // Confidence explainer
                    ui.label(egui::RichText::new("Based on static call graph and resolved symbols.").small().weak());

                    ui.add_space(10.0);

                    // Direct callers
                    if !direct_callers.is_empty() {
                        ui.label(egui::RichText::new("⚠️ Will break immediately:").strong().color(egui::Color32::RED));
                        for c in &direct_callers {
                            ui.label(format!("  • {}", c));
                        }
                    }

                    ui.add_space(5.0);

                    // Indirect callers
                    if !indirect_callers.is_empty() {
                        ui.label(egui::RichText::new("May break indirectly:").strong().color(egui::Color32::YELLOW));
                        for c in indirect_callers.iter().take(5) {
                            ui.label(format!("  • {}", c));
                        }
                        if indirect_callers.len() > 5 {
                            ui.label(format!("  ... and {} more", indirect_callers.len() - 5));
                        }
                    }

                    ui.add_space(5.0);

                    // Downstream dependencies with clearer labels
                    if !downstream.is_empty() {
                        ui.label(egui::RichText::new("Dependencies:").strong());
                        ui.label(egui::RichText::new("This function calls these lower-level helpers.").small().weak());
                        for d in &downstream {
                            ui.label(format!("  → Calls {}", d));
                        }
                    }

                    ui.add_space(10.0);

                    ui.label(format!("Total affected: {} nodes", total_affected));

                    ui.add_space(10.0);

                    // Copy button
                    if ui.button("📋 Copy as Markdown").clicked() {
                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                            let md = format!(
                                "## Impact Analysis: {}\n\n**File:** `{}`\n**Role:** {}\n**Confidence:** {}\n**Total Affected:** {} nodes",
                                target_name, target_file, role, confidence, total_affected
                            );
                            let _ = clipboard.set_text(md);
                        }
                    }
                });
                
                // Apply spoiler toggles after closure
                if toggle_file_path {
                    self.show_file_path = true;
                }
                if toggle_hide_path {
                    self.show_file_path = false;
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Enter a function or class name above to analyze its impact.");
                });
            }
            
            // Version watermark (bottom right)
            ui.with_layout(egui::Layout::bottom_up(egui::Align::RIGHT), |ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Arbor v1.5").small().weak());
            });
        });
    }
}
