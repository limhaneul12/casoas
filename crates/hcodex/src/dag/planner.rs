use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Component, Path, PathBuf};

use walkdir::WalkDir;

use crate::Result;

use super::{DagNode, DagPlan, DagWave, Hazard, HazardKind, NodeKind, NodeRole};

#[derive(Clone, Debug, Default)]
pub struct DagPlanner;

impl DagPlanner {
    pub fn generate(goal: &str, workspace: &Path) -> Result<DagPlan> {
        if goal.trim().is_empty() {
            return Err("DAG goal must not be empty".into());
        }
        let referenced =
            crate::context::SourceContextBuilder::new(workspace)?.referenced_paths(goal)?;
        if referenced.len() > 4 {
            return Err("generated DAG accepts at most four explicit file targets; provide explicit nodes for a wider declared scope".into());
        }
        let files = if referenced.is_empty() {
            discover_source_files(workspace, goal, 4)?
        } else {
            referenced
        };
        let audit_goal = contains_audit_intent(goal);
        let nodes = if audit_goal {
            audit_nodes(goal, &files)
        } else {
            refactor_nodes(goal, &files)
        };
        Self::compile_with_workspace(goal, nodes, workspace)
    }

    /// Add indexed one-hop symbol hazards without creating or updating an index.
    pub(crate) fn compile_with_workspace(
        goal: &str,
        nodes: Vec<DagNode>,
        workspace: &Path,
    ) -> Result<DagPlan> {
        let mut plan = Self::compile(goal, nodes)?;
        if plan.hazards.iter().any(|hazard| {
            !matches!(
                hazard.kind,
                HazardKind::Waw | HazardKind::Raw | HazardKind::War
            )
        }) {
            return Ok(plan);
        }
        let database = workspace.join(".codegraph/codegraph.db");
        if !database.is_file() {
            return Ok(plan);
        }
        let reader = crate::storage::CodegraphReader::open(&database)?;
        let mut expanded = BTreeMap::new();
        for node in &plan.nodes {
            let mut symbols = BTreeSet::new();
            for symbol in node
                .symbols
                .iter()
                .map(|symbol| symbol.trim())
                .filter(|symbol| !symbol.is_empty())
            {
                symbols.insert(symbol.to_owned());
                for indexed in reader.find_symbol(symbol)? {
                    symbols.insert(indexed.name);
                    symbols.insert(indexed.qualified_name);
                    symbols.extend(indexed.callers);
                    symbols.extend(indexed.callees);
                }
            }
            expanded.insert(node.id.as_str(), symbols);
        }
        for wave in &plan.waves {
            let writers = plan
                .nodes
                .iter()
                .filter(|node| wave.node_ids.contains(&node.id) && !node.write_files.is_empty())
                .collect::<Vec<_>>();
            for (index, left) in writers.iter().enumerate() {
                for right in &writers[index + 1..] {
                    let overlap = expanded[left.id.as_str()]
                        .intersection(&expanded[right.id.as_str()])
                        .cloned()
                        .collect::<Vec<_>>();
                    if !overlap.is_empty() {
                        plan.hazards.push(Hazard {
                            code: hazard_code(&HazardKind::Waw).to_owned(),
                            kind: HazardKind::Waw,
                            node_ids: vec![left.id.clone(), right.id.clone()],
                            paths: Vec::new(),
                            message: format!(
                                "CodeGraph 1-hop symbol write hazard between '{}' and '{}': {}",
                                left.id,
                                right.id,
                                overlap.join(", ")
                            ),
                        });
                    }
                }
            }
        }
        plan.blocking_hazards_count = plan.hazards.len();
        Ok(plan)
    }

