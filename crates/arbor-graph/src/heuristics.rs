//! Heuristics for detecting runtime edges and framework patterns
//!
//! Real codebases aren't clean. This module provides best-effort detection of:
//! - Dynamic/callback calls
//! - Framework-specific patterns (Flutter widgets, etc.)
//! - Possible runtime dependencies

use arbor_core::{CodeNode, NodeKind};

/// Types of uncertain edges
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UncertainEdgeKind {
    /// Callback or closure passed as argument
    Callback,
    /// Dynamic dispatch (trait objects, interfaces)
    DynamicDispatch,
    /// Framework widget tree (Flutter, React, etc.)
    WidgetTree,
    /// Event handler registration
    EventHandler,
    /// Dependency injection
    DependencyInjection,
    /// Reflection or runtime lookup
    Reflection,
}

impl std::fmt::Display for UncertainEdgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UncertainEdgeKind::Callback => write!(f, "callback"),
            UncertainEdgeKind::DynamicDispatch => write!(f, "dynamic dispatch"),
            UncertainEdgeKind::WidgetTree => write!(f, "widget tree"),
            UncertainEdgeKind::EventHandler => write!(f, "event handler"),
            UncertainEdgeKind::DependencyInjection => write!(f, "dependency injection"),
            UncertainEdgeKind::Reflection => write!(f, "reflection"),
        }
    }
}

/// An edge that might exist at runtime but cannot be proven statically
#[derive(Debug, Clone)]
pub struct UncertainEdge {
    pub from: String,
    pub to: String,
    pub kind: UncertainEdgeKind,
    pub confidence: f32, // 0.0 to 1.0
    pub reason: String,
}

/// Pattern matchers for different frameworks and languages
pub struct HeuristicsMatcher;

impl HeuristicsMatcher {
    /// Check if a node looks like a Flutter widget
    pub fn is_flutter_widget(node: &CodeNode) -> bool {
        // Widget classes typically extend StatelessWidget or StatefulWidget
        node.kind == NodeKind::Class
            && (node.name.ends_with("Widget")
                || node.name.ends_with("State")
                || node.name.ends_with("Page")
                || node.name.ends_with("Screen")
                || node.name.ends_with("View"))
    }

    /// Check if a node looks like a React component
    pub fn is_react_component(node: &CodeNode) -> bool {
        (node.kind == NodeKind::Function || node.kind == NodeKind::Class)
            && node.file.ends_with(".tsx")
            && node.name.chars().next().is_some_and(|c| c.is_uppercase())
    }

    /// Check if a node looks like an event handler
    pub fn is_event_handler(node: &CodeNode) -> bool {
        let name_lower = node.name.to_lowercase();
        (node.kind == NodeKind::Function || node.kind == NodeKind::Method)
            && (name_lower.starts_with("on")
                || name_lower.starts_with("handle")
                || name_lower.ends_with("handler")
                || name_lower.ends_with("callback")
                || name_lower.ends_with("listener"))
    }

    /// Check if a node looks like a callback parameter
    pub fn is_callback_style(node: &CodeNode) -> bool {
        let name_lower = node.name.to_lowercase();
        name_lower.ends_with("fn")
            || name_lower.ends_with("callback")
            || name_lower.ends_with("handler")
            || name_lower.starts_with("on_")
    }

    /// Check if a node looks like a factory or provider (DI pattern)
    pub fn is_dependency_injection(node: &CodeNode) -> bool {
        let name_lower = node.name.to_lowercase();
        name_lower.ends_with("factory")
            || name_lower.ends_with("provider")
            || name_lower.ends_with("injector")
            || name_lower.ends_with("container")
            || name_lower.contains("singleton")
    }

