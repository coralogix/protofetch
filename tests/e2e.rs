//! End-to-end fetch tests using file-backed fixtures.

mod infra;

use infra::{
    assert_output_contains, assert_output_excludes, run, run_locked, run_update_selected,
    run_update_selected_error, FetchResult,
};

/// Fetch a single dependency with one proto file and assert the output tree.
#[test]
fn single_file_dep() {
    let result = run("single_file_dep");

    assert_output_contains(&result, &["proto/hello.proto"]);
}

/// Two direct deps; repo2 has a transitive dep on repo1 at a different commit.
///
/// repo1 main (commit1): proto/v1.proto
/// repo1 v2   (commit2): proto/v1.proto + proto/v2.proto
/// repo2 main (commit1): proto/b.proto  +  protofetch.toml -> repo1@v2
///
/// Main manifest: repo1@main (commit1) + repo2@main.
/// Verifies which commit wins for the shared transitive dep.
#[test]
fn two_repos_transitive_dep() {
    let result = run("two_repos_transitive_dep");

    assert_output_contains(&result, &["proto/b.proto", "proto/v1.proto"]);
    assert_output_excludes(&result, &["proto/v2.proto"]);
}

/// repo1 is never listed in the main manifest. It only appears as a
/// transitive dep via repo2's own protofetch.toml. Its protos must still
/// end up in the output.
#[test]
fn transitive_dep_only() {
    let result = run("transitive_dep_only");

    assert_output_contains(&result, &["proto/a.proto", "proto/b.proto"]);
}

/// Circular module dependencies should not recurse forever during copy planning.
#[test]
fn circular_dependencies() {
    let result = run("circular_dependencies");

    assert_output_contains(&result, &["a.proto", "b.proto"]);
}

/// Circular dependencies with different dependency rules should still apply each rule.
#[test]
fn circular_dependencies_with_content_roots() {
    let result = run("circular_dependencies_with_content_roots");

    assert_output_contains(&result, &["bar.proto", "b.proto", "foo.proto"]);
}

/// Circular dependencies with a pruned back-edge should include imported files
/// from the repeated dependency even when they are outside its allow policies.
#[test]
fn circular_dependencies_with_prune() {
    let result = run("circular_dependencies_with_prune");

    assert_output_contains(&result, &["bar/bar.proto"]);
}

/// A pre-existing lock file pins repo1 to commit1. The branch has since
/// advanced to commit2. LockMode::Locked must use commit1 and not pull
/// in the new files.
#[test]
fn locked_mode_uses_pinned_commit() {
    let result = run_locked("locked_mode_uses_pinned_commit");

    assert_output_contains(&result, &["proto/v1.proto"]);
    assert_output_excludes(&result, &["proto/v2.proto"]);
}

/// allow_policies apply only to the dependency they are defined on.
/// With prune disabled, matching files from that dependency are included and
/// non-matching files are excluded, while transitive dependencies keep their own rules.
#[test]
fn allow_policies_apply_only_to_own_dependency() {
    let result = run("allow_policies_apply_only_to_own_dependency");

    assert_output_contains(
        &result,
        &[
            "public/service.proto",
            "public/child.proto",
            "internal/child.proto",
        ],
    );
    assert_output_excludes(&result, &["internal/admin.proto"]);
}

/// With prune enabled, allow_policies select the root protos from the dependency,
/// then protofetch includes those protos and their import tree, including files
/// outside the allow_policies and files from the dependency subtree.
#[test]
fn allow_policies_with_prune_include_import_tree() {
    let result = run("allow_policies_with_prune_include_import_tree");

    assert_output_contains(
        &result,
        &[
            "public/service.proto",
            "internal/common.proto",
            "shared.proto",
        ],
    );
    assert_output_excludes(&result, &["internal/admin.proto", "unused.proto"]);
}

/// With content_roots = ["api/proto"] files under api/proto/ appear without
/// that prefix in the output; files outside the root are not imported.
#[test]
fn content_roots() {
    let result = run("content_roots");

    assert_output_contains(&result, &["model.proto", "service.proto"]);
    assert_output_excludes(
        &result,
        &["api/proto/model.proto", "api/proto/service.proto"],
    );
}

/// deny_policies apply to the dependency subtree.
/// With prune disabled, matching protos from the dependency and its transitive
/// dependencies are excluded.
#[test]
fn deny_policies_apply_to_dependency_subtree() {
    let result = run("deny_policies_apply_to_dependency_subtree");

    assert_output_contains(&result, &["public/service.proto", "public/child.proto"]);
    assert_output_excludes(&result, &["internal/admin.proto", "internal/child.proto"]);
}

