---
name: land
description: >-
  Land the current Scope repository change after the user explicitly requests
  landing it. Push the current branch, open or update a pull request against
  main, wait for all required CI checks and reviews, repair failures with new
  commits, merge the pull request, and verify the resulting main branch.
disable-model-invocation: true
metadata:
  delta-action: land
---

# Land Scope changes

Use this workflow only when the user explicitly requests that the current
change be landed. Do not treat review approval, passing local checks, a pushed
branch, or pull-request preparation as permission to merge.

## Preconditions

1. Identify the current branch and repository:

   ```sh
   git branch --show-current
   git status --short --branch
   git remote -v
   ```

   These commands inspect the Git state and remotes used by the repository
   (`CONTRIBUTING.md`, "Opening a pull request").
2. The destination branch is `main`. Do not land directly on `main`; the
   project requires a pull request (`CONTRIBUTING.md`, "Opening a pull
   request").
3. Stop and ask the user if the worktree contains uncommitted changes,
   unrelated changes, an unknown branch state, or no suitable `origin` remote.
   Do not discard or silently include unrelated work.
4. Check whether the current branch already has a pull request. Reuse it when
   it targets `main`; otherwise create one after pushing the branch:

   ```sh
   gh pr list --head "<branch>" --base main --state open
   gh pr view "<number>" --json baseRefName,headRefName,state,url
   ```

   `gh` is the repository's authenticated GitHub CLI; the remote and PR
   workflow are documented in `CONTRIBUTING.md`.

## Local verification

Run the repository's documented pre-PR checks before opening or updating the
pull request:

```sh
cargo fmt --all -- --check
cargo test --locked
cargo build --locked --release
```

These exact commands are required by `CONTRIBUTING.md`, "Testing and the
quality checklist". If any command fails, fix the cause in a new conventional
commit, rerun the checks, and do not create or update the PR until they pass.

## Push and pull request

1. Push the current branch without force-updating it:

   ```sh
   git push -u origin "<branch>"
   ```

2. Create a pull request against `main` if one does not already exist. Use
   `.github/PULL_REQUEST_TEMPLATE.md` and fill every applicable section:

   - summarize the change and its reason;
   - link the issue with `Closes #NNN`;
   - select the change type;
   - list the exact tests and operating systems run;
   - complete the checklist, including documentation status.

   ```sh
   gh pr create --base main --head "<branch>" --title "<title>" --body-file "<body-file>"
   ```

   Do not invent human-authored details that are not known. Ask the user for
   missing required submission text rather than treating approval as authorship.

3. If the PR already exists, update it only as needed to reflect the current
   commits and test results. Never rewrite published history or force-push
   without an explicit user request.

## CI and review gate

After every push, wait for the checks belonging to the current PR head:

```sh
gh pr checks "<number>" --watch
gh pr view "<number>" --json statusCheckRollup,reviews,mergeStateStatus,isDraft
```

The required repository workflows are defined in `.github/workflows/`:

- `build.yml`: locked tests and release builds on Ubuntu, Windows, and macOS;
- `fmt-check.yml`: `cargo fmt --all -- --check`;
- `version-guard.yml`: rejects unauthorized `Cargo.toml` version changes.

All required checks must be successful for the current head. Pending,
failed, missing, or unverifiable checks are not success. Confirm the PR is not
a draft, is mergeable, and has the required approval before proceeding.

When a check fails:

1. Inspect the failed job and its logs with `gh run view`.
2. Identify and fix the actual cause.
3. Add a new conventional commit; do not amend or rewrite the already-pushed
   commit history.
4. Run all local verification commands again.
5. Push the new commit and wait for the complete CI cycle again.

Repeat until every required check for the latest PR head is green.

## Merge

Before merging, verify again that the PR still targets `main`, all required
checks for the latest head are successful, the required review is present, and
the merge state is clean. Then merge the PR using the repository's configured
GitHub merge behavior without forcing an unrequested strategy:

```sh
gh pr merge "<number>"
```

If GitHub reports a merge conflict, stop immediately and ask the user how to
resolve it. Do not resolve conflicts automatically.

## Post-merge verification

After the merge command succeeds:

```sh
gh pr view "<number>" --json state,mergedAt,mergeCommit,url
git fetch origin main
git show-ref --verify refs/remotes/origin/main
git merge-base --is-ancestor "<merge-commit>" origin/main
```

Confirm that the PR state is `MERGED`, the merge commit is an ancestor of
`origin/main`, and report the PR URL and resulting commit. If the merge did
not complete, report that the change has not landed and include the blocker.