    /// Check if a node is likely a production entry point.
    ///
    /// Entry points are the outermost functions that receive external requests —
    /// HTTP handlers, CLI commands, cron jobs, webhook receivers, main functions.
    /// They're the roots of execution trees; if a changed function reaches one,
    /// it means the change can affect real production traffic.
    pub fn is_likely_entry_point(node: &CodeNode) -> bool {
        if !matches!(node.kind, NodeKind::Function | NodeKind::Method) {
            return false;
        }
        let name = node.name.to_lowercase();
        let file = node.file.to_lowercase();

        // Main / program entry
        if node.name == "main" || node.name == "__main__" {
            return true;
        }

        // HTTP route handlers (Express, Gin, axum, FastAPI, Flask, Django, Rails)
        if name.ends_with("_view")
            || name.ends_with("_handler")
            || name.ends_with("_controller")
            || name.ends_with("_endpoint")
            || name.starts_with("handle_")
            || name.starts_with("get_")
            || name.starts_with("post_")
            || name.starts_with("put_")
            || name.starts_with("delete_")
            || name.starts_with("patch_")
        {
            // Only count as entry point if in a routes/views/handlers/controllers file
            if file.contains("route")
                || file.contains("view")
                || file.contains("handler")
                || file.contains("controller")
                || file.contains("endpoint")
                || file.contains("api")
            {
                return true;
            }
        }

        // Webhook / event receivers
        if name.contains("webhook")
            || name.contains("receive")
            || name.contains("subscribe")
            || name.starts_with("on_")
            || (name.starts_with("handle") && !file.contains("test"))
        {
            return true;
        }

        // Background jobs / cron / workers
        if (name.contains("job")
            || name.contains("task")
            || name.contains("worker")
            || name.contains("cron")
            || name.ends_with("_run")
            || name == "run"
            || name == "execute"
            || name == "process")
            && (file.contains("job")
                || file.contains("task")
                || file.contains("worker")
                || file.contains("cron")
                || file.contains("celery")
                || file.contains("sidekiq")
                || file.contains("background"))
        {
            return true;
        }

        // CLI command handlers
        if name.contains("command") || name.ends_with("_cmd") || name == "cli" || name == "invoke" {
            return true;
        }

        false
    }

    /// Infer uncertain edges from node patterns
    pub fn infer_uncertain_edges(nodes: &[&CodeNode]) -> Vec<UncertainEdge> {
        let mut edges = Vec::new();

        for node in nodes {
            // Event handlers likely connected to event sources
            if Self::is_event_handler(node) {
                edges.push(UncertainEdge {
                    from: "event_source".to_string(),
                    to: node.id.clone(),
                    kind: UncertainEdgeKind::EventHandler,
                    confidence: 0.7,
                    reason: format!("'{}' looks like an event handler", node.name),
                });
            }

            // Callbacks likely invoked dynamically
            if Self::is_callback_style(node) {
                edges.push(UncertainEdge {
                    from: "caller".to_string(),
                    to: node.id.clone(),
                    kind: UncertainEdgeKind::Callback,
                    confidence: 0.6,
                    reason: format!("'{}' is likely passed as a callback", node.name),
                });
            }

            // Flutter widgets part of widget tree
            if Self::is_flutter_widget(node) {
                edges.push(UncertainEdge {
                    from: "parent_widget".to_string(),
                    to: node.id.clone(),
                    kind: UncertainEdgeKind::WidgetTree,
                    confidence: 0.8,
                    reason: format!("'{}' is a Flutter widget in the widget tree", node.name),
                });
            }
        }

        edges
    }
}

/// Warnings about analysis limitations
#[derive(Debug, Clone)]
pub struct AnalysisWarning {
    pub message: String,
    pub suggestion: String,
}

impl AnalysisWarning {
    pub fn new(message: impl Into<String>, suggestion: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            suggestion: suggestion.into(),
        }
    }
}

