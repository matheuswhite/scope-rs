//! Guardrails for the release pipeline (#229 follow-up).
//!
//! Releasing is a maintainer-only action, and the machinery that enforces that
//! must not silently regress. These tests parse the GitHub Actions workflows
//! and assert the security invariants that keep a random pull request (or a
//! non-owner collaborator) from cutting a release:
//!
//!   * No workflow uses `pull_request_target` (the classic secret-exfiltration
//!     trigger: it runs with the base repo's secrets on PR-controlled code).
//!   * `publish-crates.yml` fires **only** on version tags — never on a branch
//!     push or a pull request — is guarded to the repository owner, deploys
//!     through the reviewer-gated `crates` environment, and refuses to publish
//!     a tag that doesn't match `Cargo.toml`.
//!   * The dist-generated `release.yml` never publishes on a pull request and
//!     isn't triggered by a branch push.
//!   * `version-guard.yml` still runs on pull requests.
//!   * `release-tag.yml` — which turns a merged release PR into the `vX.Y.Z`
//!     tag — is triggered by `push` and never by `pull_request` (so its own
//!     definition always comes from `main`, out of reach of a PR branch), is
//!     guarded to the repository owner, requires the merged PR's `release`
//!     label, and holds a read-only `GITHUB_TOKEN`.
//!
//! These are defense-in-depth: the root protections (tag protection ruleset,
//! branch protection on `main`, the `crates` environment reviewers) live in the
//! repo settings and are documented in CONTRIBUTING.md ("Release security").
//! But because these workflow files are what CI actually executes, pinning
//! their invariants here means any weakening shows up as a failing test.

use serde_yaml::Value;
use std::path::{Path, PathBuf};

fn workflows_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".github")
        .join("workflows")
}

fn read_workflow(name: &str) -> (String, Value) {
    let path = workflows_dir().join(name);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let value =
        serde_yaml::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
    (text, value)
}

fn workflow_files() -> Vec<PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(workflows_dir())
        .expect("workflows dir exists")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|s| s.to_str()),
                Some("yml") | Some("yaml")
            )
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "expected some workflow files");
    files
}

/// Look up a string key in a mapping, tolerating YAML 1.1 resolving the bare
/// key `on` to boolean `true` (the "Norway problem").
fn get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let map = value.as_mapping()?;
    map.iter()
        .find(|(k, _)| k.as_str() == Some(key) || (key == "on" && matches!(k, Value::Bool(true))))
        .map(|(_, v)| v)
}

fn has_key(value: &Value, key: &str) -> bool {
    get(value, key).is_some()
}

fn on_block(wf: &Value) -> &Value {
    get(wf, "on").expect("workflow has an `on:` trigger block")
}

#[test]
fn no_workflow_uses_pull_request_target() {
    for path in workflow_files() {
        let text = std::fs::read_to_string(&path).unwrap();
        let wf: Value = serde_yaml::from_str(&text).unwrap();
        assert!(
            !has_key(on_block(&wf), "pull_request_target"),
            "{} must not use `pull_request_target` (runs with base-repo secrets on PR-controlled code)",
            path.display()
        );
    }
}

#[test]
fn publish_crates_triggers_only_on_version_tags() {
    let (_text, wf) = read_workflow("publish-crates.yml");
    let on = on_block(&wf);

    assert!(
        !has_key(on, "pull_request") && !has_key(on, "pull_request_target"),
        "publish-crates must never run on a pull request"
    );

    let push = get(on, "push").expect("publish-crates triggers on push");
    assert!(
        !has_key(push, "branches"),
        "publish-crates must not trigger on branch pushes — only tags"
    );

    let tags = get(push, "tags").and_then(Value::as_sequence);
    let tags = tags.expect("publish-crates push.tags is a list");
    assert!(
        tags.iter()
            .filter_map(Value::as_str)
            .all(|p| p.starts_with('v')),
        "publish-crates tag patterns must be version tags (v...), got {tags:?}"
    );
}

#[test]
fn publish_crates_is_owner_guarded_and_reviewer_gated() {
    let (_text, wf) = read_workflow("publish-crates.yml");
    let publish = get(&wf, "jobs")
        .and_then(|jobs| get(jobs, "publish"))
        .expect("publish-crates has a `publish` job");

    // Deploys through the reviewer-gated environment.
    assert_eq!(
        get(publish, "environment").and_then(Value::as_str),
        Some("crates"),
        "publish job must deploy through the `crates` environment (required reviewers)"
    );

    // Guarded to the repository owner.
    let guard = get(publish, "if")
        .and_then(Value::as_str)
        .expect("publish job has an `if:` guard");
    assert!(
        guard.contains("github.actor") && guard.contains("github.repository_owner"),
        "publish job must be guarded to the repo owner; got if: {guard:?}"
    );
}

