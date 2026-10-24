//! Load order from the dependency graph.

use std::collections::{BTreeMap, BTreeSet};

use nexus_api::{PluginId, PluginManifest};
use petgraph::algo::{tarjan_scc, toposort};
use petgraph::graph::{DiGraph, NodeIndex};

use crate::LoadError;

/// The outcome of resolving a set of valid, uniquely named manifests.
pub(crate) struct Resolution {
    /// Plugins to load, dependencies first.
    pub order: Vec<PluginId>,
    pub rejected: Vec<(PluginId, LoadError)>,
}

/// The ids a plugin must load after: its dependencies, plus `extends` for themes.
pub(crate) fn dependency_ids(manifest: &PluginManifest) -> Vec<(PluginId, semver::VersionReq, bool)> {
    let mut deps: Vec<_> =
        manifest.dependencies.iter().map(|d| (d.id.clone(), d.version.clone(), d.optional)).collect();
    if let Some(parent) = theme_parent(manifest) {
        deps.push((parent, semver::VersionReq::STAR, false));
    }
    deps
}

/// `[theme] extends = "..."`.
pub(crate) fn theme_parent(manifest: &PluginManifest) -> Option<PluginId> {
    let extends = manifest.extra.get("theme")?.get("extends")?.as_str()?;
    PluginId::new(extends).ok()
}

pub(crate) fn resolve(manifests: &[PluginManifest]) -> Resolution {
    let by_id: BTreeMap<&PluginId, &PluginManifest> = manifests.iter().map(|m| (&m.id, m)).collect();
    let mut rejected: BTreeMap<PluginId, LoadError> = BTreeMap::new();

    // Missing and wrong-version dependencies.
    for m in by_id.values() {
        for (dep, req, optional) in dependency_ids(m) {
            let error = match by_id.get(&dep) {
                None if optional => continue,
                None => LoadError::MissingDependency { dependency: dep },
                // An optional dependency in the wrong version is ignored, like a missing one.
                Some(found) if !req.matches(&found.version) && optional => continue,
                Some(found) if !req.matches(&found.version) => LoadError::DependencyVersion {
                    dependency: dep,
                    required: req.to_string(),
                    found: found.version.to_string(),
                },
                Some(_) => continue,
            };
            rejected.entry(m.id.clone()).or_insert(error);
            break;
        }
    }

    // Edges point from a dependency to the plugin that needs it. Nodes go in sorted by id, so
    // the order never depends on registration order.
    let mut graph: DiGraph<PluginId, ()> = DiGraph::new();
    let nodes: BTreeMap<PluginId, NodeIndex> =
        by_id.keys().map(|id| ((*id).clone(), graph.add_node((*id).clone()))).collect();
    for m in by_id.values() {
        for (dep, _, _) in dependency_ids(m) {
            if let Some(&from) = nodes.get(&dep) {
                graph.add_edge(from, nodes[&m.id], ());
            }
        }
    }

    // Cycles: every member of a strongly connected component with more than one node, or with
    // a self edge.
    for component in tarjan_scc(&graph) {
        let is_cycle = component.len() > 1 || graph.contains_edge(component[0], component[0]);
        if is_cycle {
            let members: BTreeSet<PluginId> = component.iter().map(|n| graph[*n].clone()).collect();
            let members: Vec<PluginId> = members.into_iter().collect();
            for member in &members {
                rejected.insert(member.clone(), LoadError::Cycle { members: members.clone() });
            }
        }
    }

    // A plugin whose required dependency is rejected is rejected too, until nothing changes.
    loop {
        let mut changed = false;
        for m in by_id.values() {
            if rejected.contains_key(&m.id) {
                continue;
            }
            let failed =
                dependency_ids(m).into_iter().find(|(dep, _, optional)| !optional && rejected.contains_key(dep));
            if let Some((dependency, _, _)) = failed {
                rejected.insert(m.id.clone(), LoadError::DependencyRejected { dependency });
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // What remains is acyclic.
    graph.retain_nodes(|g, n| !rejected.contains_key(&g[n]));
    let order =
        toposort(&graph, None).expect("cycles were removed above").into_iter().map(|n| graph[n].clone()).collect();

    Resolution { order, rejected: rejected.into_iter().collect() }
}