    pub fn compile(goal: &str, nodes: Vec<DagNode>) -> Result<DagPlan> {
        let mut hazards = validate_basics(&nodes);
        let (waves, cycle) = partition_waves(&nodes);
        if cycle {
            hazards.push(Hazard {
                code: hazard_code(&HazardKind::Cycle).to_owned(),
                kind: HazardKind::Cycle,
                node_ids: nodes.iter().map(|node| node.id.clone()).collect(),
                paths: Vec::new(),
                message: "DAG contains a dependency cycle".to_owned(),
            });
        }
        if !cycle && !(2..=4).contains(&waves.len()) {
            hazards.push(Hazard {
                code: hazard_code(&HazardKind::WaveCountOutOfRange).to_owned(),
                kind: HazardKind::WaveCountOutOfRange,
                node_ids: Vec::new(),
                paths: Vec::new(),
                message: format!("DAG must contain 2-4 waves, found {}", waves.len()),
            });
        }
        hazards.extend(analyze_file_hazards(&nodes, &waves));
        hazards.sort_by(|a, b| {
            (&a.kind, &a.node_ids, &a.paths).cmp(&(&b.kind, &b.node_ids, &b.paths))
        });
        hazards.dedup();
        let total_nodes = nodes.len();
        let total_waves = waves.len();
        let blocking_hazards_count = hazards.len();
        let ascii_graph = render_ascii_graph(&waves);
        Ok(DagPlan {
            goal: goal.to_owned(),
            nodes,
            waves,
            hazards,
            total_nodes,
            total_waves,
            blocking_hazards_count,
            ascii_graph,
        })
    }
}

fn validate_basics(nodes: &[DagNode]) -> Vec<Hazard> {
    let mut hazards = Vec::new();
    let ids: BTreeSet<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
    if ids.len() != nodes.len() {
        hazards.push(Hazard {
            code: hazard_code(&HazardKind::DuplicateNode).to_owned(),
            kind: HazardKind::DuplicateNode,
            node_ids: nodes.iter().map(|node| node.id.clone()).collect(),
            paths: Vec::new(),
            message: "DAG node IDs must be unique".to_owned(),
        });
    }
    for node in nodes {
        let unknown: Vec<String> = node
            .dependencies
            .iter()
            .filter(|dependency| !ids.contains(dependency.as_str()))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            hazards.push(Hazard {
                code: hazard_code(&HazardKind::UnknownDependency).to_owned(),
                kind: HazardKind::UnknownDependency,
                node_ids: std::iter::once(node.id.clone()).chain(unknown).collect(),
                paths: Vec::new(),
                message: format!("node '{}' references unknown dependencies", node.id),
            });
        }
        let unsafe_paths: Vec<PathBuf> = node
            .read_files
            .iter()
            .chain(&node.write_files)
            .filter(|path| !safe_relative(path))
            .cloned()
            .collect();
        if !unsafe_paths.is_empty() {
            hazards.push(Hazard {
                code: hazard_code(&HazardKind::UnsafePath).to_owned(),
                kind: HazardKind::UnsafePath,
                node_ids: vec![node.id.clone()],
                paths: unsafe_paths,
                message: format!("node '{}' contains unsafe scope paths", node.id),
            });
        }
    }
    hazards
}

fn partition_waves(nodes: &[DagNode]) -> (Vec<DagWave>, bool) {
    let ids: BTreeSet<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
    let mut indegree: BTreeMap<&str, usize> = nodes
        .iter()
        .map(|node| {
            let known = node
                .dependencies
                .iter()
                .filter(|dependency| ids.contains(dependency.as_str()))
                .count();
            (node.id.as_str(), known)
        })
        .collect();
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for node in nodes {
        for dependency in &node.dependencies {
            if ids.contains(dependency.as_str()) {
                children.entry(dependency).or_default().push(&node.id);
            }
        }
    }
    let mut frontier: VecDeque<&str> = indegree
        .iter()
        .filter_map(|(id, count)| (*count == 0).then_some(*id))
        .collect();
    let mut waves = Vec::new();
    let mut visited = 0;
    while !frontier.is_empty() {
        let current: Vec<&str> = frontier.drain(..).collect();
        let mut next = BTreeSet::new();
        for id in &current {
            visited += 1;
            for child in children.get(id).into_iter().flatten() {
                if let Some(count) = indegree.get_mut(child) {
                    *count -= 1;
                    if *count == 0 {
                        next.insert(*child);
                    }
                }
            }
        }
        waves.push(DagWave {
            index: waves.len(),
            node_ids: current.into_iter().map(str::to_owned).collect(),
        });
        frontier.extend(next);
    }
    (waves, visited != nodes.len())
}

