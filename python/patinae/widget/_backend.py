"""WidgetBackend — proxies the StandaloneBackend interface to the browser WASM viewer."""

import os
import time
import threading


class WidgetBackend:
    """Backend that sends commands to the browser-side WASM WebViewer.

    Implements the same duck-typed interface as StandaloneBackend so the
    existing Cmd class works unchanged.
    """

    def __init__(self, widget):
        self._widget = widget
        self._query_lock = threading.Lock()
        self._response_event = threading.Event()
        self._last_response = None
        widget.observe(self._on_query_response, names=["_query_response"])

    # -----------------------------------------------------------------
    # Command execution
    # -----------------------------------------------------------------

    def execute(self, command, silent=False):
        """Send a command to the browser for execution."""
        if self._is_local_load(command):
            self._load_local_file(command)
            return
        w = self._widget
        w._command = command
        w._command_id += 1

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
        pass

    def clear_viewport_image(self):
        pass

    # -----------------------------------------------------------------
    # Keybindings (no-op — browser handles its own input)
    # -----------------------------------------------------------------

    def set_key(self, key, callback):
        pass

    def unset_key(self, key):
        pass

    # -----------------------------------------------------------------
    # Internal helpers
    # -----------------------------------------------------------------

    def _query(self, method, params, timeout=10.0):
        """Send a synchronous query and block until the browser responds."""
        with self._query_lock:
            self._response_event.clear()
            self._last_response = None

            query_id = self._widget._query_id + 1
            self._widget._query_request = {
                "id": query_id,
                "method": method,
                "params": params,
            }
            self._widget._query_id = query_id

            # Poll the kernel message loop so comm messages are processed.
            deadline = time.time() + timeout
            try:
                import IPython

                kernel = IPython.get_ipython().kernel
                while not self._response_event.is_set():
                    if time.time() > deadline:
                        raise TimeoutError(
                            f"Widget query '{method}' timed out after {timeout}s. "
                            "Is the widget visible in the notebook?"
                        )
                    kernel.do_one_iteration()
                    time.sleep(0.01)
            except (ImportError, AttributeError):
                # Not in IPython / no kernel — fall back to plain wait
                if not self._response_event.wait(timeout=timeout):
                    raise TimeoutError(
                        f"Widget query '{method}' timed out after {timeout}s."
                    )

            resp = self._last_response
            if resp and resp.get("error"):
                raise RuntimeError(resp["error"])
            return resp.get("result") if resp else None

    def _on_query_response(self, change):
        """Traitlet observer — fires when JS sets _query_response."""
        resp = change["new"]
        if resp and resp.get("id") == self._widget._query_id:
            self._last_response = resp
            self._response_event.set()

    def _is_local_load(self, command):
        """Check if this is a 'load' command with a local file path."""
        parts = command.strip().split(None, 1)
        if len(parts) < 2:
            return False
        if parts[0].lower() != "load":
            return False
        first_arg = parts[1].split(",")[0].strip()
        return not first_arg.startswith(("http://", "https://", "ftp://"))

    def _load_local_file(self, command):
        """Read a local file and send bytes to the browser via binary message."""
        parts = command.strip().split(None, 1)
        arg_str = parts[1]
        args = [a.strip() for a in arg_str.split(",")]
        filepath = args[0]

        # Determine object name and format from filename
        basename = os.path.basename(filepath)
        if basename.lower().endswith(".gz"):
            basename = basename[:-3]
        name = args[1] if len(args) > 1 and args[1] else basename.rsplit(".", 1)[0]
        ext = basename.rsplit(".", 1)[-1].lower() if "." in basename else "pdb"

        with open(filepath, "rb") as f:
            data = f.read()

        self._widget.send(
            {"type": "load_data", "name": name, "format": ext},
            buffers=[data],
        )
