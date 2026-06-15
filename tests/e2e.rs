//! End-to-end fetch tests using the `file://` git protocol.

mod infra;

use indoc::indoc;
use infra::TestWorld;
use insta::assert_snapshot;
use protofetch::LockMode;
use toml::toml;

/// Fetch a single dependency with one proto file and assert the output tree.
#[test]
fn fetch_single_file_dep() {
    let mut world = TestWorld::new();

    world.create_repo(
        "org/repo1",
        &[(
            "proto/hello.proto",
            indoc! {r#"
                syntax = "proto3";
                message Hello {}
            "#},
        )],
    );

    let result = world.fetch(toml! {
        name = "e2e-test"

        [repo1]
        url = "org/repo1"
        branch = "main"
    });

    assert_snapshot!("single_file_dep_output", result.snapshot_tree());
    assert_snapshot!("single_file_dep_lockfile", result.snapshot_lockfile());
}

/// Two direct deps; repo2 has a transitive dep on repo1 at a different commit.
///
/// repo1 main (commit1): proto/v1.proto
/// repo1 v2   (commit2): proto/v1.proto + proto/v2.proto
/// repo2 main (commit1): proto/b.proto  +  protofetch.toml → repo1@v2
///
/// Main manifest: repo1@main (commit1) + repo2@main.
/// Verifies which commit wins for the shared transitive dep.
#[test]
fn fetch_two_repos_transitive_dep() {
    let mut world = TestWorld::new();

    // v2 branch branches from commit1, adds v2.proto (so it has both v1 and v2)
    world
        .create_repo(
            "org/repo1",
            &[(
                "proto/v1.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message V1 {}
                "#},
            )],
        )
        .add_commit(
            "v2",
            &[(
                "proto/v2.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message V2 {}
                "#},
            )],
        );

    world.create_repo(
        "org/repo2",
        &[
            (
                "protofetch.toml",
                indoc! {r#"
                    name = "repo2"

                    [repo1]
                    url = "<base>/org/repo1"
                    protocol = "file"
                    branch = "v2"
                "#},
            ),
            (
                "proto/b.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message B {}
                "#},
            ),
        ],
    );

    let result = world.fetch(toml! {
        name = "e2e-test"

        [repo1]
        url = "org/repo1"
        branch = "main"

        [repo2]
        url = "org/repo2"
        branch = "main"
    });

    assert_snapshot!("two_repos_output", result.snapshot_tree());
    assert_snapshot!("two_repos_lockfile", result.snapshot_lockfile());
}

/// repo1 is never listed in the main manifest — it only appears as a
/// transitive dep via repo2's own protofetch.toml.  Its protos must still
/// end up in the output.
#[test]
fn fetch_transitive_dep_only() {
    let mut world = TestWorld::new();

    world.create_repo(
        "org/repo1",
        &[(
            "proto/a.proto",
            indoc! {r#"
                syntax = "proto3";
                message A {}
            "#},
        )],
    );

    world.create_repo(
        "org/repo2",
        &[
            (
                "protofetch.toml",
                indoc! {r#"
                    name = "repo2"

                    [repo1]
                    url = "<base>/org/repo1"
                    protocol = "file"
                    branch = "main"
                "#},
            ),
            (
                "proto/b.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message B {}
                "#},
            ),
        ],
    );

    let result = world.fetch(toml! {
        name = "e2e-test"

        [repo2]
        url = "org/repo2"
        branch = "main"
    });

    assert_snapshot!("transitive_only_output", result.snapshot_tree());
    assert_snapshot!("transitive_only_lockfile", result.snapshot_lockfile());
}

/// A pre-existing lock file pins repo1 to commit1.  The branch has since
/// advanced to commit2.  LockMode::Locked must use commit1 and not pull
/// in the new files.
#[test]
fn fetch_locked_mode_uses_pinned_commit() {
    let mut world = TestWorld::new();

    world
        .create_repo(
            "org/repo1",
            &[(
                "proto/v1.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message V1 {}
                "#},
            )],
        )
        .add_commit(
            "main",
            &[(
                "proto/v2.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message V2 {}
                "#},
            )],
        );

    let result = world.fetch_with_initial_lock(
        toml! {
            name = "e2e-test"

            [repo1]
            url = "org/repo1"
            branch = "main"
        },
        LockMode::Locked,
        toml! {
            version = 2

            [[dependencies]]
            name = "repo1"
            url = "<base>/org/repo1"
            protocol = "file"
            branch = "main"
            commit_hash = "<commit:main:1>"
        },
    );

    // Output must contain only v1.proto — locked to commit1
    assert_snapshot!("locked_mode_output", result.snapshot_tree());
    assert_snapshot!("locked_mode_lockfile", result.snapshot_lockfile());
}

/// Only files under directories matching allow_policies are copied.
/// `allow_policies = ["public/*"]` is a Prefix policy: includes everything
/// under `public/` and excludes everything else.
#[test]
fn fetch_allow_policies() {
    let mut world = TestWorld::new();

    world.create_repo(
        "org/repo1",
        &[
            (
                "public/service.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Service {}
                "#},
            ),
            (
                "internal/admin.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Admin {}
                "#},
            ),
        ],
    );

    let result = world.fetch(toml! {
        name = "e2e-test"

        [repo1]
        url = "org/repo1"
        branch = "main"
        allow_policies = ["public/*"]
    });

    // Only public/service.proto; internal/admin.proto is excluded
    assert_snapshot!("allow_policies_output", result.snapshot_tree());
}

/// With content_roots = ["api/proto"] files under api/proto/ appear without
/// that prefix in the output; files outside the root keep their original path.
#[test]
fn fetch_content_roots() {
    let mut world = TestWorld::new();

    world.create_repo(
        "org/repo1",
        &[
            (
                "api/proto/service.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Service {}
                "#},
            ),
            (
                "api/proto/model.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Model {}
                "#},
            ),
            (
                "internal/secret.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Secret {}
                "#},
            ),
        ],
    );

    let result = world.fetch(toml! {
        name = "e2e-test"

        [repo1]
        url = "org/repo1"
        branch = "main"
        content_roots = ["api/proto"]
    });

    // service.proto and model.proto appear without the api/proto prefix;
    // internal/secret.proto keeps its original path
    assert_snapshot!("content_roots_output", result.snapshot_tree());
}

/// deny_policies is the mirror of allow_policies: matching files are excluded.
/// `deny_policies = ["internal/*"]` removes everything under internal/.
#[test]
fn fetch_deny_policies() {
    let mut world = TestWorld::new();

    world.create_repo(
        "org/repo1",
        &[
            (
                "public/service.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Service {}
                "#},
            ),
            (
                "internal/admin.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Admin {}
                "#},
            ),
        ],
    );

    let result = world.fetch(toml! {
        name = "e2e-test"

        [repo1]
        url = "org/repo1"
        branch = "main"
        deny_policies = ["internal/*"]
    });

    // internal/admin.proto excluded; public/service.proto kept
    assert_snapshot!("deny_policies_output", result.snapshot_tree());
}

/// prune = true walks the import graph starting from the dep's own files and
/// only includes transitive-dep files that are actually imported.  extra.proto
/// in repo1 is never imported so it must be absent from the output.
#[test]
fn fetch_prune() {
    let mut world = TestWorld::new();

    // repo1: two files; only imported.proto is imported by repo2
    world.create_repo(
        "org/repo1",
        &[
            (
                "imported.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Imported {}
                "#},
            ),
            (
                "extra.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Extra {}
                "#},
            ),
        ],
    );

    world.create_repo(
        "org/repo2",
        &[
            (
                "protofetch.toml",
                indoc! {r#"
                    name = "repo2"

                    [repo1]
                    url = "<base>/org/repo1"
                    protocol = "file"
                    branch = "main"
                "#},
            ),
            (
                "service.proto",
                indoc! {r#"
                    syntax = "proto3";
                    import "imported.proto";
                    message Service {}
                "#},
            ),
        ],
    );

    let result = world.fetch(toml! {
        name = "e2e-test"

        [repo2]
        url = "org/repo2"
        branch = "main"
        prune = true
    });

    // service.proto + imported.proto are in the import chain; extra.proto is not
    assert_snapshot!("prune_output", result.snapshot_tree());
}

/// `revision = "<hash>"` in the manifest pins to that exact commit.
/// After pinning to commit1 the branch advances to commit2;
/// only commit1's files must appear in the output.
#[test]
fn fetch_revision_pin() {
    let mut world = TestWorld::new();

    world
        .create_repo(
            "org/repo1",
            &[(
                "proto/v1.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message V1 {}
                "#},
            )],
        )
        .add_commit(
            "main",
            &[(
                "proto/v2.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message V2 {}
                "#},
            )],
        );

    let result = world.fetch(toml! {
        name = "e2e-test"

        [repo1]
        url = "org/repo1"
        revision = "<commit:main:1>"
    });

    // Only v1.proto — pinned to commit1 before v2.proto was added
    assert_snapshot!("revision_pin_output", result.snapshot_tree());
    assert_snapshot!("revision_pin_lockfile", result.snapshot_lockfile());
}

/// `transitive = true` on a dep makes it visible as a transitive dep for the
/// prune import-graph walk of *all other* deps, even if those deps do not list
/// it in their own `protofetch.toml`.
///
/// repo_a uses `prune = true` and its `a.proto` imports `shared.proto` from
/// repo_shared.  repo_a has no `protofetch.toml`, so without `transitive = true`
/// on repo_shared the import could not be resolved.
#[test]
fn fetch_transitive_flag() {
    let mut world = TestWorld::new();

    world.create_repo(
        "org/repo_shared",
        &[(
            "shared.proto",
            indoc! {r#"
                syntax = "proto3";
                message Shared {}
            "#},
        )],
    );

    world.create_repo(
        "org/repo_a",
        &[(
            "a.proto",
            indoc! {r#"
                syntax = "proto3";
                import "shared.proto";
                message A {}
            "#},
        )],
    );

    let result = world.fetch(toml! {
        name = "e2e-test"

        [repo_a]
        url = "org/repo_a"
        branch = "main"
        prune = true

        [repo_shared]
        url = "org/repo_shared"
        branch = "main"
        transitive = true
    });

    assert_snapshot!("transitive_flag_output", result.snapshot_tree());
    assert_snapshot!("transitive_flag_lockfile", result.snapshot_lockfile());
}

/// Regex allow policy: `re://service` matches any path whose string
/// representation contains "service", using the full regex engine.
#[test]
fn fetch_regex_policy() {
    let mut world = TestWorld::new();

    world.create_repo(
        "org/repo1",
        &[
            (
                "service.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Service {}
                "#},
            ),
            (
                "internal.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Internal {}
                "#},
            ),
        ],
    );

    let result = world.fetch(toml! {
        name = "e2e-test"

        [repo1]
        url = "org/repo1"
        branch = "main"
        allow_policies = ["re://service"]
    });

    // Only service.proto matches the regex; internal.proto is excluded
    assert_snapshot!("regex_policy_output", result.snapshot_tree());
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
fn fetch_partial_lock_update() {
    let mut world = TestWorld::new();

    world
        .create_repo(
            "org/repo1",
            &[(
                "proto/v1.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message V1 {}
                "#},
            )],
        )
        .add_commit(
            "main",
            &[(
                "proto/v2.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message V2 {}
                "#},
            )],
        );

    world.create_repo(
        "org/repo2",
        &[(
            "proto/b.proto",
            indoc! {r#"
                syntax = "proto3";
                message B {}
            "#},
        )],
    );

    let result = world.fetch_with_initial_lock(
        toml! {
            name = "e2e-test"

            [repo1]
            url = "org/repo1"
            branch = "main"

            [repo2]
            url = "org/repo2"
            branch = "main"
        },
        LockMode::Update,
        toml! {
            version = 2

            [[dependencies]]
            name = "repo1"
            url = "<base>/org/repo1"
            protocol = "file"
            branch = "main"
            commit_hash = "<commit:main:1>"
        },
    );

    // repo1 stays at commit1 (v1.proto only); repo2 resolved to its head (b.proto)
    assert_snapshot!("partial_update_output", result.snapshot_tree());
    assert_snapshot!("partial_update_lockfile", result.snapshot_lockfile());
}

/// When the same dep is declared with `prune = true` in the root manifest but also
/// appears as a dependency of another dep that does not use prune, all files from
/// the shared transitive dep are included — not just those reachable via the import
/// graph.
///
/// Dependency graph:
///   root → dep_mixed (prune=true) → dep_inner
///   root → dep_ref   (no prune)   → dep_mixed
///
/// dep_inner has `imported.proto` (imported by dep_mixed's service.proto) and
/// `extra.proto` (not imported by anything).  Because dep_ref also depends on
/// dep_mixed without prune, the prune restriction on dep_mixed is lifted and all
/// of dep_inner's files must appear in the output.
#[test]
fn fetch_prune_mixed_true_and_false_rules() {
    let mut world = TestWorld::new();

    world.create_repo(
        "org/dep_inner",
        &[
            (
                "imported.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Imported {}
                "#},
            ),
            (
                "extra.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Extra {}
                "#},
            ),
        ],
    );

    world.create_repo(
        "org/dep_mixed",
        &[
            (
                "protofetch.toml",
                indoc! {r#"
                    name = "dep_mixed"

                    [dep_inner]
                    url = "<base>/org/dep_inner"
                    protocol = "file"
                    branch = "main"
                "#},
            ),
            (
                "service.proto",
                indoc! {r#"
                    syntax = "proto3";
                    import "imported.proto";
                    message Service {}
                "#},
            ),
        ],
    );

    // dep_ref depends on dep_mixed without prune, lifting the prune restriction.
    world.create_repo(
        "org/dep_ref",
        &[
            (
                "protofetch.toml",
                indoc! {r#"
                    name = "dep_ref"

                    [dep_mixed]
                    url = "<base>/org/dep_mixed"
                    protocol = "file"
                    branch = "main"
                "#},
            ),
            (
                "other.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Other {}
                "#},
            ),
        ],
    );

    let result = world.fetch(toml! {
        name = "e2e-test"

        [dep_mixed]
        url = "org/dep_mixed"
        branch = "main"
        prune = true

        [dep_ref]
        url = "org/dep_ref"
        branch = "main"
    });

    // extra.proto must appear: the prune restriction on dep_mixed is lifted by
    // dep_ref's non-pruned reference, so all of dep_inner's files are included.
    assert_snapshot!("prune_mixed_rules_output", result.snapshot_tree());
}

/// When the same dep is declared as `transitive = true` in the root manifest AND
/// also listed as a normal dependency in another dep's protofetch.toml, the dep's
/// files must still appear in the output.
///
/// Dependency graph:
///   root → dep_consumer (no prune) → shared
///   root → shared (transitive=true)
///
/// shared.proto must be in the output even though dep_consumer does not import it.
#[test]
fn fetch_transitive_flag_mixed_with_normal_transitive_dep() {
    let mut world = TestWorld::new();

    world.create_repo(
        "org/shared",
        &[(
            "shared.proto",
            indoc! {r#"
                syntax = "proto3";
                message Shared {}
            "#},
        )],
    );

    world.create_repo(
        "org/dep_consumer",
        &[
            (
                "protofetch.toml",
                indoc! {r#"
                    name = "dep_consumer"

                    [shared]
                    url = "<base>/org/shared"
                    protocol = "file"
                    branch = "main"
                "#},
            ),
            (
                "consumer.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Consumer {}
                "#},
            ),
        ],
    );

    let result = world.fetch(toml! {
        name = "e2e-test"

        [dep_consumer]
        url = "org/dep_consumer"
        branch = "main"

        [shared]
        url = "org/shared"
        branch = "main"
        transitive = true
    });

    assert_snapshot!("transitive_mixed_rules_output", result.snapshot_tree());
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
fn fetch_allow_policies_merged_across_duplicate_deps() {
    let mut world = TestWorld::new();

    world.create_repo(
        "org/shared",
        &[
            (
                "from_root/service.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Service {}
                "#},
            ),
            (
                "from_foo/model.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Model {}
                "#},
            ),
        ],
    );

    world.create_repo(
        "org/foo",
        &[
            (
                "protofetch.toml",
                indoc! {r#"
                    name = "foo"

                    [shared]
                    url = "<base>/org/shared"
                    protocol = "file"
                    branch = "main"
                    allow_policies = ["from_foo/*"]
                "#},
            ),
            (
                "foo.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Foo {}
                "#},
            ),
        ],
    );

    let result = world.fetch(toml! {
        name = "e2e-test"

        [shared]
        url = "org/shared"
        branch = "main"
        allow_policies = ["from_root/*"]

        [foo]
        url = "org/foo"
        branch = "main"
    });

    // Both subtrees must be present: from_root/ (root's policy) and from_foo/ (foo's policy)
    assert_snapshot!("allow_policies_merged_output", result.snapshot_tree());
}

#[test]
fn fetch_duplicate_dep_keeps_content_roots_and_policies_coupled() {
    let mut world = TestWorld::new();

    world.create_repo(
        "org/shared",
        &[
            (
                "root/nested/foo.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Foo {}
                "#},
            ),
            (
                "root/nested/bar.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Bar {}
                "#},
            ),
        ],
    );

    world.create_repo(
        "org/consumer",
        &[
            (
                "protofetch.toml",
                indoc! {r#"
                    name = "consumer"

                    [shared]
                    url = "<base>/org/shared"
                    protocol = "file"
                    branch = "main"
                    content_roots = ["root/nested"]
                    allow_policies = ["bar.proto"]
                "#},
            ),
            (
                "consumer.proto",
                indoc! {r#"
                    syntax = "proto3";
                    message Consumer {}
                "#},
            ),
        ],
    );

    let result = world.fetch(toml! {
        name = "e2e-test"

        [shared]
        url = "org/shared"
        branch = "main"
        content_roots = ["root"]
        allow_policies = ["nested/foo.proto"]

        [consumer]
        url = "org/consumer"
        branch = "main"
    });

    assert_snapshot!(
        "duplicate_dep_content_roots_and_policies_output",
        result.snapshot_tree()
    );
}
