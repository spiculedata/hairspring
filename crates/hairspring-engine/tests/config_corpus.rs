//! Every configuration the documentation shows must keep parsing to exactly the same targets.
//!
//! The golden file records, for each fenced `toml` block that defines targets, what `Config::parse` made of it
//! (serialised) or the error it gave. Choosing a backend by id must not change any of them. Regenerate the file only
//! on purpose: `UPDATE_GOLDEN=1 cargo test -p hairspring-engine --test config_corpus`.
use hairspring_engine::config::Config;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if !matches!(
                name,
                "internal" | "node_modules" | "target" | "build" | ".git"
            ) {
                markdown_files(&path, out);
            }
        } else if name.ends_with(".md") {
            out.push(path);
        }
    }
}

/// The text of each fenced `toml` block in `text` that defines a target.
fn target_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        match (&mut current, line.trim_start()) {
            (None, l) if l.starts_with("```toml") => current = Some(String::new()),
            (Some(_), l) if l.starts_with("```") => {
                let block = current.take().unwrap_or_default();
                if block.contains("[target.") {
                    blocks.push(block);
                }
            }
            (Some(block), _) => {
                block.push_str(line);
                block.push('\n');
            }
            _ => {}
        }
    }
    blocks
}

fn parse_line(text: &str) -> String {
    match Config::parse(text) {
        Ok(config) => format!("ok {}", serde_json::to_string(&config).expect("serialises")),
        Err(e) => format!("err {e}"),
    }
}

/// Configs written out here: the starter file's sim target and its commented process target with every key, and the
/// other target kinds.
const INLINE: [&str; 4] = [
    "[target.lock-lease-demo]\nkind = \"sim\"\nsim = \"lock-lease\"\ndescription = \"Lease-based lock\"\n\
     [target.lock-lease-demo.budget]\nmax_steps = 200000\nwall_secs = 60\n",
    "[target.my-service]\nkind = \"process\"\ncommand = [\"/opt/app/bin/service\", \"--test-mode\"]\n\
     env = { MODE = \"test\" }\ntrial_timeout_secs = 120\nhang_is_violation = true\nparallelism = 4\n\
     hermit_args = [\"--base-env=minimal\"]\npass_env = false\nchaos = true\nmax_timeslice_ns = 1000000\n\
     skid_margin = 25\n[target.my-service.budget]\nmax_trials = 100\nwall_secs = 600\n",
    "[target.sticky]\nkind = \"process\"\ncommand = [\"/bin/true\"]\nheuristic = \"sticky-random\"\nsticky_permille = 900\n",
    "[target.w]\nkind = \"wasm\"\nmodule = \"m.wasm\"\n",
];

fn corpus() -> String {
    let root = repo_root();
    let mut out = String::new();
    for (index, text) in INLINE.iter().enumerate() {
        let _ = writeln!(out, "inline#{index}\n{}", parse_line(text));
    }
    let mut files = vec![root.join("README.md")];
    for dir in ["docs", "examples", "integrations", "sdks"] {
        markdown_files(&root.join(dir), &mut files);
    }
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let relative = file
            .strip_prefix(&root)
            .unwrap_or(&file)
            .display()
            .to_string();
        for (index, block) in target_blocks(&text).iter().enumerate() {
            let _ = writeln!(out, "{relative}#{index}\n{}", parse_line(block));
        }
    }
    out
}

#[test]
fn documented_configs_parse_exactly_as_before() {
    let golden = repo_root().join("crates/hairspring-engine/tests/golden/config_corpus.txt");
    let now = corpus();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(golden.parent().expect("has a parent"))
            .expect("creates the directory");
        std::fs::write(&golden, &now).expect("writes the golden file");
        return;
    }
    let expected = std::fs::read_to_string(&golden).expect("the golden file exists");
    assert_eq!(
        now, expected,
        "a documented config no longer parses as it did"
    );
}

#[test]
fn the_corpus_is_not_empty_and_every_block_parses() {
    let now = corpus();
    let ok = now.lines().filter(|l| l.starts_with("ok ")).count();
    assert!(ok >= 9, "{now}");
    assert!(!now.lines().any(|l| l.starts_with("err ")), "{now}");
}
