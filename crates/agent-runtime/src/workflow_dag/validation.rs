use anyhow::{anyhow, Result};
use std::collections::{HashMap, VecDeque};

use super::types::{WorkflowDefinition, WorkflowNode};

/// Validate the entire graph (structure, acyclicity, and typed edge contracts) before allowing any node to produce side effects.
pub fn validate_workflow(workflow: &WorkflowDefinition) -> Result<()> {
    if workflow.nodes.is_empty() {
        return Err(anyhow!("Workflow must contain at least one node"));
    }
    let mut degrees = HashMap::new();
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut node_map: HashMap<&str, &WorkflowNode> = HashMap::new();
    for node in &workflow.nodes {
        if node.id.trim().is_empty() || degrees.insert(node.id.as_str(), 0usize).is_some() {
            return Err(anyhow!("Empty or duplicate node ID: {}", node.id));
        }
        adjacency.insert(&node.id, Vec::new());
        node_map.insert(&node.id, node);
    }
    for edge in &workflow.edges {
        let Some(src_node) = node_map.get(edge.source.as_str()) else {
            return Err(anyhow!("Edge {} references an unknown node", edge.id));
        };
        let Some(dst_node) = node_map.get(edge.target.as_str()) else {
            return Err(anyhow!("Edge {} references an unknown node", edge.id));
        };
        if let (Some(out_contract), Some(in_contract)) = (
            src_node.data.output_contract.as_ref(),
            dst_node.data.input_contract.as_ref(),
        ) {
            if !out_contract.is_compatible_with(in_contract) {
                return Err(anyhow!(
                    "EdgeTypeMismatch on edge '{}' ({} -> {}): output {:?} is incompatible with required input {:?}",
                    edge.id,
                    edge.source,
                    edge.target,
                    out_contract,
                    in_contract
                ));
            }
        }
        *degrees
            .get_mut(edge.target.as_str())
            .ok_or_else(|| anyhow!("Target node {} not found in degrees map", edge.target))? += 1;
        adjacency
            .get_mut(edge.source.as_str())
            .ok_or_else(|| anyhow!("Source node {} not found in adjacency map", edge.source))?
            .push(&edge.target);
    }
    let mut queue: VecDeque<_> = degrees
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut visited = 0;
    while let Some(id) = queue.pop_front() {
        visited += 1;
        if let Some(neighbors) = adjacency.get(id) {
            for neighbor in neighbors {
                let degree = degrees.get_mut(neighbor).ok_or_else(|| {
                    anyhow!("Neighbor node {} not found in degrees map", neighbor)
                })?;
                *degree = degree.saturating_sub(1);
                if *degree == 0 {
                    queue.push_back(*neighbor);
                }
            }
        }
    }
    if visited != workflow.nodes.len() {
        return Err(anyhow!("Workflow contains a cycle"));
    }
    Ok(())
}
