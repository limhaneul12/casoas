use std::fs;

use tempfile::tempdir;

use super::*;

#[test]
fn existing_codegraph_one_hop_rejects_disjoint_writer_symbol_overlap() -> Result<()> {
    let workspace = tempdir()?;
    fs::create_dir(workspace.path().join(".codegraph"))?;
    let database = workspace.path().join(".codegraph/codegraph.db");
    let connection = rusqlite::Connection::open(&database)?;
    connection.execute_batch("CREATE TABLE nodes (id TEXT PRIMARY KEY,kind TEXT,name TEXT,qualified_name TEXT,file_path TEXT,start_line INTEGER,end_line INTEGER);CREATE TABLE edges (source TEXT,target TEXT);INSERT INTO nodes VALUES ('left','function','left','pkg.left','left.rs',1,3),('right','function','right','pkg.right','right.rs',1,3),('shared','function','shared','pkg.shared','common.rs',1,3);INSERT INTO edges VALUES ('left','shared'),('right','shared');")?;
    drop(connection);
    let make = |id: &str, dependencies: &[&str], write: bool| DagNode {
        id: id.to_owned(),
        title: id.to_owned(),
        task: id.to_owned(),
        role: NodeRole::InterfaceBuilder,
        node_kind: NodeKind::Standard,
        model: "claude-sonnet-5-5-medium".to_owned(),
        reasoning_effort: "medium".to_owned(),
        dependencies: dependencies.iter().map(|id| (*id).to_owned()).collect(),
        read_files: Vec::new(),
        write_files: if write {
            vec![PathBuf::from(format!("{id}.rs"))]
        } else {
            Vec::new()
        },
        symbols: vec![id.to_owned()],
        verify_command: None,
        reproduction_expected_exit_nonzero: false,
    };
    let nodes = vec![
        make("left", &[], true),
        make("right", &[], true),
        make("audit", &["left"], false),
        make("final", &["audit", "right"], false),
    ];
    assert!(DagPlanner::compile("symbol test", nodes.clone())?.is_executable());
    let plan = DagPlanner::compile_with_workspace("symbol test", nodes.clone(), workspace.path())?;
    assert!(
        plan.hazards
            .iter()
            .any(|hazard| hazard.kind == HazardKind::Waw
                && hazard.paths.is_empty()
                && hazard.message.contains("shared"))
    );
    let mut readers = nodes.clone();
    let mut qualified = nodes.clone();
    qualified[0].symbols = vec!["pkg.left".to_owned()];
    qualified[1].symbols = vec!["pkg.shared".to_owned()];
    assert!(
        !DagPlanner::compile_with_workspace("symbol test", qualified, workspace.path())?
            .is_executable()
    );
    readers[1].write_files.clear();
    assert!(
        DagPlanner::compile_with_workspace("symbol test", readers, workspace.path())?
            .is_executable()
    );
    let mut ordered = nodes;
    ordered[1].dependencies.push("left".to_owned());
    assert!(
        DagPlanner::compile_with_workspace("symbol test", ordered, workspace.path())?
            .is_executable()
    );
    let absent = tempdir()?;
    assert!(
        DagPlanner::compile_with_workspace("symbol test", plan.nodes, absent.path())?
            .is_executable()
    );
    assert!(!absent.path().join(".codegraph").exists());
    Ok(())
}

#[test]
fn ignored_subtrees_never_become_campaign_source() -> Result<()> {
    let workspace = tempdir()?;
    for directory in [
        "target",
        "node_modules",
        ".git",
        ".codegraph",
        ".venv",
        ".agents",
        ".omx",
        ".codex",
        ".hcodex",
    ] {
        let nested = workspace.path().join(directory).join("nested");
        fs::create_dir_all(&nested)?;
        fs::write(nested.join("generated.rs"), "fn generated() {}")?;
    }
    fs::create_dir(workspace.path().join("src"))?;
    fs::write(workspace.path().join("src/lib.rs"), "fn actual() {}")?;
    let plan = DagPlanner::generate("audit source", workspace.path())?;
    let sources: BTreeSet<_> = plan
        .nodes
        .iter()
        .flat_map(|node| &node.read_files)
        .collect();
    assert_eq!(sources, BTreeSet::from([&PathBuf::from("src/lib.rs")]));
    Ok(())
}