/// With prune enabled, deny_policies exclude matching protos and their dependencies.
#[test]
fn deny_policies_with_prune_exclude_matching_files_and_deps() {
    let result = run("deny_policies_with_prune_exclude_matching_files_and_deps");

    assert_output_contains(&result, &["public/service.proto", "shared/public.proto"]);
    assert_output_excludes(&result, &["internal/admin.proto", "shared/secret.proto"]);
}

/// `revision = "<hash>"` in the manifest pins to that exact commit.
/// After pinning to commit1 the branch advances to commit2;
/// only commit1's files must appear in the output.
#[test]
fn revision_pin() {
    let result = run("revision_pin");

    assert_output_contains(&result, &["proto/v1.proto"]);
    assert_output_excludes(&result, &["proto/v2.proto"]);
}

/// `transitive = true` on a dep makes it visible as a transitive dep for the
/// prune import-graph walk of *all other* deps, even if those deps do not list
/// it in their own `protofetch.toml`.
///
/// repo_a uses `prune = true` and its `a.proto` imports `shared.proto` from
/// repo_shared. repo_a has no `protofetch.toml`, so without `transitive = true`
/// on repo_shared the import could not be resolved. Unimported files from
/// repo_shared are not fetched directly.
#[test]
fn transitive_flag() {
    let result = run("transitive_flag");

    assert_output_contains(&result, &["a.proto", "shared.proto"]);
    assert_output_excludes(&result, &["unused.proto"]);
}

/// Root-declared `transitive = true` deps can satisfy prune import graph walks
/// across each other, even when those transitive deps import files from one
/// another and are not listed in nested `protofetch.toml` manifests.
///
/// Dependency graph:
///   root -> repo_a (prune=true)
///   root -> repo_b (transitive=true)
///   root -> repo_c (transitive=true)
///
/// Import graph:
///   repo_a/a.proto -> repo_b/b_1.proto
///   repo_b/b_1.proto -> repo_c/c_1.proto
///   repo_c/c_1.proto -> repo_b/b_2.proto
///   repo_b/b_2.proto -> repo_c/c_2.proto
#[test]
fn transitive_cross_dependencies() {
    let result = run("transitive_cross_dependencies");

    assert_output_contains(
        &result,
        &["b_1.proto", "b_2.proto", "c_1.proto", "c_2.proto"],
    );
}

/// Regex allow policy: `re://service` matches any path whose string
/// representation contains "service", using the full regex engine.
#[test]
fn regex_policy() {
    let result = run("regex_policy");

    assert_output_contains(&result, &["service.proto"]);
    assert_output_excludes(&result, &["internal.proto"]);
}

/// LockMode::Update with a pre-existing lock is a partial update:
/// deps already in the lock keep their pinned commit even if the branch
/// has advanced, while deps new to the manifest are resolved fresh.
///
/// Initial lock: repo1 @ commit1.
/// Branch advances to commit2.
/// Manifest adds repo2.
/// After update: repo1 still @ commit1, repo2 @ its head.
#[test]
fn partial_lock_update() {
    let result = run("partial_lock_update");

    assert_output_contains(&result, &["proto/b.proto", "proto/v1.proto"]);
    assert_output_excludes(&result, &["proto/v2.proto"]);
}

#[test]
fn selected_lock_update() {
    let result = run_update_selected("selected_lock_update", "repo1", None);

    assert_lockfile_dependency_commit(&result, "repo1", "<commit:main:2>");
    assert_lockfile_dependency_commit(&result, "repo2", "<commit:main:3>");
}

#[test]
fn selected_precise_lock_update() {
    let result = run_update_selected(
        "selected_precise_lock_update",
        "repo1",
        Some("<commit:main:2>"),
    );

    assert_lockfile_dependency_commit(&result, "repo1", "<commit:main:2>");
    assert_lockfile_dependency_commit(&result, "repo2", "<commit:main:4>");
}

#[test]
fn selected_precise_lock_update_rejects_invalid_commit_hash() {
    let error = run_update_selected_error(
        "selected_precise_revision_mismatch",
        "repo1",
        "not-a-commit",
    );

    assert!(
        error.contains("Invalid commit hash not-a-commit"),
        "unexpected error: {error}"
    );
}

