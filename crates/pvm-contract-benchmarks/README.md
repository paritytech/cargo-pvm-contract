# pvm-contract-benchmarks

Binary size comparison tool for PolkaVM contracts.

## Overview

This crate builds multiple contract variants (no-alloc, with-alloc, builder-dsl, storage) in both debug and release profiles, measures their binary sizes, and generates comparison reports. It is used in CI to detect size regressions in pull requests.

**Goals:**
- Track binary size impact of code changes
- Prevent unintentional size regressions (>5% per artifact)
- Compare different allocation strategies and API choices

## Prerequisites

- **Rust nightly**: Required for `-Zbuild-std=core,alloc`
- **solc 0.8.26**: Required for Solidity interface compilation

Install nightly:
```bash
rustup toolchain install nightly --component rust-src --profile minimal
```

Install solc (Linux):
```bash
SOLC_VERSION=0.8.26
curl -L https://github.com/ethereum/solidity/releases/download/v${SOLC_VERSION}/solc-static-linux -o solc
chmod +x solc
sudo mv solc /usr/local/bin/solc
```

## Local Usage

### Run Binary Size Benchmark

From repository root:

```bash
cargo +nightly run -p pvm-contract-benchmarks --bin build-and-measure
```

This will:
1. Build all contract variants (fibonacci, mytoken, multi) in debug and release profiles
2. Save `.polkavm` artifacts to `target/benchmark-artifacts/`
3. Generate a comparison report at `target/benchmark-results/binary-sizes.md`

**Output locations:**
- Artifacts: `target/benchmark-artifacts/{contract}_{variant}.{profile}.polkavm`
- Report: `target/benchmark-results/binary-sizes.md`

### Interpret Results

**IMPORTANT: Only release profile results matter for decision-making.**

The benchmark report includes both debug and release builds, but:
- **Release profile** is the source of truth for binary size comparisons
- **Debug profile** is included only for diagnostics and troubleshooting
- All CI regression checks and PR decisions are based on release artifacts only

#### View Release-Only Results

After running the benchmark, filter the report to show only release rows:

```bash
# View only release profile results
grep -E '(^\|.*Profile|release)' target/benchmark-results/binary-sizes.md

# Or exclude debug rows entirely
grep -v 'debug' target/benchmark-results/binary-sizes.md | grep -E '(^\||Contract)'
```

#### Report Structure

The full report shows:
- **Overall Comparison**: All variants in a single table
- **Per-Contract Comparison**: Grouped by contract name
- **Size Differences**: Percentage overhead vs no-alloc baseline

Example output (release rows only):
```
| Contract  | Variant    | Profile | Size (bytes) | Size (KB) |
|-----------|------------|---------|--------------|-----------|
| fibonacci | no-alloc   | release | 1024         | 1.00      |
| fibonacci | with-alloc | release | 2048         | 2.00      |
```

## CI Behavior

### Pull Requests

When a PR is opened or updated:
1. Builds current branch artifacts
2. Checks out `origin/main` in a git worktree
3. Builds baseline artifacts from `origin/main`
4. Compares each artifact using a **5% regression threshold**
5. Posts a sticky comment on the PR with comparison table
6. **Fails the check** if any artifact exceeds 5% size increase

**Comparison table columns:**
- **Baseline**: Size from `origin/main`
- **Current**: Size from PR branch
- **Delta**: Percentage change
- **Status**: `OK` (<=5%), `FAIL` (>5%), `NEW` (added), `DEL` (removed)

### Main Branch

When commits are pushed to `main`:
1. Builds all artifacts
2. Posts size report to GitHub Actions step summary
3. No comparison or regression check (baseline is being updated)

## Re-Baselining

If a size increase is **intentional** (e.g., new feature, dependency upgrade):

1. **Verify the increase is justified** by reviewing the comparison table in the PR comment
2. **Document the reason** in the PR description
3. **Merge the PR** — this updates the baseline on `main`
4. Future PRs will compare against the new baseline

**Do not:**
- Increase the 5% threshold without team discussion
- Merge size regressions without understanding the cause
- Ignore `FAIL` status without investigation

## Extending Benchmarks

To add a new contract:

1. Add contract source files to `crates/cargo-pvm-contract/templates/examples/{contract}/`
   - `{contract}_no_alloc.rs`
   - `{contract}_with_alloc.rs`
   - `{contract}_dsl.rs`
   - `{contract}.sol` (interface)

2. Update `contracts` list in `src/bin/build-and-measure.rs`:
   ```rust
   let contracts = vec!["fibonacci", "mytoken", "multi", "your_contract"];
   ```

3. Run locally to verify all variants build successfully

## Troubleshooting

**Build fails with "target JSON not found":**
- Ensure nightly toolchain is installed with `rust-src` component

**Build fails with "solc not found":**
- Install solc 0.8.26 (see Prerequisites)

**CI fails with "Binary size regression detected":**
- Review the PR comment comparison table
- Identify which artifact(s) exceeded 5%
- Investigate code changes causing the increase
- Either optimize the code or document why the increase is acceptable

**Worktree errors in CI:**
- Ensure `fetch-depth: 0` in checkout step (required for `origin/main` ref)
- Cleanup step runs with `always()` to prevent stale worktrees

## Architecture

- `src/lib.rs`: Core logic for parsing `.polkavm` filenames, collecting variants, generating reports
- `src/bin/build-and-measure.rs`: CLI tool that builds all variants and generates reports
- `.github/workflows/benchmark.yml`: CI job that runs comparison and posts PR comments

**Artifact naming convention:**
```
{contract}_{variant}.{profile}.polkavm
```

Examples:
- `fibonacci_no-alloc.release.polkavm`
- `mytoken_with-alloc.debug.polkavm`
- `multi_builder-dsl.release.polkavm`
