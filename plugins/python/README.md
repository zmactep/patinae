# Python plugin

Run Python inside the Patinae desktop application. The plugin embeds CPython
and adds inline execution, `.py` script loading, per-atom expressions, and a
Python editor panel. Scripts control the current desktop scene through `cmd`.

This is the native interpreter plugin. The separate [Python package](../../python/README.md)
provides `patinae`, `cmd`, and notebook support; it must be importable by the
embedded interpreter for `cmd.*` calls to work.

## Build and install

Build the plugin and application from the same source revision. The repository
Makefile selects uv-managed CPython 3.13. You also need Rust, Cargo, and uv;
building the Python wheel through the Makefile additionally needs Node.js/npm
for widget assets.

From the repository root:

```sh
uv python install 3.13
make patinae
make plugins
make python-release
uv venv --python 3.13 .venv
uv pip install --python .venv/bin/python python/target/wheels/patinae-*.whl numpy
uv run --no-project --python .venv/bin/python ./target/release/patinae
```

Use the wheel built from this checkout if the output directory also contains
older wheels. `make plugins` stages the native libraries beside the executable.
The local `.venv` supplies the matching interpreter and installed package at
startup; no environment activation is needed.

To build only this plugin, select the interpreter explicitly:

```sh
PYO3_PYTHON="$(uv python find --python-preference only-managed 3.13)" \
  cargo build --locked --release -p python-plugin
mkdir -p ~/.patinae/plugins
cp target/release/libpython_plugin.dylib ~/.patinae/plugins/
```

On Linux copy `libpython_plugin.so`; on Windows copy `python_plugin.dll` and
use `.venv/Scripts/python.exe` in place of `.venv/bin/python`. Windows also
needs the matching Python DLL available when the plugin loads; the repository's
Windows bundle includes its runtime and dependency sidecar. The complete macOS
application bundle can be built with `make app-full`.

Restart Patinae after installation. See the [plugin overview](../README.md) for
custom plugin directories and shared-library discovery.

## Inline commands and scripts

Enter these in the Patinae REPL:

```pml
python print("hello from Patinae")
/import math; print(math.pi)
python from patinae import cmd; cmd.color("cyan", "all")
run analysis.py
```

`python <code>` and `/<code>` execute the rest of the line as Python. `run`
dispatches `.py` files to the plugin. The interpreter retains variables and
imports between executions. When the package is available, `cmd` and `stored`
are imported automatically.

For example, save this as `analysis.py` and run it against a loaded structure:

```python
from patinae import cmd

print("Atoms:", cmd.count_atoms("all"))
cmd.show("cartoon", "polymer")
cmd.color("cyan", "chain A")
```

## Iterate and alter atoms

```pml
iterate name CA, print(model, chain, resi, resn, b)
alter name CA, b=20.0
```

`iterate <selection>, <expression>` evaluates an expression for each selected
atom without applying atom changes. `alter` applies changes to supported atom
properties.

Mutable properties are `name`, `resn`, `resv`, `chain`, `segi`, `alt`, `elem`,
`b`, `q`, `vdw`, `partial_charge`, `formal_charge`, `ss`, `color`, and `type`.
Coordinates `x`, `y`, `z` and identifiers `index`, `ID`, `rank`, `model`, and
`hetatm` are read-only in `alter`. `resi` is also available when iterating.

Use `stored` to collect values across atoms:

```pml
python stored.b_values = []
iterate name CA, stored.b_values.append(b)
python print(stored.b_values)
```

## Panel and background work

The **Python** bottom panel starts hidden. It provides a syntax-highlighted
editor, **Run script**, **Stop**, and **Clear output**, with task status and
captured output. Clearing output does not reset the interpreter.

Python execution runs on a worker and is tracked by the host task system. REPL
submission can return before execution finishes; output arrives later. Python
`cmd` methods wait for their accepted child work by default. For explicit task
observation, use the package's [task API](../../python/README.md#background-tasks).

Stopping a script requests cooperative cancellation. Changes already applied to
the scene remain. Calls blocked inside native extension code may delay Python
interruption. The panel retains a bounded output tail, so write large results
to a file from the script when needed.

## Troubleshooting

- **No matching Python found:** the runtime major/minor version must match the
  CPython used at build time. Startup logs report that version. Discovery checks
  the bundled interpreter, `VIRTUAL_ENV`, the current directory's `.venv`, then
  `PATH`. A user-supplied `PYTHONHOME` bypasses automatic configuration.
- **`patinae` cannot be imported:** install the matching wheel into the interpreter
  environment used by the plugin. Plain Python can work while `cmd`, `iterate`,
  and `alter` remain unavailable.
- **A command is missing:** check `capabilities plugins`, `help python`, and
  `capabilities formats run`, then restart after rebuilding.

## Source

[runtime.rs](src/runtime.rs) registers commands, the `.py` handler, and the panel.
[engine.rs](src/engine.rs) discovers CPython and evaluates code;
[worker.rs](src/worker.rs) and [handler.rs](src/handler.rs) connect execution to
host tasks. [commands.rs](src/commands.rs) defines the REPL syntax.
