# Contributing to Patinae

This guide covers development setup, code changes, and verification. For
installation and usage, start with the [README](README.md).

## Development setup

Use the stable Rust toolchain used by [CI](.github/workflows/rust.yml), with
Clippy and rustfmt. The full local check also requires Bash, Make, Node.js/npm
(CI uses Node.js 22), and [uv](https://docs.astral.sh/uv/).
Embedded Python checks use uv-managed Python 3.13.

On Linux, install the native development libraries listed in the
`Install Linux dependencies` step of the CI workflow before building the desktop
app. Platform packaging instructions live in [macos/README.md](macos/README.md)
and [windows/README.md](windows/README.md).

Clone the repository and run the desktop app:

```bash
git clone https://github.com/zmactep/patinae
cd patinae
make patinae-dev
```

| Target | Command |
| --- | --- |
| Patinae release build | `make patinae` |
| Patinae dev run | `make patinae-dev` |
| Reference plugins | `make plugins` |
| Python wheel, including web/widget assets | `make python-release` |
| Web viewer | `make web-build` |

Run `make help` for all targets. For faster native iteration, use
`make patinae-fast`, or `make patinae-fast-plugins` to build the app and stage
reference plugins beside it in `target/dev-fast/`.

The Python and web packages are separate Cargo workspaces, excluded from the
root workspace. Changes to shared crates can affect all three workspaces.
See the [Python guide](python/README.md), [web guide](web/README.md), and
[plugin development guide](docs/make-your-own-plugin.md) for their setup and APIs.

## Rules for changes

- Keep changes focused on one problem. Separate unrelated refactoring and
  formatting from behavior changes.
- Put shared molecular, command, scene, and rendering behavior in the owning
  crate. Keep platform-specific UI and integration code at the front-end boundary.
- Follow the surrounding code style and use rustfmt. Resolve Clippy warnings;
  explain any narrowly scoped lint suppression.
- Document the safety assumptions of unsafe code. The workspace enables
  `unsafe_op_in_unsafe_fn` and `undocumented_unsafe_blocks` lints.
- Preserve existing command, session, and plugin behavior unless a compatibility
  change is intentional and documented. Add regression coverage when changing
  these contracts.
- Add or update tests for changed behavior and bug fixes. For rendering or UI
  changes, also inspect the result in the affected viewer and record the platform
  and relevant GPU/backend details.
- Update usage examples and the relevant README when commands, configuration,
  or public APIs change.
- Keep dependency changes deliberate and commit the affected lockfiles. Check
  the root, Python, and web workspaces when shared Rust dependencies change.

## Architecture

```text
patinae/
├── patinae                    Native desktop application
├── crates/patinae-mol         Molecular data model: atoms, bonds, molecules, coordinate sets
├── crates/patinae-io          Structure, map, session, fetch, and trajectory IO
├── crates/patinae-select      Selection parser and evaluator
├── crates/patinae-color       Named colors, palettes, schemes, and ramps
├── crates/patinae-settings    Configuration and settings system
├── crates/patinae-algos       Alignment, symmetry, sequence, and analysis algorithms
├── crates/patinae-mm          Molecular mechanics, force fields, and minimization
├── crates/patinae-render      WebGPU renderer
├── crates/patinae-scene       Viewer state, scene graph, camera, input, and render bridge
├── crates/patinae-session     `.prs` and `.pse` session support
├── crates/patinae-cmd         Command parser, executor, and command registry
├── crates/patinae-framework   Shared app messaging and component infrastructure
├── crates/patinae-plugin      Plugin SDK
├── crates/patinae-plugin-host Runtime plugin host for front ends
├── plugins                    Reference Rust plugins
├── tools/prs-upgrade          Legacy `.prs` migration utility
├── bench                      Performance benchmarks
├── python                     Python package and Jupyter widget
└── web                        WebAssembly/WebGPU viewer package
```

## Verification

Run the baseline checks locally before opening a pull request:

```bash
rustup update stable
rustup component add --toolchain stable clippy rustfmt
rustup target add --toolchain stable wasm32-unknown-unknown
uv python install 3.13
RUSTUP_TOOLCHAIN=stable make test-ci
```

Additional compliance tools are useful when installed locally:
`cargo audit` or `cargo deny` for dependency policy, `cargo hack` for feature
matrices, `cargo udeps` for unused dependencies, and Miri for targeted
unsafe-code tests.

The full `make test-ci` run checks formatting in all three workspaces, release
metadata and lockfiles, workspace and desktop Clippy, core and workspace tests,
GUI event dispatch, the WASM/widget build, Rust bindings, an installed Python
wheel, and the dynamic plugin ABI. It continues after failed checks and returns
a nonzero exit status if any check fails.

Local checks cover the current host. GitHub Actions also checks the desktop on
macOS and Windows and the native workspace and bindings on Linux. Explicit GPU
contracts run separately and require a compatible adapter.

Use focused checks while iterating:

| Check | Command |
| --- | --- |
| Workspace compilation | `make check` |
| Workspace Clippy | `make lint` |
| Core and application tests without plugin runtimes | `make test-core` |
| Workspace CPU tests, including plugins | `make test` |
| Rust bindings and installed Python wheel | `make test-bindings` |
| Dynamic AI plugin ABI | `make test-abi` |
| Explicit GPU contracts | `make test-gpu` |
| Release versions and Cargo lockfiles | `make check-release-metadata` |

After web changes, run `make widget-build` before `make test-bindings` so the
Python wheel includes fresh viewer assets. The full `make test-ci` run rebuilds
them itself. When changing local CI orchestration, also run
`bash scripts/test-check-ci.sh`, as the CI workflow does.

For documentation-only changes, check Markdown formatting, local links, and
any commands or examples you changed; a full build is unnecessary unless the
change needs runtime verification.

## Preparing a pull request

Describe the problem, the resulting behavior, and how you verified it. Include
screenshots for visible UI changes and list any checks you could not run.
Use concise commit messages consistent with the repository history, such as
`fix(render): ...` or `docs: ...`.

For a release version change, use `make version V=X.Y.Z` to update the shared
metadata, then run `make check-release-metadata` and review the resulting diff.
Ordinary fixes do not need a version bump.