fn analyze_file_hazards(nodes: &[DagNode], waves: &[DagWave]) -> Vec<Hazard> {
    let ancestors = ancestor_map(nodes);
    let wave_by_node: BTreeMap<&str, usize> = waves
        .iter()
        .flat_map(|wave| {
            wave.node_ids
                .iter()
                .map(move |id| (id.as_str(), wave.index))
        })
        .collect();
    let mut hazards = Vec::new();
    for (index, left) in nodes.iter().enumerate() {
        let left_reads = normalized_paths(&left.read_files);
        let left_writes = normalized_paths(&left.write_files);
        for right in &nodes[index + 1..] {
            let right_reads = normalized_paths(&right.read_files);
            let right_writes = normalized_paths(&right.write_files);
            let ordered = ancestors
                .get(right.id.as_str())
                .is_some_and(|set| set.contains(left.id.as_str()))
                || ancestors
                    .get(left.id.as_str())
                    .is_some_and(|set| set.contains(right.id.as_str()));
            let same_wave =
                wave_by_node.get(left.id.as_str()) == wave_by_node.get(right.id.as_str());
            add_hazard(
                &mut hazards,
                HazardKind::Waw,
                left,
                right,
                &left_writes
                    .intersection(&right_writes)
                    .cloned()
                    .collect::<Vec<_>>(),
                same_wave || !ordered,
            );
            add_hazard(
                &mut hazards,
                HazardKind::Raw,
                left,
                right,
                &left_writes
                    .intersection(&right_reads)
                    .cloned()
                    .collect::<Vec<_>>(),
                !ordered,
            );
            add_hazard(
                &mut hazards,
                HazardKind::War,
                left,
                right,
                &left_reads
                    .intersection(&right_writes)
                    .cloned()
                    .collect::<Vec<_>>(),
                !ordered,
            );
        }
    }
    hazards
}

fn add_hazard(
    hazards: &mut Vec<Hazard>,
    kind: HazardKind,
    left: &DagNode,
    right: &DagNode,
    paths: &[PathBuf],
    active: bool,
) {
    if active && !paths.is_empty() {
        hazards.push(Hazard {
            code: hazard_code(&kind).to_owned(),
            message: format!(
                "unordered {kind:?} hazard between '{}' and '{}'",
                left.id, right.id
            ),
            kind,
            node_ids: vec![left.id.clone(), right.id.clone()],
            paths: paths.to_vec(),
        });
    }
}

fn ancestor_map(nodes: &[DagNode]) -> BTreeMap<&str, BTreeSet<&str>> {
    let by_id: BTreeMap<&str, &DagNode> =
        nodes.iter().map(|node| (node.id.as_str(), node)).collect();
    let mut result = BTreeMap::new();
    for node in nodes {
        let mut found = BTreeSet::new();
        let mut pending: Vec<&str> = node.dependencies.iter().map(String::as_str).collect();
        while let Some(dependency) = pending.pop() {
            if found.insert(dependency)
                && let Some(parent) = by_id.get(dependency)
            {
                pending.extend(parent.dependencies.iter().map(String::as_str));
            }
        }
        result.insert(node.id.as_str(), found);
    }
    result
}

