# Fork features

Generated from `scripts/fork-features/registry.toml` by
`scripts/fork-features/check.py --md`. Edit the registry, not this file.

pc-combined is upstream `main` plus the features below. Upstream merges
(#1499 deleting `crates/ui`, the 0.3.8/0.3.9 composer refactor) have dropped or
broken fork features without a conflict, so the fork now guards them:

- **The registry** (this table): every feature, who knows it, the branch that
  carries it alone, its commits, and what fails if it goes.
- **`scripts/fork-features/check.py`**: markers (a call site, an RPC method, a
  setting key, a test's own name that must still be in the tree), the zgx pin,
  the Luau/cargo tests named here, and *survival* (how many of each feature's
  commit lines are still in the tree, against a recorded floor).
- **A patch queue, in the registry**: each feature keeps its own branch
  (`branch` below), and `check.py --risk` merges each one onto the new main with
  `git merge-tree` (nothing touched), so a conflict is named per feature before
  `pc-combined` is merged.

## The procedure

Merging main into pc-combined (whoever does it):

1. `git fetch origin && scripts/fork-features/check.py --risk`: the features
   whose files main changed or deleted, and the ones that would conflict.
2. `git merge origin/main`; resolve; for each conflicted or "DELETED upstream"
   feature, read its commits (`git log --oneline -- <file>`) and keep or port it.
3. `scripts/fork-features/check.py --survival --tests`: markers, pin, the Luau
   tests, and the survival floors. A failure is a feature the merge dropped or
   broke: restore it (or ask its owner chat), don't edit the registry to pass.
   Changing a marker, status or floor is a decision: say why in the commit and
   tell the owner.
4. Commit the merge. Then, before every build:

   `scripts/fork-features/check.py --gate --tests` (add `--rust` when the build
   is for the engine or the zgx app and there's time: it compiles)

   It fetches, fails if HEAD is behind `origin/main` (the standing rule: don't
   build), and fails on any feature gone. Don't stage a build that fails it.

Adding a feature (new fork commit, new fork branch): add a `[[feature]]` to the
registry in the same commit, with markers that only exist while it's wired in
and at least one test; run `check.py --bless` for its survival floor; run
`check.py --md`.

When upstream ships its own version of a fork feature: set `status =
"superseded"` with `why`, keep a marker on the upstream behaviour, and drop the
fork commits' survival floor (`--bless`).

## Features