/// Check for common patterns that limit static analysis accuracy
pub fn detect_analysis_limitations(nodes: &[&CodeNode]) -> Vec<AnalysisWarning> {
    let mut warnings = Vec::new();

    let callback_count = nodes
        .iter()
        .filter(|n| HeuristicsMatcher::is_callback_style(n))
        .count();
    if callback_count > 5 {
        warnings.push(AnalysisWarning::new(
            format!("Found {} callback-style nodes", callback_count),
            "Callbacks may be invoked dynamically. Verify runtime behavior.",
        ));
    }

    let event_handler_count = nodes
        .iter()
        .filter(|n| HeuristicsMatcher::is_event_handler(n))
        .count();
    if event_handler_count > 3 {
        warnings.push(AnalysisWarning::new(
            format!("Found {} event handlers", event_handler_count),
            "Event handlers are connected at runtime. Check event sources.",
        ));
    }

    let widget_count = nodes
        .iter()
        .filter(|n| HeuristicsMatcher::is_flutter_widget(n))
        .count();
    if widget_count > 0 {
        warnings.push(AnalysisWarning::new(
            format!("Detected {} Flutter widgets", widget_count),
            "Widget tree hierarchy is determined at runtime.",
        ));
    }

    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flutter_widget_detection() {
        let widget = CodeNode::new("HomeWidget", "HomeWidget", NodeKind::Class, "home.dart");
        assert!(HeuristicsMatcher::is_flutter_widget(&widget));

        let state = CodeNode::new("HomeState", "HomeState", NodeKind::Class, "home.dart");
        assert!(HeuristicsMatcher::is_flutter_widget(&state));

        let non_widget = CodeNode::new(
            "UserService",
            "UserService",
            NodeKind::Class,
            "service.dart",
        );
        assert!(!HeuristicsMatcher::is_flutter_widget(&non_widget));
    }

    #[test]
    fn test_event_handler_detection() {
        let handler = CodeNode::new("onClick", "onClick", NodeKind::Function, "button.ts");
        assert!(HeuristicsMatcher::is_event_handler(&handler));

        let handler2 = CodeNode::new(
            "handleSubmit",
            "handleSubmit",
            NodeKind::Function,
            "form.ts",
        );
        assert!(HeuristicsMatcher::is_event_handler(&handler2));

        let non_handler = CodeNode::new("calculate", "calculate", NodeKind::Function, "math.ts");
        assert!(!HeuristicsMatcher::is_event_handler(&non_handler));
    }

    #[test]
    fn test_react_component_detection() {
        let component = CodeNode::new(
            "UserProfile",
            "UserProfile",
            NodeKind::Function,
            "profile.tsx",
        );
        assert!(HeuristicsMatcher::is_react_component(&component));

        let non_component = CodeNode::new("helper", "helper", NodeKind::Function, "utils.tsx");
        assert!(!HeuristicsMatcher::is_react_component(&non_component));

        // Not a .tsx file -> not a React component
        let wrong_ext = CodeNode::new(
            "UserProfile",
            "UserProfile",
            NodeKind::Function,
            "profile.rs",
        );
        assert!(!HeuristicsMatcher::is_react_component(&wrong_ext));

        // Class in .tsx is also a React component
        let class_comp = CodeNode::new("AppContainer", "AppContainer", NodeKind::Class, "app.tsx");
        assert!(HeuristicsMatcher::is_react_component(&class_comp));
    }

    #[test]
    fn test_callback_style_detection() {
        let callback = CodeNode::new(
            "on_click_handler",
            "on_click_handler",
            NodeKind::Function,
            "a.rs",
        );
        assert!(HeuristicsMatcher::is_callback_style(&callback));

        let callback_fn = CodeNode::new("sortFn", "sortFn", NodeKind::Function, "a.ts");
        assert!(HeuristicsMatcher::is_callback_style(&callback_fn));

        let regular = CodeNode::new("process_data", "process_data", NodeKind::Function, "a.rs");
        assert!(!HeuristicsMatcher::is_callback_style(&regular));
    }

    #[test]
    fn test_dependency_injection_detection() {
        let factory = CodeNode::new("UserFactory", "UserFactory", NodeKind::Class, "factory.ts");
        assert!(HeuristicsMatcher::is_dependency_injection(&factory));

        let provider = CodeNode::new("AuthProvider", "AuthProvider", NodeKind::Class, "auth.ts");
        assert!(HeuristicsMatcher::is_dependency_injection(&provider));

        let regular = CodeNode::new("UserService", "UserService", NodeKind::Class, "service.ts");
        assert!(!HeuristicsMatcher::is_dependency_injection(&regular));
    }

    #[test]
    fn test_infer_uncertain_edges_from_patterns() {
        let handler = CodeNode::new("onClick", "onClick", NodeKind::Function, "button.ts");
        let widget = CodeNode::new("HomeWidget", "HomeWidget", NodeKind::Class, "home.dart");
        let regular = CodeNode::new("calculate", "calculate", NodeKind::Function, "math.ts");

        let nodes: Vec<&CodeNode> = vec![&handler, &widget, &regular];
        let edges = HeuristicsMatcher::infer_uncertain_edges(&nodes);

        // Should have edges for handler (EventHandler) and widget (WidgetTree)
        assert!(edges
            .iter()
            .any(|e| matches!(e.kind, UncertainEdgeKind::EventHandler)));
        assert!(edges
            .iter()
            .any(|e| matches!(e.kind, UncertainEdgeKind::WidgetTree)));
        // Regular function shouldn't produce uncertain edges
        assert!(!edges.iter().any(|e| e.to == regular.id));
    }

    #[test]
    fn test_detect_analysis_limitations_callbacks() {
        // Create 6+ callback-style nodes to trigger the warning
        let nodes: Vec<CodeNode> = (0..7)
            .map(|i| {
                CodeNode::new(
                    format!("on_event_{}", i),
                    format!("on_event_{}", i),
                    NodeKind::Function,
                    "events.ts",
                )
            })
            .collect();
        let node_refs: Vec<&CodeNode> = nodes.iter().collect();

        let warnings = detect_analysis_limitations(&node_refs);
        assert!(!warnings.is_empty());
        assert!(warnings.iter().any(|w| w.message.contains("callback")));
    }

    #[test]
    fn test_detect_analysis_limitations_flutter_widgets() {
        let widgets: Vec<CodeNode> = vec![CodeNode::new(
            "HomeWidget",
            "HomeWidget",
            NodeKind::Class,
            "home.dart",
        )];
        let node_refs: Vec<&CodeNode> = widgets.iter().collect();

        let warnings = detect_analysis_limitations(&node_refs);
        assert!(warnings.iter().any(|w| w.message.contains("Flutter")));
    }

    #[test]
    fn test_uncertain_edge_kind_display() {
        assert_eq!(UncertainEdgeKind::Callback.to_string(), "callback");
        assert_eq!(
            UncertainEdgeKind::DynamicDispatch.to_string(),
            "dynamic dispatch"
        );
        assert_eq!(UncertainEdgeKind::WidgetTree.to_string(), "widget tree");
        assert_eq!(UncertainEdgeKind::EventHandler.to_string(), "event handler");
        assert_eq!(
            UncertainEdgeKind::DependencyInjection.to_string(),
            "dependency injection"
        );
        assert_eq!(UncertainEdgeKind::Reflection.to_string(), "reflection");
    }

    #[test]
    fn test_no_warnings_for_clean_code() {
        let nodes: Vec<CodeNode> = vec![
            CodeNode::new("main", "main", NodeKind::Function, "main.rs"),
            CodeNode::new("helper", "helper", NodeKind::Function, "utils.rs"),
        ];
        let node_refs: Vec<&CodeNode> = nodes.iter().collect();

        let warnings = detect_analysis_limitations(&node_refs);
        assert!(warnings.is_empty());
    }
}