#[test]
fn explicit_goal_file_references_drive_generated_scope() -> Result<()> {
    let workspace = tempdir()?;
    fs::write(workspace.path().join("unrelated.rs"), "fn unrelated() {}")?;
    fs::create_dir(workspace.path().join("src"))?;
    fs::write(workspace.path().join("src/target.rs"), "fn target() {}")?;
    let plan = DagPlanner::generate("audit @src/target.rs", workspace.path())?;
    let files = plan
        .nodes
        .iter()
        .flat_map(|node| &node.read_files)
        .collect::<BTreeSet<_>>();
    assert_eq!(files, BTreeSet::from([&PathBuf::from("src/target.rs")]));
    let mut mentions = Vec::new();
    for index in 0..5 {
        let path = format!("explicit{index}.rs");
        fs::write(workspace.path().join(&path), "fn explicit() {}")?;
        mentions.push(format!("@{path}"));
    }
    assert!(
        DagPlanner::generate(&format!("audit {}", mentions.join(" ")), workspace.path()).is_err()
    );
    Ok(())
}

#[test]
fn generates_dynamic_multi_frontier_shapes() -> Result<()> {
    let workspace = tempdir()?;
    fs::write(workspace.path().join("a.rs"), "fn a() {}")?;
    fs::write(workspace.path().join("b.rs"), "fn b() {}")?;
    let normal = DagPlanner::generate("refactor both modules", workspace.path())?;
    assert_eq!(
        normal
            .waves
            .iter()
            .map(|wave| wave.node_ids.len())
            .collect::<Vec<_>>(),
        [2, 2, 1]
    );
    assert!(normal.is_executable());
    let audit = DagPlanner::generate("audit SQL security", workspace.path())?;
    assert_eq!(
        audit
            .waves
            .iter()
            .map(|wave| wave.node_ids.len())
            .collect::<Vec<_>>(),
        [1, 3, 1]
    );
    assert!(audit.is_executable());
    Ok(())
}

#[test]
fn source_structure_changes_node_count_within_contract_bounds() -> Result<()> {
    let one = tempdir()?;
    fs::write(one.path().join("a.rs"), "fn a() {}")?;
    let one_plan = DagPlanner::generate("refactor", one.path())?;
    assert_eq!(one_plan.nodes.len(), 4);

    let three = tempdir()?;
    for name in ["a.rs", "b.rs", "c.rs"] {
        fs::write(three.path().join(name), format!("fn {}() {{}}", &name[..1]))?;
    }
    let three_plan = DagPlanner::generate("refactor", three.path())?;
    assert_eq!(three_plan.nodes.len(), 7);

    let four = tempdir()?;
    for name in ["a.rs", "b.rs", "c.rs", "d.rs"] {
        fs::write(four.path().join(name), format!("fn {}() {{}}", &name[..1]))?;
    }
    let audit_plan = DagPlanner::generate("audit security", four.path())?;
    assert_eq!(audit_plan.nodes.len(), 8);
    assert!(
        [one_plan, three_plan, audit_plan]
            .iter()
            .all(|plan| plan.is_executable() && (4..=8).contains(&plan.nodes.len()))
    );
    Ok(())
}