fn normalized_paths(paths: &[PathBuf]) -> BTreeSet<PathBuf> {
    paths
        .iter()
        .filter(|path| safe_relative(path))
        .cloned()
        .collect()
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && !path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

fn discover_source_files(workspace: &Path, goal: &str, limit: usize) -> Result<Vec<PathBuf>> {
    let root = workspace.canonicalize()?;
    let mut inventory = Vec::new();
    for entry in WalkDir::new(&root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0 || !entry.file_type().is_dir() || !ignored_directory(entry.path())
        })
        .filter_map(|entry| entry.ok())
    {
        let path = entry.path();
        if entry.file_type().is_file() && is_source_file(path) {
            inventory.push(path.strip_prefix(&root)?.to_path_buf());
        }
    }
    let goal_lower = goal.to_lowercase();
    let goal_mentions_test = goal_lower.contains("test") || goal_lower.contains("테스트");
    let goal_mentions_sql = goal_lower.contains("sql") || goal_lower.contains("쿼리");
    inventory.sort_by(|left, right| {
        (
            is_config_file(left),
            is_sql_file(left) && !goal_mentions_sql,
            is_test_file(left) && !goal_mentions_test,
        )
            .cmp(&(
                is_config_file(right),
                is_sql_file(right) && !goal_mentions_sql,
                is_test_file(right) && !goal_mentions_test,
            ))
            .then_with(|| left.cmp(right))
    });
    let preferred: Vec<PathBuf> = inventory
        .iter()
        .filter(|path| {
            (!is_config_file(path) || goal.contains("Cargo.toml"))
                && (!is_sql_file(path) || goal_mentions_sql)
                && (!is_test_file(path) || goal_mentions_test)
        })
        .cloned()
        .collect();
    if !preferred.is_empty() && preferred.len() <= limit {
        return Ok(preferred);
    }
    if inventory.len() <= limit {
        return Ok(inventory);
    }
    if let Ok(relevance) = crate::context::discover_relevance(&root, goal, limit.saturating_mul(2))
        && relevance.selection_reason != "workspace_inventory_fallback_no_semantic_match"
    {
        let mut selected: Vec<PathBuf> = relevance
            .files
            .into_iter()
            .filter(|path| {
                (!is_config_file(path) || goal.contains("Cargo.toml"))
                    && (!is_sql_file(path) || goal_mentions_sql)
                    && (!is_test_file(path) || goal_mentions_test)
            })
            .take(limit)
            .collect();
        if !selected.is_empty() {
            for candidate in &inventory {
                if selected.len() >= limit.min(3) {
                    break;
                }
                if !is_config_file(candidate)
                    && (!is_sql_file(candidate) || goal_mentions_sql)
                    && (!is_test_file(candidate) || goal_mentions_test)
                    && !selected.contains(candidate)
                {
                    selected.push(candidate.clone());
                }
            }
            return Ok(selected);
        }
    }
    let mut fallback = if preferred.is_empty() {
        inventory
    } else {
        preferred
    };
    fallback.truncate(limit);
    Ok(fallback)
}

fn ignored_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name,
                ".git"
                    | ".codegraph"
                    | ".venv"
                    | ".agents"
                    | ".omx"
                    | ".codex"
                    | ".hcodex"
                    | "target"
                    | "node_modules"
            )
        })
}

fn is_config_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == "toml")
}

fn is_sql_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == "sql")
}

fn is_test_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "tests.rs" || name.ends_with("_test.rs"))
}

fn is_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext,
                "rs" | "py" | "ts" | "tsx" | "js" | "jsx" | "sql" | "toml"
            )
        })
}

fn contains_audit_intent(goal: &str) -> bool {
    let normalized = goal.to_lowercase();
    ["audit", "security", "sql", "보안", "감사", "검증"]
        .iter()
        .any(|needle| normalized.contains(needle))
}