#[test]
fn publish_crates_verifies_tag_matches_cargo_version() {
    let (_text, wf) = read_workflow("publish-crates.yml");
    let steps = get(&wf, "jobs")
        .and_then(|jobs| get(jobs, "publish"))
        .and_then(|publish| get(publish, "steps"))
        .and_then(Value::as_sequence)
        .expect("publish job has steps");

    let verifies = steps.iter().any(|step| {
        get(step, "run")
            .and_then(Value::as_str)
            .is_some_and(|run| run.contains("cargo metadata") && run.contains("GITHUB_REF_NAME"))
    });
    assert!(
        verifies,
        "publish job must verify the tag matches the Cargo.toml version before publishing"
    );
}

#[test]
fn release_never_publishes_on_pull_request() {
    let (text, wf) = read_workflow("release.yml");
    let on = on_block(&wf);

    // dist runs `plan` on PRs, which is fine, but a branch push must not
    // trigger the release pipeline.
    if let Some(push) = get(on, "push") {
        assert!(
            !has_key(push, "branches"),
            "release.yml must not trigger the release pipeline on branch pushes"
        );
        assert!(has_key(push, "tags"), "release.yml should trigger on tags");
    }

    // The publish gate: dist only "publishes" when this isn't a pull request.
    assert!(
        text.contains("publishing: ${{ !github.event.pull_request }}"),
        "release.yml lost its `publishing: !github.event.pull_request` gate — \
         a pull request could publish. Re-check the dist-generated workflow."
    );
}

#[test]
fn release_tag_is_never_driven_by_a_pull_request() {
    let (_text, wf) = read_workflow("release-tag.yml");
    let on = on_block(&wf);

    // The whole point of the `push` trigger: for a push, GitHub takes the
    // workflow definition from the pushed branch (`main`), so a PR branch can't
    // rewrite this file and still get the tag-pushing token. A
    // `pull_request`-triggered run would use the PR's copy of the definition.
    assert!(
        !has_key(on, "pull_request") && !has_key(on, "pull_request_target"),
        "release-tag must not be triggered by a pull request — its definition \
         would then come from the PR branch, which can edit it"
    );

    let push = get(on, "push").expect("release-tag triggers on push");
    let branches = get(push, "branches")
        .and_then(Value::as_sequence)
        .expect("release-tag push.branches is a list");
    assert!(
        branches.iter().filter_map(Value::as_str).eq(["main"]),
        "release-tag must only tag pushes to main, got {branches:?}"
    );
}

#[test]
fn release_tag_is_owner_guarded_and_label_gated() {
    let (_text, wf) = read_workflow("release-tag.yml");

    // The workflow's own token must not be able to write anything; the tag is
    // pushed with the maintainer PAT instead.
    assert_eq!(
        get(&wf, "permissions")
            .and_then(|p| get(p, "contents"))
            .and_then(Value::as_str),
        Some("read"),
        "release-tag's GITHUB_TOKEN must stay read-only for contents"
    );

    let tag = get(&wf, "jobs")
        .and_then(|jobs| get(jobs, "tag"))
        .expect("release-tag has a `tag` job");

    let guard = get(tag, "if")
        .and_then(Value::as_str)
        .expect("tag job has an `if:` guard");
    assert!(
        guard.contains("github.actor") && guard.contains("github.repository_owner"),
        "tag job must be guarded to the repo owner; got if: {guard:?}"
    );

    assert_eq!(
        get(tag, "environment").and_then(Value::as_str),
        Some("release-tag"),
        "tag job must draw its PAT from the `release-tag` environment"
    );

    // The tag is only cut for a merged PR that carries the `release` label —
    // the same label version-guard demands before allowing the bump.
    let steps = get(tag, "steps")
        .and_then(Value::as_sequence)
        .expect("tag job has steps");
    let checks_label = steps.iter().any(|step| {
        get(step, "run")
            .and_then(Value::as_str)
            .is_some_and(|run| run.contains("/pulls") && run.contains("'release'"))
    });
    assert!(
        checks_label,
        "tag job must require the merged PR to carry the `release` label"
    );
}

#[test]
fn version_guard_runs_on_pull_requests() {
    let (_text, wf) = read_workflow("version-guard.yml");
    assert!(
        has_key(on_block(&wf), "pull_request"),
        "version-guard must run on pull requests to block version bumps"
    );
}
