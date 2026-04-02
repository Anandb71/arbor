//! Confidence scoring for impact analysis
//!
//! Provides explainable risk levels (Low/Medium/High) based on graph structure.

use crate::ImpactAnalysis;

/// Confidence level for an analysis result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfidenceLevel {
    /// High confidence - well-connected node with clear edges
    High,
    /// Medium confidence - some uncertainty exists
    Medium,
    /// Low confidence - significant unknowns
    Low,
}

impl std::fmt::Display for ConfidenceLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfidenceLevel::High => write!(f, "High"),
            ConfidenceLevel::Medium => write!(f, "Medium"),
            ConfidenceLevel::Low => write!(f, "Low"),
        }
    }
}

/// Reasons explaining the confidence level
#[derive(Debug, Clone)]
pub struct ConfidenceExplanation {
    pub level: ConfidenceLevel,
    pub reasons: Vec<String>,
    pub suggestions: Vec<String>,
}

impl ConfidenceExplanation {
    /// Compute confidence from an impact analysis
    pub fn from_analysis(analysis: &ImpactAnalysis) -> Self {
        let mut reasons = Vec::new();
        let mut suggestions = Vec::new();

        let upstream_count = analysis.upstream.len();
        let downstream_count = analysis.downstream.len();
        let total = analysis.total_affected;

        // Determine base confidence from connectivity
        let level = if upstream_count == 0 && downstream_count == 0 {
            // Isolated node
            reasons.push("Node appears isolated (no detected connections)".to_string());
            suggestions
                .push("Verify if this is called dynamically or from external code".to_string());
            ConfidenceLevel::Low
        } else if upstream_count == 0 {
            // Entry point
            reasons.push("Node is an entry point (no internal callers)".to_string());
            reasons.push(format!("Has {} downstream dependencies", downstream_count));
            if downstream_count > 5 {
                suggestions.push("Consider impact on downstream dependencies".to_string());
                ConfidenceLevel::Medium
            } else {
                ConfidenceLevel::High
            }
        } else if downstream_count == 0 {
            // Leaf/utility node
            reasons.push("Node is a utility (no outgoing dependencies)".to_string());
            reasons.push(format!("Called by {} upstream nodes", upstream_count));
            ConfidenceLevel::High
        } else {
            // Connected node
            reasons.push(format!(
                "{} callers, {} dependencies",
                upstream_count, downstream_count
            ));

            if total > 50 {
                reasons.push("Very large blast radius".to_string());
                suggestions
                    .push("This change affects a significant portion of the codebase".to_string());
                ConfidenceLevel::Low
            } else if total > 20 {
                reasons.push("Large blast radius detected".to_string());
                suggestions
                    .push("Consider breaking this change into smaller refactors".to_string());
                ConfidenceLevel::Medium
            } else {
                reasons.push("Well-connected with manageable impact".to_string());
                ConfidenceLevel::High
            }
        };

        // Add structural insights
        if total > 0 {
            let direct_count = analysis
                .upstream
                .iter()
                .filter(|n| n.hop_distance == 1)
                .count();
            if direct_count > 0 {
                reasons.push(format!("{} nodes will break immediately", direct_count));
            }
        }

        // Standard disclaimer
        suggestions.push("Tests still recommended for behavioral verification".to_string());

        Self {
            level,
            reasons,
            suggestions,
        }
    }
}

/// Node role classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRole {
    /// Entry point - receives control from outside
    EntryPoint,
    /// Utility - helper function called by others
    Utility,
    /// Core logic - central to the domain
    CoreLogic,
    /// Isolated - no detected connections
    Isolated,
    /// Adapter - boundary between layers
    Adapter,
}

impl std::fmt::Display for NodeRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeRole::EntryPoint => write!(f, "Entry Point"),
            NodeRole::Utility => write!(f, "Utility"),
            NodeRole::CoreLogic => write!(f, "Core Logic"),
            NodeRole::Isolated => write!(f, "Isolated"),
            NodeRole::Adapter => write!(f, "Adapter"),
        }
    }
}