fn refactor_nodes(goal: &str, files: &[PathBuf]) -> Vec<DagNode> {
    let effective_files = if files.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        files.to_vec()
    };
    if effective_files.len() >= 3 {
        return multi_wave_refactor_nodes(goal, files, &effective_files);
    }
    let start_count = effective_files.len().clamp(1, 3);
    let middle_count = if effective_files.len() == 1 {
        2
    } else {
        effective_files.len().min(4)
    };
    let mut nodes = Vec::new();
    let mut starts = Vec::new();
    for index in 0..start_count {
        let id = format!("n1_module_map_{}", index + 1);
        starts.push(id.clone());
        let source = effective_files[index % effective_files.len()].clone();
        let (title, role, model, effort) = match index {
            0 => (
                "Topology spec",
                NodeRole::TopologyPlanner,
                "gemini-4-argon-high",
                "high",
            ),
            1 => (
                "Boundary map",
                NodeRole::BoundaryMapper,
                "gpt-6-luna",
                "max",
            ),
            _ => (
                "Security audit",
                NodeRole::SecurityAuditor,
                "gpt-6-astra",
                "xhigh",
            ),
        };
        nodes.push(node(
            &id,
            title,
            goal,
            role,
            model,
            effort,
            vec![],
            if source == Path::new(".") {
                Vec::new()
            } else {
                vec![source]
            },
            vec![],
        ));
    }
    let start_refs: Vec<&str> = starts.iter().map(String::as_str).collect();
    let mut middle = Vec::new();
    for index in 0..middle_count {
        let id = format!("n2_module_craft_{}", index + 1);
        middle.push(id.clone());
        let target = effective_files[index % effective_files.len()].clone();
        let single_file_verifier = effective_files.len() == 1 && index == 1;
        let dependencies = if single_file_verifier {
            vec![middle[0].as_str()]
        } else {
            start_refs.clone()
        };
        let (title, role, model, effort, writes_target) = match index {
            0 => (
                "Interface craft",
                NodeRole::InterfaceBuilder,
                "claude-sonnet-5-5-medium",
                "medium",
                true,
            ),
            1 => (
                "Contract verify",
                NodeRole::ContractVerifier,
                "gpt-6.1-sol",
                "xhigh",
                false,
            ),
            2 => (
                "Architecture review",
                NodeRole::InterfaceBuilder,
                "claude-opus-5-5-medium",
                "medium",
                false,
            ),
            _ => (
                "Security audit",
                NodeRole::SecurityAuditor,
                "gpt-6-astra",
                "xhigh",
                false,
            ),
        };
        nodes.push(node(
            &id,
            title,
            goal,
            role,
            model,
            effort,
            dependencies,
            vec![target.clone()],
            if !writes_target || target == Path::new(".") {
                Vec::new()
            } else {
                vec![target]
            },
        ));
    }
    let middle_refs: Vec<&str> = if effective_files.len() == 1 {
        middle.last().map(String::as_str).into_iter().collect()
    } else {
        middle.iter().map(String::as_str).collect()
    };
    nodes.push(node(
        "n3_integration_fan_in",
        "Integration fan-in",
        goal,
        NodeRole::IntegrationFanIn,
        "gemini-4-argon-high",
        "high",
        middle_refs,
        files.to_vec(),
        vec![],
    ));
    nodes
}

fn multi_wave_refactor_nodes(
    goal: &str,
    files: &[PathBuf],
    effective_files: &[PathBuf],
) -> Vec<DagNode> {
    let f0 = effective_files[0].clone();
    let f1 = effective_files[1 % effective_files.len()].clone();
    let f2 = effective_files[2 % effective_files.len()].clone();
    let f3 = effective_files[3 % effective_files.len()].clone();
    let mut nodes = vec![
        node(
            "n1_module_map_1",
            "Topology spec",
            goal,
            NodeRole::TopologyPlanner,
            "gemini-4-argon-high",
            "high",
            vec![],
            vec![f0.clone()],
            vec![],
        ),
        node(
            "n1_module_map_2",
            "Boundary map",
            goal,
            NodeRole::BoundaryMapper,
            "gpt-6-luna",
            "max",
            vec![],
            vec![f1.clone()],
            vec![],
        ),
        node(
            "n2_arch_opus",
            "Architecture review",
            goal,
            NodeRole::InterfaceBuilder,
            "claude-opus-5-5-medium",
            "medium",
            vec!["n1_module_map_1", "n1_module_map_2"],
            vec![f2.clone()],
            vec![],
        ),
        node(
            "n2_contract_verify",
            "Contract verify",
            goal,
            NodeRole::ContractVerifier,
            "gpt-6.1-sol",
            "xhigh",
            vec!["n1_module_map_1", "n1_module_map_2"],
            vec![f3],
            vec![],
        ),
        node(
            "n3_module_craft_1",
            "Interface craft",
            goal,
            NodeRole::InterfaceBuilder,
            "claude-sonnet-5-5-medium",
            "medium",
            vec!["n2_arch_opus", "n2_contract_verify"],
            vec![f0.clone()],
            vec![f0],
        ),
    ];
    let mut wave3_ids = vec!["n3_module_craft_1"];
    if effective_files.len() >= 4 {
        nodes.push(node(
            "n3_module_craft_2",
            "Interface craft",
            goal,
            NodeRole::InterfaceBuilder,
            "claude-sonnet-5-5-medium",
            "medium",
            vec!["n2_arch_opus", "n2_contract_verify"],
            vec![f1.clone()],
            vec![f1],
        ));
        wave3_ids.push("n3_module_craft_2");
    }
    nodes.push(node(
        "n3_security_audit",
        "Security audit",
        goal,
        NodeRole::SecurityAuditor,
        "gpt-6-astra",
        "xhigh",
        vec!["n2_arch_opus", "n2_contract_verify"],
        vec![f2],
        vec![],
    ));
    wave3_ids.push("n3_security_audit");
    nodes.push(node(
        "n4_integration_fan_in",
        "Integration fan-in",
        goal,
        NodeRole::IntegrationFanIn,
        "gemini-4-argon-high",
        "high",
        wave3_ids,
        files.to_vec(),
        vec![],
    ));
    nodes
}

