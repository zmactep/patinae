#!/usr/bin/env bash
# Exercise CI orchestration without compiling or downloading dependencies.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
runtime=$(mktemp -d "${TMPDIR:-/tmp}/patinae-check-ci.XXXXXX")
trap 'rm -rf "$runtime"' EXIT
mkdir -p "$runtime/bin"
cat > "$runtime/bin/stub" <<'STUB'
#!/usr/bin/env bash
set -eu
command="$(basename "$0"):$*"
printf '%s\n' "$command" >> "$CI_TEST_CALLS"
if [[ "$command" == "${CI_TEST_FAIL:-}" ]]; then
    exit 42
fi
STUB
chmod +x "$runtime/bin/stub"
for tool in cargo make npm; do
    ln -s stub "$runtime/bin/$tool"
done
export PATH="$runtime/bin:$PATH"
export CARGO="$runtime/bin/cargo"
export MAKE="$runtime/bin/make"
export CI_TEST_CALLS="$runtime/calls"

for failure in '' 'cargo:fmt --all -- --check' 'make:lint' 'npm:ci'; do
    export CI_TEST_FAIL="$failure"
    : > "$CI_TEST_CALLS"
    status=0
    bash "$root/scripts/check-ci.sh" > "$runtime/output" 2>&1 || status=$?
    if [[ -z "$failure" ]]; then
        [[ "$status" == 0 ]]
        ! grep -q '^FAIL ' "$runtime/output"
    else
        [[ "$status" == 1 ]]
        grep -q '^FAIL (exit 42)' "$runtime/output"
    fi
    # Early failures must neither skip the last lane nor hide the summary.
    grep -qx 'make:test-abi' "$CI_TEST_CALLS"
    grep -q '^== CI summary (local host) ==' "$runtime/output"
    [[ $(grep -Ec '^(PASS|FAIL \(exit [0-9]+\)) ' "$runtime/output") == 14 ]]
done
printf 'CI orchestration: success and all three failure scenarios passed.\n'