impl NodeRole {
    /// Determine role from impact analysis
    pub fn from_analysis(analysis: &ImpactAnalysis) -> Self {
        let has_upstream = !analysis.upstream.is_empty();
        let has_downstream = !analysis.downstream.is_empty();

        match (has_upstream, has_downstream) {
            (false, false) => NodeRole::Isolated,
            (false, true) => NodeRole::EntryPoint,
            (true, false) => NodeRole::Utility,
            (true, true) => {
                // Distinguish between adapter and core logic
                let upstream_count = analysis.upstream.len();
                let downstream_count = analysis.downstream.len();

                // Adapters typically have few callers but many dependencies (or vice versa)
                if (upstream_count <= 2 && downstream_count > 5)
                    || (downstream_count <= 2 && upstream_count > 5)
                {
                    NodeRole::Adapter
                } else {
                    NodeRole::CoreLogic
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AffectedNode, EdgeKind, ImpactDirection, ImpactSeverity, NodeInfo};

    fn node_info(id: &str) -> NodeInfo {
        NodeInfo {
            id: id.to_string(),
            name: id.to_string(),
            qualified_name: id.to_string(),
            kind: "function".to_string(),
            file: "test.rs".to_string(),
            line_start: 1,
            line_end: 1,
            signature: None,
            centrality: 0.0,
        }
    }

    fn affected(id: &str, hop_distance: usize, direction: ImpactDirection) -> AffectedNode {
        AffectedNode {
            node_id: crate::NodeId::new(hop_distance),
            node_info: node_info(id),
            severity: ImpactSeverity::from_hops(hop_distance),
            hop_distance,
            entry_edge: EdgeKind::Calls,
            direction,
        }
    }

    fn analysis(upstream: usize, downstream: usize, total_affected: usize) -> ImpactAnalysis {
        let upstream_nodes = (0..upstream)
            .map(|i| {
                let hop = if i % 2 == 0 { 1 } else { 2 };
                affected(&format!("u{i}"), hop, ImpactDirection::Upstream)
            })
            .collect();

        let downstream_nodes = (0..downstream)
            .map(|i| {
                let hop = if i % 2 == 0 { 1 } else { 2 };
                affected(&format!("d{i}"), hop, ImpactDirection::Downstream)
            })
            .collect();

        ImpactAnalysis {
            target: node_info("target"),
            upstream: upstream_nodes,
            downstream: downstream_nodes,
            total_affected,
            max_depth: 3,
            query_time_ms: 1,
        }
    }

    #[test]
    fn test_confidence_level_display() {
        assert_eq!(ConfidenceLevel::High.to_string(), "High");
        assert_eq!(ConfidenceLevel::Medium.to_string(), "Medium");
        assert_eq!(ConfidenceLevel::Low.to_string(), "Low");
    }

    #[test]
    fn test_node_role_display() {
        assert_eq!(NodeRole::EntryPoint.to_string(), "Entry Point");
        assert_eq!(NodeRole::Utility.to_string(), "Utility");
        assert_eq!(NodeRole::CoreLogic.to_string(), "Core Logic");
        assert_eq!(NodeRole::Isolated.to_string(), "Isolated");
        assert_eq!(NodeRole::Adapter.to_string(), "Adapter");
    }

    #[test]
    fn test_confidence_connected_thresholds_regression() {
        let medium_case = analysis(10, 20, 30);
        let low_case = analysis(20, 40, 60);

        let medium = ConfidenceExplanation::from_analysis(&medium_case);
        let low = ConfidenceExplanation::from_analysis(&low_case);

        assert_eq!(medium.level, ConfidenceLevel::Medium);
        assert_eq!(low.level, ConfidenceLevel::Low);
        assert!(medium
            .reasons
            .iter()
            .any(|r| r.contains("Large blast radius")));
        assert!(low.reasons.iter().any(|r| r.contains("Very large blast radius")));
    }

    #[test]
    fn test_confidence_entry_point_matrix_120_cases() {
        let mut cases = 0;
        for downstream in 1..=120 {
            let a = analysis(0, downstream, downstream);
            let explanation = ConfidenceExplanation::from_analysis(&a);
            let expected = if downstream > 5 {
                ConfidenceLevel::Medium
            } else {
                ConfidenceLevel::High
            };
            assert_eq!(
                explanation.level, expected,
                "entry-point mismatch for downstream={downstream}"
            );
            cases += 1;
        }
        assert_eq!(cases, 120);
    }

    #[test]
    fn test_confidence_utility_matrix_120_cases() {
        let mut cases = 0;
        for upstream in 1..=120 {
            let a = analysis(upstream, 0, upstream);
            let explanation = ConfidenceExplanation::from_analysis(&a);
            assert_eq!(
                explanation.level,
                ConfidenceLevel::High,
                "utility mismatch for upstream={upstream}"
            );
            cases += 1;
        }
        assert_eq!(cases, 120);
    }

    #[test]
    fn test_confidence_connected_matrix_121_cases() {
        let mut cases = 0;
        for upstream in 1..=11 {
            for downstream in 1..=11 {
                // Ensure we exercise all connected-tier branches deterministically:
                // <=20 (High), 21..=50 (Medium), >50 (Low)
                let total = match (upstream + downstream) % 3 {
                    0 => 15,
                    1 => 35,
                    _ => 70,
                };

                let expected = if total > 50 {
                    ConfidenceLevel::Low
                } else if total > 20 {
                    ConfidenceLevel::Medium
                } else {
                    ConfidenceLevel::High
                };

                let a = analysis(upstream, downstream, total);
                let explanation = ConfidenceExplanation::from_analysis(&a);
                assert_eq!(
                    explanation.level, expected,
                    "connected mismatch for upstream={upstream}, downstream={downstream}, total={total}"
                );
                cases += 1;
            }
        }
        assert_eq!(cases, 121);
    }

    #[test]
    fn test_node_role_matrix_121_cases() {
        let mut cases = 0;
        for upstream in 0..=10 {
            for downstream in 0..=10 {
                let a = analysis(upstream, downstream, upstream + downstream);
                let role = NodeRole::from_analysis(&a);
                let expected = match (upstream > 0, downstream > 0) {
                    (false, false) => NodeRole::Isolated,
                    (false, true) => NodeRole::EntryPoint,
                    (true, false) => NodeRole::Utility,
                    (true, true) => {
                        if (upstream <= 2 && downstream > 5) || (downstream <= 2 && upstream > 5)
                        {
                            NodeRole::Adapter
                        } else {
                            NodeRole::CoreLogic
                        }
                    }
                };

                assert_eq!(
                    role, expected,
                    "role mismatch for upstream={upstream}, downstream={downstream}"
                );
                cases += 1;
            }
        }
        assert_eq!(cases, 121);
    }

    #[test]
    fn test_confidence_standard_suggestion_always_present() {
        for (upstream, downstream, total) in [
            (0, 0, 0),
            (0, 8, 8),
            (12, 0, 12),
            (4, 4, 15),
            (4, 20, 30),
            (20, 20, 70),
        ] {
            let a = analysis(upstream, downstream, total);
            let explanation = ConfidenceExplanation::from_analysis(&a);
            assert!(
                explanation
                    .suggestions
                    .iter()
                    .any(|s| s.contains("Tests still recommended")),
                "missing standard suggestion for upstream={upstream}, downstream={downstream}, total={total}"
            );
        }
    }
}