#[test]
fn rejects_unknown_cycle_and_file_hazards() -> Result<()> {
    let base = |id: &str| DagNode {
        id: id.to_owned(),
        title: id.to_owned(),
        task: id.to_owned(),
        role: NodeRole::InterfaceBuilder,
        node_kind: NodeKind::Standard,
        model: "gpt-6.1-sol".to_owned(),
        reasoning_effort: "xhigh".to_owned(),
        dependencies: Vec::new(),
        read_files: Vec::new(),
        write_files: Vec::new(),
        symbols: Vec::new(),
        verify_command: None,
        reproduction_expected_exit_nonzero: true,
    };
    let mut nodes = vec![base("a"), base("b"), base("c"), base("d")];
    nodes[0].dependencies = vec!["b".to_owned()];
    nodes[1].dependencies = vec!["a".to_owned()];
    nodes[2].dependencies = vec!["missing".to_owned()];
    nodes[0].write_files = vec![PathBuf::from("same.rs")];
    nodes[3].write_files = vec![PathBuf::from("same.rs")];
    let plan = DagPlanner::compile("bad graph", nodes)?;
    let kinds: BTreeSet<HazardKind> = plan
        .hazards
        .iter()
        .map(|hazard| hazard.kind.clone())
        .collect();
    assert!(kinds.contains(&HazardKind::Cycle));
    assert!(kinds.contains(&HazardKind::UnknownDependency));
    assert!(kinds.contains(&HazardKind::Waw));
    Ok(())
}

#[test]
fn rejects_unordered_raw_and_war_hazards() -> Result<()> {
    let base = |id: &str| DagNode {
        id: id.to_owned(),
        title: id.to_owned(),
        task: id.to_owned(),
        role: NodeRole::InterfaceBuilder,
        node_kind: NodeKind::Standard,
        model: "gpt-6.1-sol".to_owned(),
        reasoning_effort: "xhigh".to_owned(),
        dependencies: Vec::new(),
        read_files: Vec::new(),
        write_files: Vec::new(),
        symbols: Vec::new(),
        verify_command: None,
        reproduction_expected_exit_nonzero: true,
    };
    let mut nodes = vec![base("a"), base("b"), base("c"), base("d")];
    nodes[0].write_files = vec![PathBuf::from("raw.rs")];
    nodes[1].read_files = vec![PathBuf::from("raw.rs")];
    nodes[2].read_files = vec![PathBuf::from("war.rs")];
    nodes[3].write_files = vec![PathBuf::from("war.rs")];
    let plan = DagPlanner::compile("unordered graph", nodes)?;
    let kinds: BTreeSet<HazardKind> = plan
        .hazards
        .iter()
        .map(|hazard| hazard.kind.clone())
        .collect();
    assert!(kinds.contains(&HazardKind::Raw));
    assert!(kinds.contains(&HazardKind::War));
    Ok(())
}

#[test]
fn refactor_plan_excludes_unrequested_sql_and_pairs_codex_with_antigravity() -> Result<()> {
    let workspace = tempdir()?;
    fs::write(workspace.path().join("alpha_query.sql"), "SELECT 1;")?;
    fs::write(workspace.path().join("beta.rs"), "pub fn beta() {}")?;
    fs::write(workspace.path().join("gamma.rs"), "pub fn gamma() {}")?;
    fs::write(workspace.path().join("delta.rs"), "pub fn delta() {}")?;
    let plan = DagPlanner::generate("refactor beta module", workspace.path())?;
    let all_files: BTreeSet<&PathBuf> = plan
        .nodes
        .iter()
        .flat_map(|node| node.read_files.iter().chain(&node.write_files))
        .collect();
    assert!(!all_files.contains(&PathBuf::from("alpha_query.sql")));
    let codex_lanes = plan
        .nodes
        .iter()
        .filter(|node| node.model.starts_with("gpt-6"))
        .count();
    let agy_lanes = plan
        .nodes
        .iter()
        .filter(|node| node.model.starts_with("gemini-") || node.model.starts_with("claude-"))
        .count();
    assert!(codex_lanes >= 2);
    assert!(agy_lanes >= 2);
    assert_eq!(plan.waves.len(), 4);
    assert!(
        plan.nodes
            .iter()
            .any(|node| node.model == "claude-opus-5-5-medium"),
        "4-wave OMH topology must include Claude Opus 5.5 medium pole"
    );
    assert!(
        plan.nodes
            .iter()
            .any(|node| node.model == "claude-sonnet-5-5-medium"),
        "4-wave OMH topology must include Claude Sonnet 5.5 medium pole"
    );
    Ok(())
}
