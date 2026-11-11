# Plan 004: The workflows that build shipped files use actions pinned to commit SHAs

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat c190987..HEAD -- .github/workflows/release.yml .github/workflows/linux-build.yml`
> If either file changed since this plan was written, compare the "Current
> state" list against the live files before proceeding; on a mismatch, treat
> it as a STOP condition.

## Status

- **Priority**: P3
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: security
- **Planned at**: commit `c190987`, 2026-11-11

## Why this matters

`release.yml` builds the macOS and Windows apps and signs the update files
with the updater key; every installed copy trusts updates signed with that
key and installs them automatically. `linux-build.yml` builds the `.deb` that
is attached to releases by hand. Both run other people's GitHub Actions
referenced by a tag or branch name (`@v4`, `@v2`, `@stable`). Whoever controls
such a name can point it at new code, which would then run inside the job
that builds and signs. Pinning each action to a full commit SHA means only
the reviewed code runs until someone deliberately updates the pin. This is
hardening, not a fix for a known compromise. (The downloads of libVLC and
FFmpeg are already checked against checksums.)

## Current state

Every remote `uses:` line in the two files, at the planned commit:

`.github/workflows/release.yml`

| Line | Reference |
|---|---|
| 40 | `actions/checkout@v4` |
| 67 | `actions/checkout@v4` |
| 69 | `actions/setup-node@v4` |
| 74 | `dtolnay/rust-toolchain@stable` |
| 76 | `Swatinem/rust-cache@v2` |
| 104 | `actions/upload-artifact@v4` |
| 113 | `actions/checkout@v4` |
| 114 | `actions/download-artifact@v4` |
| 121 | `actions/upload-artifact@v4` |

Lines 26, 29 and 32 are `uses: ./.github/workflows/...` (this repository's
own workflows). Leave those as they are.

`.github/workflows/linux-build.yml`

| Line | Reference |
|---|---|
| 17 | `actions/checkout@v4` |
| 18 | `actions/setup-node@v4` |
| 22 | `dtolnay/rust-toolchain@stable` (followed by `with:` / `components: clippy`) |
| 25 | `Swatinem/rust-cache@v2` |
| 68 | `actions/upload-artifact@v4` |
| 74 | `actions/upload-artifact@v4` |

`release.yml:74` today has no `with:` block:

```yaml
      - uses: dtolnay/rust-toolchain@stable

      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: src-tauri
```

`linux-build.yml:22-24` today:

```yaml
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
```

One special case: `dtolnay/rust-toolchain` chooses the Rust version from the
name of the ref it is called with (`@stable` means stable Rust). Pinned to a
SHA, it has no name to read, so the version must be passed as the `toolchain`
input. Step 2 covers this.

**Conventions to match**: the workflows use two-space YAML indentation and
keep comments short. Keep every other line of both files byte-for-byte the
same.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Commit SHA a ref points at | `gh api repos/<owner>/<repo>/commits/<ref> --jq .sha` | one 40-character hex string |
| List remaining unpinned refs | the `grep` command below this table | no output when done |
| YAML still parses | the `ruby` command below this table | exit 0, no output |
| Workflow lint (only if `actionlint` is installed) | `actionlint .github/workflows/release.yml .github/workflows/linux-build.yml` | exit 0 |

The two longer commands, to copy exactly:

```sh
grep -nE "uses: +[^./]" .github/workflows/release.yml .github/workflows/linux-build.yml | grep -vE "@[0-9a-f]{40}( |$)"
ruby -e 'require "yaml"; ARGV.each { |f| YAML.load_file(f) }' .github/workflows/release.yml .github/workflows/linux-build.yml
```

`gh` must be signed in (`gh auth status` exits 0). These calls only read
public repositories.

## Scope

**In scope** (the only files you should modify):
- `.github/workflows/release.yml`
- `.github/workflows/linux-build.yml`
- `plans/README.md` (status row only)

**Out of scope** (do NOT touch):
- `.github/workflows/checks.yml`, `ui-checks.yml`, `engine-tests.yml`,
  `mutants.yml` and `.github/actions/scope/action.yml`. They run with
  read-only permissions and no secrets; pinning them is a possible follow-up,
  not part of this plan.
- Any step's `run:` script, `with:` values (other than adding `toolchain:`),
  permissions, triggers, or job structure.
- Adding Dependabot, Renovate, or any other automation.
- **Do not run the Release workflow** (`gh workflow run release.yml` or the
  Actions UI). It creates a draft release and signs update files.

## Git workflow

- Other agent threads edit the main checkout. Work in your own worktree:
  `git worktree add ../revlc-plan-004 -b ci/pin-release-actions main`
  and never run `git commit -a` in the main checkout.
- Commit message style is a plain sentence with an area prefix, as in
  `git log`: `CI: a distinct gate name per workflow, for branch protection`,
  `CI: don't re-run on PR description edits`.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Resolve the SHAs

Run each command and write down the result. Each must print exactly one
40-character lowercase hex string.