#[test]
fn selected_precise_lock_update_rejects_missing_commit() {
    let commit = "1111111111111111111111111111111111111111";
    let error = run_update_selected_error("selected_precise_revision_mismatch", "repo1", commit);

    assert!(
        error.contains(&format!("Commit {commit} was not found")),
        "unexpected error: {error}"
    );
}

#[test]
fn selected_precise_lock_update_rejects_commit_from_another_branch() {
    let error = run_update_selected_error(
        "selected_precise_revision_mismatch",
        "repo1",
        "<commit:side:1>",
    );

    assert!(
        error.contains("does not belong to the branch main"),
        "unexpected error: {error}"
    );
}

#[test]
fn selected_precise_lock_update_rejects_revision_mismatch() {
    let error = run_update_selected_error(
        "selected_precise_revision_mismatch",
        "repo1",
        "<commit:main:2>",
    );

    assert!(
        error.contains("does not match revision"),
        "unexpected error: {error}"
    );
}

fn assert_lockfile_dependency_commit(result: &FetchResult, name: &str, commit: &str) {
    let snapshot = result.snapshot_lockfile();
    let dependency = snapshot
        .split("\n\n")
        .find(|dependency| {
            dependency.contains("[[dependencies]]")
                && dependency.contains(&format!("name = \"{name}\""))
        })
        .unwrap_or_else(|| panic!("expected lockfile dependency {name}\n\n{snapshot}"));

    assert!(
        dependency.contains(&format!("commit_hash = \"{commit}\"")),
        "expected {name} to be locked to {commit}\n\n{dependency}"
    );
}

/// When the same dep is declared with `prune = true` in the root manifest but also
/// appears as a dependency of another dep that does not use prune, all files from
/// the shared transitive dep are included, not just those reachable via the import
/// graph.
///
/// Dependency graph:
///   root -> dep_mixed (prune=true) -> dep_inner
///   root -> dep_ref   (no prune)   -> dep_mixed
///
/// dep_inner has `imported.proto` (imported by dep_mixed's service.proto) and
/// `extra.proto` (not imported by anything). Because dep_ref also depends on
/// dep_mixed without prune, the prune restriction on dep_mixed is lifted and all
/// of dep_inner's files must appear in the output.
#[test]
fn prune_mixed_true_and_false_rules() {
    let result = run("prune_mixed_true_and_false_rules");

    assert_output_contains(
        &result,
        &[
            "extra.proto",
            "imported.proto",
            "other.proto",
            "service.proto",
        ],
    );
}

/// When the same dep is declared as `transitive = true` in the root manifest AND
/// also listed as a normal dependency in another dep's protofetch.toml, the dep's
/// files must still appear in the output.
///
/// Dependency graph:
///   root -> dep_consumer (no prune) -> shared
///   root -> shared (transitive=true)
///
/// shared.proto must be in the output even though dep_consumer does not import it.
#[test]
fn transitive_flag_mixed_with_normal_transitive_dep() {
    let result = run("transitive_flag_mixed_with_normal_transitive_dep");

    assert_output_contains(&result, &["consumer.proto", "shared.proto"]);
}

/// When the same dep appears both directly in the root manifest and transitively
/// via another dep, allow/deny policy sets from all occurrences must be unioned.
///
/// - root declares `shared` with `allow_policies = ["from_root/*"]`
/// - foo's protofetch.toml declares `shared` with `allow_policies = ["from_foo/*"]`
/// - root also declares `foo`
///
/// Without the fix (issue #183) only `from_root/` files would appear.
/// With the fix both subtrees must be present (union semantics).
#[test]
fn allow_policies_merged_across_duplicate_deps() {
    let result = run("allow_policies_merged_across_duplicate_deps");

    assert_output_contains(
        &result,
        &[
            "foo.proto",
            "from_foo/model.proto",
            "from_root/service.proto",
        ],
    );
}

#[test]
fn duplicate_dep_keeps_content_roots_and_policies_coupled() {
    let result = run("duplicate_dep_keeps_content_roots_and_policies_coupled");

    assert_output_contains(
        &result,
        &["bar.proto", "consumer.proto", "nested/foo.proto"],
    );
}

#[test]
fn duplicate_dep_same_file_under_different_content_roots() {
    let result = run("duplicate_dep_same_file_under_different_content_roots");

    assert_output_contains(&result, &["consumer.proto", "nested/shared.proto"]);
}
