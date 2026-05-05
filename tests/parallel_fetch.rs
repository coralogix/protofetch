//! End-to-end smoke test for the parallel fetch pipeline.
//!
//! `Coordinate`'s URL parser is opinionated about `forge/org/repo` shape and
//! always emits `https://` or `ssh://`, so we cannot point real fixtures at
//! `file://` URLs from a unit test. These tests therefore exercise the
//! runtime/wiring with empty dependency graphs, asserting that the tokio
//! runtime is built, the lock file is produced, and the proto output
//! directory is created. The behavior of the parallel resolver itself is
//! covered by `src/fetch/parallel.rs::tests`.

use std::fs;

use protofetch::{LockMode, Protofetch};
use tempfile::TempDir;

#[test]
fn empty_descriptor_runs_through_parallel_pipeline() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();

    fs::write(
        project.path().join("protofetch.toml"),
        r#"name = "smoke_test"
"#,
    )
    .unwrap();

    let pf = Protofetch::builder()
        .root(project.path().to_path_buf())
        .cache_directory(cache.path().to_path_buf())
        .jobs(8)
        .copy_jobs(4)
        .try_build()
        .unwrap();

    pf.fetch(LockMode::Update).unwrap();

    let lock = fs::read_to_string(project.path().join("protofetch.lock")).unwrap();
    assert!(lock.contains("version = 2"), "got lockfile: {}", lock);
    assert!(project.path().join("proto_src").is_dir());
}

#[test]
fn jobs_one_falls_back_to_sequential_behavior() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();

    fs::write(
        project.path().join("protofetch.toml"),
        r#"name = "smoke_test"
"#,
    )
    .unwrap();

    let pf = Protofetch::builder()
        .root(project.path().to_path_buf())
        .cache_directory(cache.path().to_path_buf())
        .jobs(1)
        .copy_jobs(1)
        .try_build()
        .unwrap();

    pf.fetch(LockMode::Update).unwrap();
    pf.lock(LockMode::Locked).unwrap();
}
