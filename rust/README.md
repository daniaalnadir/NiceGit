# NiceGit (Rust)

The cross-platform Rust port of NiceGit. Run the commands below from the repository root.

## Testing locally

`rust/scripts/test-all.sh` runs the test suite on this Mac and in a Linux Docker container at the same time. It is usually faster than waiting for GitHub CI:

```sh
rust/scripts/test-all.sh                                           # full suite, once
rust/scripts/test-all.sh -n 3                                      # three repeats per platform, to catch flaky tests
rust/scripts/test-all.sh ui_tests::command_palette_runs_a_command  # only tests matching a filter
```

- The macOS run uses your local cargo. The Linux run uses the `rust:latest` Docker image with 2 CPUs. `rust/` is mounted read-only, and build output and the cargo registry are cached in `${XDG_CACHE_HOME:-~/.cache}/nicegit-linux`, so later runs are faster.
- Without a running Docker daemon, the script runs macOS only.
- Each output line is prefixed with `[macos]` or `[linux]`. Full logs go to a temp folder that the script prints at the start. The summary gives the pass and fail counts per platform and names the failing tests. The script exits non-zero if any repeat fails.
- Windows is not covered locally. GitHub CI (`.github/workflows/rust-ci.yml`, `windows-latest`) is the only check for it.
