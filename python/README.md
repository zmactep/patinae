# Patinae Python Bindings

Python bindings for the Rust/WebGPU molecular visualization workspace.

## Installation

```bash
pip install patinae
```

## Background tasks

Command methods wait for accepted background work by default. Pass `wait=False`
to receive task IDs immediately, then observe or cancel them through `tasks`:

```python
from patinae import cmd, tasks

reply = cmd.fetch("1CRN", wait=False)
task_id = reply["task_ids"][0]
snapshot = tasks.get(task_id)
snapshot = tasks.wait(task_id, timeout=30)
```

`tasks.list(**filters)` lists retained tasks; `tasks.cancel(task_id)` requests
cancellation. A wait timeout does not cancel the task.

## Jupyter Widget

The package includes an [anywidget](https://anywidget.dev/)-based widget for interactive visualization in notebooks. Works in JupyterLab, Jupyter Notebook, VS Code, and Google Colab.

```bash
pip install patinae[widget]
```

```python
from patinae.widget import Viewer

view = Viewer()
view.show()
cmd = view.get_cmd()
cmd.fetch("1CRN")
cmd.show("cartoon")
cmd.color("green", "chain A")
```

Features:
- **Tracked commands** — wait by default; use `wait=False` and `view.tasks` to observe or cancel background work in this viewer
- **Synchronous queries** — request/response channel for commands that return data
- **Local file loading** — load structures from the local filesystem into the browser viewer
- **Picking support** — optional click-to-select atoms (`Viewer(picking=True)`)
- **Configurable layout** — `width` and `height` parameters with sensible defaults

## License

BSD-3-Clause