```sh
gh api repos/actions/checkout/commits/v4 --jq .sha
gh api repos/actions/setup-node/commits/v4 --jq .sha
gh api repos/actions/upload-artifact/commits/v4 --jq .sha
gh api repos/actions/download-artifact/commits/v4 --jq .sha
gh api repos/Swatinem/rust-cache/commits/v2 --jq .sha
gh api repos/dtolnay/rust-toolchain/commits/master --jq .sha
```

Then confirm the `dtolnay/rust-toolchain` commit accepts a `toolchain` input:

```sh
gh api "repos/dtolnay/rust-toolchain/contents/action.yml?ref=<the sha you got for master>" --jq .content | base64 --decode | grep -n "toolchain:"
```

**Verify**: six SHAs recorded, each matching `^[0-9a-f]{40}$`; the last command prints at least one line containing `toolchain:` under `inputs:`.

### Step 2: Pin every reference

In both files, replace each reference in the "Current state" tables with
`<owner>/<repo>@<sha> # <the old ref>`. Examples of the shape (use the SHAs
from step 1, not these placeholders):

```yaml
      - uses: actions/checkout@<sha from step 1> # v4
```

```yaml
      - uses: Swatinem/rust-cache@<sha from step 1> # v2
        with:
          workspaces: src-tauri
```

For `dtolnay/rust-toolchain`, pin to the `master` SHA and pass the toolchain
explicitly.

`release.yml` (it had no `with:` block; add one):

```yaml
      - uses: dtolnay/rust-toolchain@<sha from step 1> # master
        with:
          toolchain: stable
```

`linux-build.yml` (keep `components`, add `toolchain`):

```yaml
      - uses: dtolnay/rust-toolchain@<sha from step 1> # master
        with:
          toolchain: stable
          components: clippy
```

Use the same SHA for every occurrence of the same action.

**Verify**:
- `grep -nE "uses: +[^./]" .github/workflows/release.yml .github/workflows/linux-build.yml | grep -vE "@[0-9a-f]{40}( |$)"` → no output.
- `grep -cE "uses: +[^./].*@[0-9a-f]{40} # " .github/workflows/release.yml` → `9`.
- `grep -cE "uses: +[^./].*@[0-9a-f]{40} # " .github/workflows/linux-build.yml` → `6`.
- `grep -c "toolchain: stable" .github/workflows/release.yml .github/workflows/linux-build.yml` → `1` for each file.

### Step 3: Check nothing else moved

**Verify**:
- `ruby -e 'require "yaml"; ARGV.each { |f| YAML.load_file(f) }' .github/workflows/release.yml .github/workflows/linux-build.yml` → exit 0.
- `git diff --stat` → exactly two files changed; `release.yml` with 9 lines replaced plus 2 added, `linux-build.yml` with 6 lines replaced plus 1 added.
- `git diff | grep -E "^[-+]" | grep -vE "^(\+\+\+|---)" | grep -vE "uses:|toolchain: stable|^\+ +with:$"` → no output (every changed line is a `uses:` line, the added `toolchain: stable`, or the one added `with:`).
- If `actionlint` is installed: `actionlint .github/workflows/release.yml .github/workflows/linux-build.yml` → exit 0.

## Test plan

There are no unit tests for workflows. Static verification is in the steps.
The real proof is a green run, which needs a push and is the operator's call:

- **Linux build** can be run safely on the branch
  (`gh workflow run linux-build.yml --ref ci/pin-release-actions`): it only
  produces branch artifacts. Run it only if the operator has told you to push.
- **Release** is exercised by the next real release. Never trigger it to test.

## Done criteria

ALL must hold:

- [ ] The "list remaining unpinned refs" command prints nothing
- [ ] `release.yml` has 9 pinned `uses:` lines and `linux-build.yml` has 6, each with a trailing `# <old ref>` comment
- [ ] `toolchain: stable` appears once in each file
- [ ] Both files parse as YAML
- [ ] `git status --short` lists only the two workflow files and `plans/README.md`
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- A `gh api` call fails, returns nothing, or returns something that isn't a
  40-character SHA. Never guess or copy a SHA from memory or from another
  repository's workflow.
- The `dtolnay/rust-toolchain` `action.yml` at the resolved SHA has no
  `toolchain` input.
- The `uses:` lines in either file differ from the tables in "Current state"
  (new actions added, different versions).
- You are asked or tempted to run the Release workflow.

## Maintenance notes

- Pinned SHAs don't move, so fixes to the actions no longer arrive on their
  own. To update one, resolve the tag again with the command in step 1 and
  replace the SHA everywhere it appears. GitHub's Dependabot can do this for
  the `github-actions` ecosystem; adding it was left out here because it is
  the maintainer's choice whether to take on those pull requests.
- New steps added to these two workflows should be pinned the same way.
- Reviewer: spot-check two SHAs against the action's tags page, and confirm
  the first Linux build after merging is green (it exercises
  `rust-toolchain`, `rust-cache`, `setup-node` and `upload-artifact`).
- Follow-up left out: the same pinning for `checks.yml`, `ui-checks.yml`,
  `engine-tests.yml` and `mutants.yml`, which gate releases but hold no
  secrets.
