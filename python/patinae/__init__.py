"""
Patinae: Python bindings for the Rust/WebGPU molecular visualization workspace
with a PyMOL-compatible command surface.

Unified package that works in two modes:
- Standalone: ``python -c "from patinae import cmd; cmd.load('x.pdb')"``
- Embedded: ``run script.py`` inside the Patinae GUI (plugin sets backend)

The ``cmd`` object is created lazily on first access.
"""

import sys
from contextvars import ContextVar

from ._cmd import Cmd
from . import tasks
from .tasks import CommandError, TaskError, TaskApiError, Tasks
from ._labels import LabelAnchor, LabelEntity, LabelObject, LabelRevisions
from ._patinae import (
    Atom,
    Bond,
    Color,
    CoordSet,
    Element,
    # Types
    ObjectMolecule,
    # Exceptions
    PatinaeError,
    SelectionError,
    SelectionResult,
    color,
    io,
    # Submodules
    mol,
    selecting,
    settings,
)

__version__ = "0.4.0"

__all__ = [
    "cmd",
    "tasks",
    "Cmd",
    "CommandError",
    "TaskError",
    "TaskApiError",
    "Tasks",
    "ObjectMolecule",
    "Atom",
    "Bond",
    "Element",
    "CoordSet",
    "Color",
    "SelectionResult",
    "LabelAnchor",
    "LabelEntity",
    "LabelObject",
    "LabelRevisions",
    "PatinaeError",
    "SelectionError",
    "mol",
    "io",
    "selecting",
    "color",
    "settings",
]

# Lazy cmd singleton
_cmd = None
_task_cmd = ContextVar("patinae_task_cmd", default=None)


class Store:
    def __getattr__(self, name):
        return self.__dict__.get(name, None)

    def __setattr__(self, name, value):
        object.__setattr__(self, name, value)


stored = Store()


def _get_cmd():
    """Get or create the global cmd instance."""
    global _cmd
    task_cmd = _task_cmd.get()
    if task_cmd is not None:
        return task_cmd
    if _cmd is None:
        if hasattr(sys, "_patinae_backend"):
            # Embedded mode: running inside Patinae GUI (plugin set this up)
            _cmd = Cmd(sys._patinae_backend)
        else:
            # Standalone mode: own Session + CommandExecutor
            from ._patinae import _create_standalone_backend

            _cmd = Cmd(_create_standalone_backend())
    return _cmd


def __getattr__(name):
    if name == "cmd":
        return _get_cmd()
    raise AttributeError(f"module 'patinae' has no attribute {name!r}")
