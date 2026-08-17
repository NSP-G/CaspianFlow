//! DAG topology — topological ordering and cycle detection.
//!
//! Builds a [`petgraph::DiGraph`] from a workflow's steps and `depends_on`
//! edges, then uses [`petgraph::algo::toposort`] for both cycle detection
//! and a valid execution order. Pure logic — no runtime or IO dependency,
//! so it is cheap enough to run on every execution (10-node workflow < 1ms).

use std::collections::HashMap;

use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};

use crate::types::{WorkflowError, WorkflowResult};
use crate::workflow::schema::Workflow;

/// The result of computing a workflow's DAG topology.
#[derive(Debug, Clone)]
pub struct TopologyResult {
    /// Step IDs in a valid topological execution order.
    pub order: Vec<String>,
    /// `step_id -> position in `order` (0-based).
    pub index: HashMap<String, usize>,
}

/// Build a `DiGraph` from a workflow and compute its topological order.
///
/// Edge direction is `dependency -> step` (one edge per `depends_on`
/// entry), yielding an order where every step appears after all of its
/// dependencies.
///
/// # Errors
///
/// - [`WorkflowError::MissingDependency`] if a step references an unknown
///   dependency id — detected before topo-sort with a precise message.
/// - [`WorkflowError::CycleDetected`] if the DAG contains a cycle.
pub fn compute_topology(workflow: &Workflow) -> WorkflowResult<TopologyResult> {
    let mut graph = DiGraph::<String, ()>::new();
    let mut node_indices: HashMap<String, NodeIndex> = HashMap::with_capacity(workflow.steps.len());

    // 1. One node per step.
    for step in &workflow.steps {
        let idx = graph.add_node(step.id.clone());
        node_indices.insert(step.id.clone(), idx);
    }

    // 2. One directed edge dependency -> step for each dependency.
    for step in &workflow.steps {
        if step.depends_on.is_empty() {
            continue;
        }
        let step_idx = node_indices[&step.id];
        for dep in &step.depends_on {
            let &dep_idx =
                node_indices
                    .get(dep)
                    .ok_or_else(|| WorkflowError::MissingDependency {
                        workflow_name: workflow.name.clone(),
                        step_id: step.id.clone(),
                        dependency: dep.clone(),
                    })?;
            graph.add_edge(dep_idx, step_idx, ());
        }
    }

    // 3. Topological sort (also detects cycles).
    let sorted = toposort(&graph, None).map_err(|cycle| {
        let node_id = graph[cycle.node_id()].clone();
        WorkflowError::CycleDetected {
            workflow_name: workflow.name.clone(),
            cycle: node_id,
        }
    })?;

    let order: Vec<String> = sorted.iter().map(|idx| graph[*idx].clone()).collect();
    let index: HashMap<String, usize> = order
        .iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), i))
        .collect();

    Ok(TopologyResult { order, index })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::schema::Workflow;
    use std::time::Instant;

    /// Build a minimal workflow from `(id, [dependencies])` pairs.
    fn workflow_with_steps(steps: &[(&str, &[&str])]) -> Workflow {
        let mut yaml = String::from("name: \"wf\"\nschema_version: \"1.0\"\nsteps:\n");
        for (id, deps) in steps {
            yaml.push_str(&format!("  - id: \"{id}\"\n    skill: \"noop\"\n"));
            if !deps.is_empty() {
                let deps_yaml: Vec<String> = deps.iter().map(|d| format!("\"{d}\"")).collect();
                yaml.push_str(&format!("    depends_on: [{}]\n", deps_yaml.join(", ")));
            }
        }
        Workflow::from_yaml(&yaml).unwrap()
    }

    #[test]
    fn test_topo_single_step() {
        let wf = workflow_with_steps(&[("only", &[])]);
        let topo = compute_topology(&wf).unwrap();
        assert_eq!(topo.order, vec!["only"]);
        assert_eq!(topo.index.get("only"), Some(&0));
    }

    #[test]
    fn test_topo_linear_chain() {
        // a -> b -> c
        let wf = workflow_with_steps(&[("a", &[]), ("b", &["a"]), ("c", &["b"])]);
        let topo = compute_topology(&wf).unwrap();
        assert_eq!(topo.order, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_topo_fork_join() {
        // a -> b, a -> c, d depends on [b, c]
        let wf =
            workflow_with_steps(&[("a", &[]), ("b", &["a"]), ("c", &["a"]), ("d", &["b", "c"])]);
        let topo = compute_topology(&wf).unwrap();
        // a must come first, d last; b and c are interchangeable in the middle.
        assert_eq!(topo.order.len(), 4);
        assert_eq!(topo.order[0], "a");
        assert_eq!(topo.order[3], "d");
        assert!(topo.index["b"] < topo.index["d"]);
        assert!(topo.index["c"] < topo.index["d"]);
        assert!(topo.index["a"] < topo.index["b"]);
        assert!(topo.index["a"] < topo.index["c"]);
    }

    #[test]
    fn test_topo_independent_steps() {
        let wf = workflow_with_steps(&[("x", &[]), ("y", &[]), ("z", &[])]);
        let topo = compute_topology(&wf).unwrap();
        assert_eq!(topo.order.len(), 3);
        // All three must be present (order may vary).
        assert!(topo.order.contains(&"x".to_string()));
        assert!(topo.order.contains(&"y".to_string()));
        assert!(topo.order.contains(&"z".to_string()));
    }

    #[test]
    fn test_topo_missing_dependency() {
        // b depends on a missing step "ghost"
        let wf = workflow_with_steps(&[("a", &[]), ("b", &["ghost"])]);
        let err = compute_topology(&wf).unwrap_err();
        match err {
            WorkflowError::MissingDependency {
                step_id,
                dependency,
                ..
            } => {
                assert_eq!(step_id, "b");
                assert_eq!(dependency, "ghost");
            }
            other => panic!("expected MissingDependency, got {other:?}"),
        }
    }

    #[test]
    fn test_topo_cycle_detected() {
        // a -> b -> c -> a
        let wf = workflow_with_steps(&[("a", &["c"]), ("b", &["a"]), ("c", &["b"])]);
        let err = compute_topology(&wf).unwrap_err();
        assert!(matches!(err, WorkflowError::CycleDetected { .. }));
    }

    #[test]
    fn test_topo_self_cycle() {
        // a depends on itself
        let wf = workflow_with_steps(&[("a", &["a"])]);
        let err = compute_topology(&wf).unwrap_err();
        assert!(matches!(err, WorkflowError::CycleDetected { .. }));
    }

    #[test]
    fn test_topo_performance_10_nodes_under_1ms() {
        // Build a 10-node linear chain: s0 -> s1 -> ... -> s9
        // Own the dependency lists first so the slices below outlive `steps`.
        let deps: Vec<Vec<&str>> = (0..10)
            .map(|i| if i == 0 { vec![] } else { vec![id_of(i - 1)] })
            .collect();
        let steps: Vec<(&str, &[&str])> = (0..10).map(|i| (id_of(i), deps[i].as_slice())).collect();
        let wf = workflow_with_steps(&steps);
        // Warm up, then assert each run is well under 1ms (100 iterations).
        for _ in 0..100 {
            let start = Instant::now();
            let topo = compute_topology(&wf).unwrap();
            let elapsed = start.elapsed().as_nanos();
            assert!(topo.order.len() == 10, "all 10 steps ordered");
            assert!(
                elapsed < 1_000_000,
                "toposort took {elapsed}ns, expected < 1ms"
            );
        }
    }

    /// Helper returning the id string for index `i` used above.
    fn id_of(i: usize) -> &'static str {
        match i {
            0 => "s0",
            1 => "s1",
            2 => "s2",
            3 => "s3",
            4 => "s4",
            5 => "s5",
            6 => "s6",
            7 => "s7",
            8 => "s8",
            _ => "s9",
        }
    }
}
