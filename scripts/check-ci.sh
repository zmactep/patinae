#!/usr/bin/env bash
# Run every local CI lane, even when an earlier independent check fails.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
cargo_bin=${CARGO:-cargo}
make_bin=${MAKE:-make}
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0

names=()
results=()
failed=0
run_check() {
    local name=$1
    shift
    printf '\n== %s ==\n' "$name"
    names+=("$name")
    if "$@"; then
        results+=(PASS)
    else
        local status=$?
        results+=("FAIL (exit $status)")
        failed=1
    fi
}

run_check 'Rust toolchain' "$cargo_bin" --version
run_check 'Clippy version' "$cargo_bin" clippy --version
run_check 'Workspace formatting' "$cargo_bin" fmt --all -- --check
run_check 'Python formatting' "$cargo_bin" fmt --manifest-path python/Cargo.toml --all -- --check
run_check 'Web formatting' "$cargo_bin" fmt --manifest-path web/Cargo.toml --all -- --check
run_check 'Release metadata and lockfiles' "$make_bin" check-release-metadata
run_check 'Workspace Clippy' "$make_bin" lint
# Separate feature resolution matches the desktop jobs rather than relying on
# features unified by the full workspace invocation.
run_check 'Desktop Clippy' "$cargo_bin" clippy --locked -p patinae --all-targets -- -D warnings
run_check 'Core tests' "$make_bin" test-core
run_check 'Workspace tests' "$make_bin" test
run_check 'GUI event dispatch' "$cargo_bin" test --locked -p patinae --bin patinae app::events::tests -- --test-threads=1
run_check 'Web and widget build' bash -c 'cd web && npm ci && npm run build:ci'
run_check 'Bindings and installed wheel' "$make_bin" test-bindings
run_check 'Dynamic plugin ABI' "$make_bin" test-abi

printf '\n== CI summary (local host) ==\n'
for ((i = 0; i < ${#names[@]}; i++)); do
    printf '%-18s %s\n' "${results[i]}" "${names[i]}"
done
exit "$failed"
