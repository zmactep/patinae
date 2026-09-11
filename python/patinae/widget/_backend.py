"""WidgetBackend — proxies the StandaloneBackend interface to the browser WASM viewer."""

from pathlib import Path
import time
import threading
from ..tasks import TaskApiError


class WidgetBackend:
    """Backend that sends commands to the browser-side WASM WebViewer.

    Implements the same duck-typed interface as StandaloneBackend so the
    existing Cmd class works unchanged.
    """

    def __init__(self, widget):
        self._widget = widget
        self._next_id = 0
        self._pending = {}
        self._lock = threading.Lock()
        self._closed = False
        self._view_id = None
        self._view_ready = threading.Event()
        widget.on_msg(self._on_message)

    # -----------------------------------------------------------------
    # Command execution
    # -----------------------------------------------------------------

    def execute(self, command, quiet=False):
        """Execute using the viewer's parser, including notebook-local files."""
        paths = self._query("command_files", {"command": command})
        files = {}
        pending = list(paths)
        while pending:
            path = pending.pop(0)
            if path in files:
                continue
            data = Path(path).expanduser().read_bytes()
            files[path] = data
            if Path(path).suffix.lower() in (".pml", ".patinaerc"):
                nested = self._query("command_files", {"command": data.decode("utf-8"), "script_path": path})
                pending.extend(name for name in nested if name not in files)
        paths = list(files)
        buffers = list(files.values())
        return self._query("execute", {"command": command, "quiet": quiet,
                                       "files": paths}, buffers=buffers)

    def get_task(self, task_id):
        return self._query("get_task", {"id": task_id})

    def list_tasks(self, filters):
        return self._query("list_tasks", filters)

    def cancel_task(self, task_id):
        return self._query("cancel_task", {"id": task_id})

    def wait_task(self, task_id, timeout=None):
        # A finite transport liveness timeout also detects a disconnected view.
        # Long task waits use repeated bounded server waits, never cancel work.
        deadline = None if timeout is None else time.monotonic() + timeout
        while True:
            remaining = None if deadline is None else max(0, deadline - time.monotonic())
            interval = 5.0 if remaining is None else min(5.0, remaining)
            result = self._query("wait_task", {"id": task_id, "timeout_ms": interval * 1000},
                                 timeout=interval + 10.0)
            if result is not None:
                return result
            if deadline is not None and time.monotonic() >= deadline:
                raise TaskApiError("timeout", f"Waiting for task {task_id} timed out")

    # -----------------------------------------------------------------
    # Synchronous queries
    # -----------------------------------------------------------------

    def count_atoms(self, selection="all"):
        """Count atoms in a selection (synchronous round-trip to browser)."""
        return self._query("count_atoms", {"selection": selection})

    def get_names(self):
        """Get names of all loaded objects (synchronous round-trip to browser)."""
        result = self._query("get_names", {})
        return list(result) if result else []

    def get_label(self, name):
        """Get a read-only label object snapshot from the browser viewer."""
        result = self._query("get_label", {"name": str(name)})
        if result is None:
            raise KeyError(f"label object {name!r} not found")
        return result

    def get_movie_state(self):
        """Get current movie state from the browser viewer."""
        return self._query("get_movie_state", {})

    def update_animations(self, dt):
        """Explicitly advance browser-side animations."""
        return bool(self._query("update_animations", {"dt": float(dt)}))

    # -----------------------------------------------------------------
    # Not yet supported in widget mode
    # -----------------------------------------------------------------

    def get_model(self, name):
        raise NotImplementedError(
            "get_model() is not yet supported in widget mode. "
            "Use count_atoms(), get_names(), or the browser viewer directly."
        )

    def iterate(self, selection, expression, space=None):
        raise NotImplementedError(
            "iterate() is not yet supported in widget mode."
        )

    def alter(self, selection, expression, space=None):
        raise NotImplementedError(
            "alter() is not yet supported in widget mode."
        )

    # -----------------------------------------------------------------
    # Viewport image (not applicable — rendering is in the browser)
    # -----------------------------------------------------------------

    def get_viewport_image(self):
        return None

    def set_viewport_image(self, array):
        raise NotImplementedError("This operation is unavailable in the notebook viewer")

    def clear_viewport_image(self):
        raise NotImplementedError("This operation is unavailable in the notebook viewer")

    # -----------------------------------------------------------------
    # Keybindings (no-op — browser handles its own input)
    # -----------------------------------------------------------------

    def set_key(self, key, callback):
        raise NotImplementedError("This operation is unavailable in the notebook viewer")

    def unset_key(self, key):
        raise NotImplementedError("This operation is unavailable in the notebook viewer")

    # -----------------------------------------------------------------
    # Internal helpers
    # -----------------------------------------------------------------

    def _query(self, method, params, timeout=10.0, buffers=None):
        """Correlate one request while servicing UI comms during a cell."""
        from jupyter_ui_poll import ui_events

        event = threading.Event()
        slot = {"event": event}
        with self._lock:
            if self._closed:
                raise RuntimeError("Widget frontend disconnected")
            self._next_id += 1
            request_id = self._next_id
            self._pending[request_id] = slot
        try:
            deadline = time.monotonic() + timeout
            with ui_events() as poll:
                while not self._view_ready.is_set():
                    if time.monotonic() >= deadline:
                        raise TimeoutError("Widget frontend did not connect")
                    poll(10)
                    self._view_ready.wait(0.01)
                self._widget.send({"protocol": 1, "view_id": self._view_id,
                                   "id": request_id, "method": method, "params": params},
                                  buffers=buffers or [])
                while not event.is_set():
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        raise TimeoutError(f"Widget request {method!r} timed out; frontend may be disconnected")
                    poll(10)
                    event.wait(min(0.01, remaining))
            response = slot["response"]
            if response.get("error") is not None:
                error = response["error"]
                if isinstance(error, dict) and error.get("code") == "timeout":
                    return None
                if isinstance(error, dict):
                    raise TaskApiError(error.get("code", "transport_error"),
                                       error.get("message", str(error)))
                raise TaskApiError("transport_error", str(error))
            return response["result"]
        finally:
            with self._lock:
                self._pending.pop(request_id, None)

    def _on_message(self, widget, content, buffers):
        if content.get("protocol") != 1:
            return
        with self._lock:
            if content.get("event") == "ready":
                if self._view_id is None:
                    self._view_id = content["view_id"]
                    self._view_ready.set()
                return
            if content.get("view_id") != self._view_id:
                return
            if content.get("event") == "disconnected":
                self._closed = True
                for slot in self._pending.values():
                    slot["response"] = {"error": "Widget frontend disconnected"}
                    slot["event"].set()
                return
            slot = self._pending.get(content.get("id"))
            if slot is not None and not slot["event"].is_set():
                slot["response"] = content
                slot["event"].set()
