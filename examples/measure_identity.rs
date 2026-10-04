use knot::models::{NAMESPACE_KNOT, ParsedEntity};
use knot::pipeline::input::discover_files;
use knot::pipeline::parser::{ParseConfig, parse_files_stream};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use uuid::Uuid;

type EntityKey = (String, String, usize, usize, String, String);

fn measure_repo(repo_name: &str, dir_path: &Path) {
    let parse_cfg = ParseConfig {
        custom_queries_path: None,
        repo_name: repo_name.to_string(),
        include_config_files: true,
        repo_path: Some(dir_path.to_string_lossy().to_string()),
        ..Default::default()
    };

    let files = discover_files(dir_path.to_str().unwrap(), true).expect("discover_files failed");

    let parse_repo = || {
        let (tx, mut rx) = mpsc::channel::<ParsedEntity>(1024);
        let cpus = 16;
        let files_clone = files.clone();
        let cfg_clone = parse_cfg.clone();
        std::thread::spawn(move || {
            parse_files_stream(&files_clone, &cfg_clone, tx, cpus, None);
        });

        let mut map: BTreeMap<EntityKey, Vec<Uuid>> = BTreeMap::new();
        let mut all_entities = Vec::new();
        while let Some(entity) = rx.blocking_recv() {
            let key = (
                entity.file_path.clone(),
                entity.fqn.clone(),
                entity.start_line,
                entity.end_line,
                entity.kind.to_string(),
                entity.name.clone(),
            );
            map.entry(key).or_default().push(entity.uuid);
            all_entities.push(entity);
        }
        (map, all_entities)
    };

    let (map1, run1) = parse_repo();
    let (map2, _run2) = parse_repo();

    assert_eq!(map1.len(), map2.len());
    let mut diff_uuid_count = 0;
    for (key, uuids1) in &map1 {
        let uuids2 = &map2[key];
        assert_eq!(uuids1.len(), uuids2.len());
        for (u1, u2) in uuids1.iter().zip(uuids2.iter()) {
            if u1 != u2 {
                diff_uuid_count += 1;
            }
        }
    }

    let mut seen_uuids = HashSet::new();
    let mut collision_count = 0;
    for e in &run1 {
        if !seen_uuids.insert(e.uuid) {
            collision_count += 1;
        }
    }

    let mut changed_from_base_formula = 0;
    for e in &run1 {
        let legacy_identity = format!("{}:{}:{}:{}", e.repo_name, e.file_path, e.fqn, e.start_line);
        let legacy_uuid = Uuid::new_v5(&NAMESPACE_KNOT, legacy_identity.as_bytes());
        if e.uuid != legacy_uuid {
            changed_from_base_formula += 1;
        }
    }

    println!(
        "REPO {:12} | total_entities: {:5} | diff_uuid_across_runs: {} | collisions_in_run: {} | changed_from_base_formula: {}",
        repo_name,
        run1.len(),
        diff_uuid_count,
        collision_count,
        changed_from_base_formula
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let target_repo = args.get(1).map(|s| s.as_str());

    let home = std::env::var("HOME").unwrap();
    let snapshot_base = PathBuf::from(home).join(".knot/repos");

    println!("=== Measurement Report ===");
    for repo in &["knot", "knot-site", "spring-ai"] {
        if let Some(target) = target_repo
            && target != *repo
        {
            continue;
        }
        let repo_dir = snapshot_base.join(repo);
        if repo_dir.exists() {
            measure_repo(repo, &repo_dir);
        } else {
            println!("Repo directory not found: {:?}", repo_dir);
        }
    }
}
