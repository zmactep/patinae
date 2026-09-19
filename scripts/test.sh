#!/usr/bin/env bash
# Run the same test lanes locally and in CI, without touching installed plugins.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"
uv_bin=${UV:-uv}
cargo_bin=${CARGO:-cargo}
python_version=${PATINAE_TEST_PYTHON_VERSION:-3.13}
export PYO3_PYTHON
PYO3_PYTHON=$("$uv_bin" python find --system --python-preference only-managed "$python_version")
# Embedded PyO3 binaries do not get uv's interpreter startup path discovery.
export PYTHONHOME
PYTHONHOME=$("$uv_bin" run --no-project --python "$PYO3_PYTHON" --no-sync python -I -c 'import sys; print(sys.base_prefix)')
work="$root/target/test-runtime"
mkdir -p "$work"

case "${1:-}" in
    workspace)
        "$cargo_bin" test --locked --workspace
        ;;
    bindings)
        "$cargo_bin" test --locked --manifest-path python/Cargo.toml --lib
        "$cargo_bin" test --locked --manifest-path web/Cargo.toml --lib
        unset PYTHONHOME
        env_dir="$work/python-$python_version"
        if [[ ! -f "$env_dir/pyvenv.cfg" ]]; then
            "$uv_bin" venv --python "$PYO3_PYTHON" "$env_dir"
        fi
        test_python="$env_dir/bin/python"
        if [[ ! -x "$test_python" ]]; then test_python="$env_dir/Scripts/python.exe"; fi
        "$uv_bin" pip install --python "$test_python" 'maturin>=1,<2' 'pytest>=7' 'numpy>=1.20'
        wheels=$(mktemp -d "$work/wheels.XXXXXX")
        trap 'rm -rf "$wheels"' EXIT
        # maturin needs clang to interpret its Darwin linker arguments.
        if [[ "$(uname -s)" == Darwin ]]; then
            export CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=clang
            export CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER=clang
        fi
        "$uv_bin" run --no-project --python "$test_python" --no-sync python -m maturin build \
            --locked --manifest-path python/Cargo.toml --interpreter "$PYO3_PYTHON" --out "$wheels"
        "$uv_bin" pip install --python "$test_python" --reinstall --no-deps "$wheels"/*.whl
        # Isolated mode excludes cwd/PYTHONPATH; importlib prevents pytest from
        # prepending the source package. Assert the wheel is actually imported.
        cd "$work"
        "$uv_bin" run --no-project --python "$test_python" --no-sync python -I -c \
            'import pathlib, patinae, sys; p = pathlib.Path(patinae.__file__).resolve(); assert p.is_relative_to(pathlib.Path(sys.prefix).resolve()), p; print("Installed wheel:", p)'
        "$uv_bin" run --no-project --python "$test_python" --no-sync python -I -m pytest \
            --import-mode=importlib "$root/python/tests" -q
        ;;
    abi)
        plugins=$(mktemp -d "$work/plugins.XXXXXX")
        trap 'rm -rf "$plugins"' EXIT
        case "$(uname -s)" in
            Darwin) suffix=dylib; prefix=lib ;;
            MINGW*|MSYS*|CYGWIN*) suffix=dll; prefix= ;;
            *) suffix=so; prefix=lib ;;
        esac
        names=(hello raytracer ai python)
        if [[ "$suffix" != dll ]]; then names+=(ipc); fi
        packages=()
        for plugin in "${names[@]}"; do packages+=(-p "${plugin}-plugin"); done
        "$cargo_bin" build --locked "${packages[@]}"
        build_dir=${CARGO_TARGET_DIR:-"$root/target"}
        for plugin in "${names[@]}"; do
            cp "$build_dir/debug/${prefix}${plugin}_plugin.$suffix" "$plugins/"
        done
        export PATINAE_PLUGIN_TEST_DIR="$plugins"
        export PATINAE_AI_TEST_LIBRARY="$plugins/${prefix}ai_plugin.$suffix"
        "$cargo_bin" test --locked -p patinae-plugin-host loads_built_reference_plugins -- --ignored --test-threads=1
        "$cargo_bin" test --locked -p ai-plugin dynamic_plugin_round_trips_commands_images_and_cancellation -- --ignored --test-threads=1
        ;;
    gpu)
        "$cargo_bin" test --locked -p patinae-render -p raytracer-plugin -p patinae-plugin-host gpu_ -- --ignored --test-threads=1 --nocapture
        ;;
    *) echo 'Usage: scripts/test.sh {workspace|bindings|abi|gpu}' >&2; exit 2 ;;
esac
