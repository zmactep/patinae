"""Viewer widget — anywidget subclass that embeds the WASM molecular viewer."""

import base64
import pathlib

import anywidget
import traitlets

_STATIC = pathlib.Path(__file__).parent / "static"


class Viewer(anywidget.AnyWidget):
    """Interactive 3D molecular viewer for Jupyter notebooks.

    Uses the Patinae WASM + WebGPU viewer running in the browser.
    Requires a WebGPU-capable browser (Chrome, Edge, or Firefox Nightly).

    Usage::

        from patinae.widget import Viewer

        view = Viewer()
        view.show()
        cmd = view.get_cmd()
        cmd.fetch("1CRN")
        cmd.show("cartoon")
        cmd.color("green", "chain A")
    """

    _esm = pathlib.Path(__file__).parent / "_frontend.js"

    # --- Synced traitlets ---

    # WASM glue JS source (~52KB string)
    _glue_js = traitlets.Unicode("").tag(sync=True)

    # WASM binary as base64 (~4MB string, sent once on init)
    _wasm_b64 = traitlets.Unicode("").tag(sync=True)

    # Layout
    _width = traitlets.Unicode("100%").tag(sync=True)
    _height = traitlets.Unicode("500px").tag(sync=True)

    # Picking (click-to-select atoms)
    _picking = traitlets.Bool(False).tag(sync=True)

    def __init__(self, width="100%", height="500px", picking=False, **kwargs):
        glue_js = (_STATIC / "patinae_web_glue.js").read_text()
        wasm_b64 = base64.b64encode(
            (_STATIC / "patinae_web_bg.wasm").read_bytes()
        ).decode("ascii")
        super().__init__(_glue_js=glue_js, _wasm_b64=wasm_b64, **kwargs)
        self._width = width
        self._height = height
        self._picking = picking
        from ._backend import WidgetBackend
        from ..tasks import Tasks

        self._backend = WidgetBackend(self)
        self._tasks = Tasks(self._backend)
        self._cmd = None

    @property
    def tasks(self):
        """Task observation and cancellation bound to this viewer's session."""
        return self._tasks

    def show(self):
        """Display the widget in the notebook."""
        from IPython.display import display

        display(self)

    def get_cmd(self):
        """Return a Cmd object that proxies commands to the browser WASM viewer.

        The returned object has the same API as ``patinae.cmd``.
        """
        from .._cmd import Cmd
        if self._cmd is None:
            self._cmd = Cmd(self._backend)
        return self._cmd