fn audit_nodes(goal: &str, files: &[PathBuf]) -> Vec<DagNode> {
    let reads = files.to_vec();
    let audit_count = match files.len() {
        0 | 1 => 2,
        2 => 3,
        3 => 4,
        _ => 6,
    };
    let mut nodes = vec![node(
        "n1_topology_spec",
        "Audit topology",
        goal,
        NodeRole::TopologyPlanner,
        "gemini-4-argon-high",
        "high",
        vec![],
        reads.clone(),
        vec![],
    )];
    let mut auditors = Vec::new();
    for index in 0..audit_count {
        let id = format!("n2_audit_lane_{}", index + 1);
        auditors.push(id.clone());
        let (title, role, model, effort) = match index % 3 {
            0 => (
                "Surgical inspection",
                NodeRole::InterfaceBuilder,
                "claude-sonnet-5-5-medium",
                "medium",
            ),
            1 => (
                "Contract gate",
                NodeRole::ContractVerifier,
                "gpt-6.1-sol",
                "xhigh",
            ),
            _ => (
                "Security audit",
                NodeRole::SecurityAuditor,
                "gpt-6-astra",
                "xhigh",
            ),
        };
        nodes.push(node(
            &id,
            title,
            goal,
            role,
            model,
            effort,
            vec!["n1_topology_spec"],
            reads.clone(),
            vec![],
        ));
    }
    let audit_refs: Vec<&str> = auditors.iter().map(String::as_str).collect();
    nodes.push(node(
        "n3_integration_fan_in",
        "Audit fan-in",
        goal,
        NodeRole::IntegrationFanIn,
        "gemini-4-argon-high",
        "high",
        audit_refs,
        reads,
        vec![],
    ));
    nodes
}

#[allow(clippy::too_many_arguments)]
fn node(
    id: &str,
    title: &str,
    goal: &str,
    role: NodeRole,
    model: &str,
    effort: &str,
    dependencies: Vec<&str>,
    read_files: Vec<PathBuf>,
    write_files: Vec<PathBuf>,
) -> DagNode {
    DagNode {
        id: id.to_owned(),
        title: title.to_owned(),
        task: goal.to_owned(),
        role,
        node_kind: NodeKind::Standard,
        model: model.to_owned(),
        reasoning_effort: effort.to_owned(),
        dependencies: dependencies.into_iter().map(str::to_owned).collect(),
        read_files,
        write_files,
        symbols: Vec::new(),
        verify_command: None,
        reproduction_expected_exit_nonzero: true,
    }
}

fn hazard_code(kind: &HazardKind) -> &'static str {
    match kind {
        HazardKind::DuplicateNode => "DAG_DUPLICATE_NODE_ID",
        HazardKind::UnknownDependency => "DAG_UNKNOWN_DEPENDENCY",
        HazardKind::Cycle => "DAG_CYCLE_DETECTED",
        HazardKind::UnsafePath => "DAG_INVALID_SCOPE_PATH",
        HazardKind::WaveCountOutOfRange => "DAG_WAVE_COUNT_OUT_OF_RANGE",
        HazardKind::NodeCountOutOfRange => "DAG_NODE_COUNT_OUT_OF_RANGE",
        HazardKind::Waw => "DAG_WAW_SCOPE_CONFLICT",
        HazardKind::Raw | HazardKind::War => "DAG_MISSING_RAW_EDGE",
    }
}

fn render_ascii_graph(waves: &[DagWave]) -> String {
    waves
        .iter()
        .map(|wave| format!("Wave {}: {}", wave.index, wave.node_ids.join(" | ")))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests;
