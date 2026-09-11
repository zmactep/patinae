"""
Unified Cmd class for Patinae.

Works with any backend (StandaloneBackend or PluginBackend).
All commands are thin wrappers that build command strings
and delegate to ``backend.execute()``.
"""


class Cmd:
    """Patinae command interface.

    Wraps a backend (standalone or embedded) and provides
    a PyMOL-compatible API.
    """

    def __init__(self, backend):
        self._backend = backend

    # =====================================================================
    # File I/O
    # =====================================================================

    def load(self, filename, object=None, state=0, format=None, quiet=True, *, wait=True):
        """Load a molecular file."""
        cmd_str = f"load {filename}"
        if object:
            cmd_str += f", {object}"
        if state:
            cmd_str += f", state={state}"
        if format:
            cmd_str += f", format={format}"
        return self._execute(cmd_str, quiet, wait=wait)

    def save(self, filename, selection="all", state=-1, format=None, quiet=True, *, wait=True):
        """Save molecular data to a file."""
        cmd_str = f"save {filename}, {selection}"
        if state != -1:
            cmd_str += f", state={state}"
        if format:
            cmd_str += f", format={format}"
        return self._execute(cmd_str, quiet, wait=wait)

    def fetch(self, code, name=None, state=0, type_="cif", quiet=True, *, wait=True):
        """Fetch a structure from the PDB."""
        cmd_str = f"fetch {code}"
        if name:
            cmd_str += f", {name}"
        if type_ != "cif":
            cmd_str += f", type={type_}"
        return self._execute(cmd_str, quiet, wait=wait)

    # =====================================================================
    # Display
    # =====================================================================

    def show(self, representation, selection="all", *, wait=True):
        """Show a representation."""
        return self._execute(f"show {representation}, {selection}", wait=wait)

    def hide(self, representation, selection="all", *, wait=True):
        """Hide a representation."""
        return self._execute(f"hide {representation}, {selection}", wait=wait)

    def show_as(self, representation, selection="all", *, wait=True):
        """Show only the specified representation (hide others)."""
        return self._execute(f"as {representation}, {selection}", wait=wait)

    def color(self, color, selection="all", *, wait=True):
        """Color a selection."""
        return self._execute(f"color {color}, {selection}", wait=wait)

    def bg_color(self, color, *, wait=True):
        """Set background color."""
        return self._execute(f"bg_color {color}", wait=wait)

    def label(self, selection, expression, object=None, quiet=True, *, wait=True):
        """Create or append an atom-anchored label collection.

        ``expression`` uses the Patinae label expression syntax. Property
        names such as ``name`` or ``resi`` are stored at creation time. A
        literal string must include its command quotes, for example
        ``'\"active site\"'``.

        Omitting ``object`` creates a new auto-named ``LabelObject``. Passing
        an existing label object name appends entities in selection order.
        """
        cmd_str = f"label {selection}, {expression}"
        if object is not None:
            cmd_str += f", object={object}"
        return self._execute(cmd_str, quiet, wait=wait)

    # =====================================================================
    # Selections
    # =====================================================================

    def select(self, name, selection, *, wait=True):
        """Create a named selection."""
        return self._execute(f"select {name}, {selection}", wait=wait)

    def deselect(self, *, wait=True):
        """Deselect all."""
        return self._execute("deselect", wait=wait)

    def count_atoms(self, selection="all"):
        """Count atoms in a selection."""
        return self._backend.count_atoms(selection)

    # =====================================================================
    # Viewing
    # =====================================================================

    def zoom(self, selection="all", buffer=0.0, complete=0, *, wait=True):
        """Zoom to a selection."""
        return self._execute(f"zoom {selection}, {buffer}, {complete}", wait=wait)

    def center(self, selection="all", *, wait=True):
        """Center on a selection."""
        return self._execute(f"center {selection}", wait=wait)

    def orient(self, selection="all", *, wait=True):
        """Orient on a selection."""
        return self._execute(f"orient {selection}", wait=wait)

    def reset(self, *, wait=True):
        """Reset the view."""
        return self._execute("reset", wait=wait)

    # =====================================================================
    # Movie / animation
    # =====================================================================

    def mset(self, specification, quiet=True, *, wait=True):
        """Set movie frames from a PyMOL-style frame specification."""
        return self._execute(f"mset {specification}", quiet, wait=wait)

    def madd(self, specification, quiet=True, *, wait=True):
        """Append movie frames using the mset specification syntax."""
        return self._execute(f"madd {specification}", quiet, wait=wait)

    def mplay(self, quiet=True, *, wait=True):
        """Start movie playback."""
        return self._execute("mplay", quiet, wait=wait)

    def mstop(self, quiet=True, *, wait=True):
        """Stop movie playback and rewind to frame 1."""
        return self._execute("mstop", quiet, wait=wait)

    def mpause(self, quiet=True, *, wait=True):
        """Pause movie playback."""
        return self._execute("mpause", quiet, wait=wait)

    def mtoggle(self, quiet=True, *, wait=True):
        """Toggle movie playback."""
        return self._execute("mtoggle", quiet, wait=wait)

    def frame(self, frame_number, quiet=True, *, wait=True):
        """Go to a 1-based movie frame."""
        return self._execute(f"frame {frame_number}", quiet, wait=wait)

    def forward(self, quiet=True, *, wait=True):
        """Advance one movie frame."""
        return self._execute("forward", quiet, wait=wait)

    def backward(self, quiet=True, *, wait=True):
        """Go back one movie frame."""
        return self._execute("backward", quiet, wait=wait)

    def rewind(self, quiet=True, *, wait=True):
        """Go to the first movie frame."""
        return self._execute("rewind", quiet, wait=wait)

    def ending(self, quiet=True, *, wait=True):
        """Go to the last movie frame."""
        return self._execute("ending", quiet, wait=wait)

    def rock(self, mode=None, quiet=True, *, wait=True):
        """Toggle or explicitly set rock animation."""
        cmd_str = "rock" if mode is None else f"rock {mode}"
        return self._execute(cmd_str, quiet, wait=wait)

    def mview(self, action, frame=None, scene=None, object=None, state=None, quiet=True, *, wait=True):
        """Store, recall, clear, or interpolate movie keyframes."""
        parts = [f"mview {action}"]
        if frame is not None:
            parts.append(str(frame))
        if scene is not None:
            parts.append(f"scene={scene}")
        if object is not None:
            parts.append(f"object={object}")
        if state is not None:
            parts.append(f"state={state}")
        return self._execute(", ".join(parts), quiet, wait=wait)

    def mpng(self, prefix, first=None, last=None, preserve=0, width=0, height=0, quiet=True, *, wait=True):
        """Render movie frames to a PNG sequence."""
        args = [str(prefix)]
        if first is not None:
            args.append(f"first={first}")
        if last is not None:
            args.append(f"last={last}")
        if preserve:
            args.append(f"preserve={preserve}")
        if width:
            args.append(f"width={width}")
        if height:
            args.append(f"height={height}")
        return self._execute("mpng " + ", ".join(args), quiet, wait=wait)

    def mproduce(
        self,
        filename,
        first=None,
        last=None,
        preserve=0,
        quality=90,
        width=0,
        height=0,
        quiet=True,
        *,
        wait=True,
    ):
        """Export a movie to a video file."""
        args = [str(filename)]
        if first is not None:
            args.append(f"first={first}")
        if last is not None:
            args.append(f"last={last}")
        if preserve:
            args.append(f"preserve={preserve}")
        if quality != 90:
            args.append(f"quality={quality}")
        if width:
            args.append(f"width={width}")
        if height:
            args.append(f"height={height}")
        return self._execute("mproduce " + ", ".join(args), quiet, wait=wait)

    def get_movie_state(self):
        """Return the backend movie state snapshot."""
        return self._backend.get_movie_state()

    def update_animations(self, dt):
        """Explicitly advance movie, rock, and camera animations by dt seconds."""
        return self._backend.update_animations(dt)

    # =====================================================================
    # Objects
    # =====================================================================

    def delete(self, name, *, wait=True):
        """Delete an object or selection."""
        return self._execute(f"delete {name}", wait=wait)

    def get_names(self):
        """Get the names of all loaded objects."""
        return self._backend.get_names()

    def enable(self, name, *, wait=True):
        """Enable (show) an object."""
        return self._execute(f"enable {name}", wait=wait)

    def disable(self, name, *, wait=True):
        """Disable (hide) an object."""
        return self._execute(f"disable {name}", wait=wait)

    # =====================================================================
    # Data access
    # =====================================================================

    def get_model(self, name):
        """Get a molecular object by name."""
        return self._backend.get_model(name)

    def get_label(self, name):
        """Get an immutable snapshot of a first-class label object."""
        from ._labels import LabelObject

        value = self._backend.get_label(name)
        if isinstance(value, LabelObject):
            return value
        return LabelObject.from_mapping(value)

    # =====================================================================
    # Settings
    # =====================================================================

    def set(self, name, value, selection=None, quiet=True, *, wait=True):
        """Set a setting."""
        cmd_str = f"set {name}, {value}"
        if selection:
            cmd_str += f", {selection}"
        return self._execute(cmd_str, quiet, wait=wait)

    def get(self, name, selection=None, quiet=True, *, wait=True):
        """Get a setting value."""
        cmd_str = f"get {name}"
        if selection:
            cmd_str += f", {selection}"
        return self._execute(cmd_str, quiet, wait=wait)

    # =====================================================================
    # Image output
    # =====================================================================

    def png(self, filename, width=0, height=0, ray=0, quiet=True, *, wait=True):
        """Save a PNG image."""
        cmd_str = f"png {filename}"
        if width:
            cmd_str += f", {width}"
        if height:
            cmd_str += f", {height}"
        if ray:
            cmd_str += f", ray={ray}"
        return self._execute(cmd_str, quiet, wait=wait)

    def ray(self, width=0, height=0, quiet=True, *, wait=True):
        """Ray trace the scene."""
        cmd_str = "ray"
        if width:
            cmd_str += f" {width}"
        if height:
            cmd_str += f", {height}"
        return self._execute(cmd_str, quiet, wait=wait)

    def get_viewport_image(self):
        """Get viewport image as numpy array (H, W, 4) uint8, or None."""
        return self._backend.get_viewport_image()

    def set_viewport_image(self, array):
        """Set viewport image from numpy array (H, W, 4) uint8."""
        self._backend.set_viewport_image(array)

    def clear_viewport_image(self):
        """Clear viewport image overlay."""
        self._backend.clear_viewport_image()

    def is_interrupt_requested(self):
        """Return True when the embedded host requested script cancellation."""
        checker = getattr(self._backend, "is_interrupt_requested", None)
        return bool(checker()) if checker is not None else False

    def should_stop(self):
        """Alias for is_interrupt_requested()."""
        return self.is_interrupt_requested()

    # =====================================================================
    # Control
    # =====================================================================

    def refresh(self, *, wait=True):
        """Refresh the scene."""
        return self._execute("refresh", wait=wait)

    def rebuild(self, selection="all", *, wait=True):
        """Rebuild representations."""
        return self._execute(f"rebuild {selection}", wait=wait)

    def reinitialize(self, what="everything", *, wait=True):
        """Reinitialize the scene."""
        return self._execute(f"reinitialize {what}", wait=wait)

    reinit = reinitialize

    # =====================================================================
    # Iteration
    # =====================================================================

    def iterate(self, selection, expression, space=None):
        """Execute expression for each atom in selection (read-only).

        Atom properties available as local variables:
            name, resn, resv, resi, chain, segi, alt, elem,
            b, q, vdw, partial_charge, formal_charge,
            ss, color, type, hetatm, index, ID, rank, model,
            x, y, z

        Use ``space`` to accumulate data across atoms::

            mylist = []
            cmd.iterate("name CA", "mylist.append(b)", space=locals())
        """
        return self._backend.iterate(selection, expression, space)

    def alter(self, selection, expression, space=None):
        """Execute expression for each atom in selection (read-write).

        Modifiable properties:
            name, resn, resv, chain, segi, alt, elem,
            b, q, vdw, partial_charge, formal_charge,
            ss, color, type

        Coordinates (x, y, z) are read-only — use alter_state for those.

        Example::

            cmd.alter("all", "b=0.0")
            cmd.alter("chain A", "chain='B'")
        """
        return self._backend.alter(selection, expression, space)

    # =====================================================================
    # Keybindings
    # =====================================================================

    def set_key(self, key, callback):
        """Bind a key or key combination to a Python callback.

        The callback is called with no arguments when the key is pressed.
        Rebinding the same key replaces the previous callback.

        Examples::

            cmd.set_key("F1", my_help_fn)
            cmd.set_key("ctrl+s", lambda: cmd.save("output.pdb"))
            cmd.set_key("ctrl+shift+r", reload_fn)
        """
        if not callable(callback):
            raise TypeError("callback must be callable")
        self._backend.set_key(key, callback)

    def unset_key(self, key):
        """Unbind a key or key combination.

        No error if the key was not bound.

        Examples::

            cmd.unset_key("ctrl+s")
        """
        self._backend.unset_key(key)

    # =====================================================================
    # General execution
    # =====================================================================

    def do(self, command, quiet=False, *, wait=True):
        """Execute an arbitrary command string."""
        return self._execute(command, quiet, wait=wait)

    def run(self, filename, quiet=False, *, wait=True):
        """Run a script, waiting for its body and all child tasks by default."""
        return self._execute(f"run {filename}", quiet, wait=wait)

    def _execute(self, command, quiet=False, *, wait=True):
        from .tasks import CommandError, TaskError

        reply = self._backend.execute(command, quiet)
        if not isinstance(reply, dict) or "task_ids" not in reply or "result" not in reply:
            raise RuntimeError("Backend returned an unsupported command protocol")
        failure = reply["result"].get("Err")
        if failure is not None:
            raise CommandError(str(failure), reply)
        if wait:
            for task_id in reply["task_ids"]:
                try:
                    snapshot = self._backend.wait_task(str(task_id), None)
                except Exception as error:
                    raise TaskError(str(error), task_id, reply,
                                    code=getattr(error, "code", None)) from error
                if snapshot["state"] != "succeeded":
                    outcome = snapshot.get("outcome") or {}
                    status = outcome.get("status") or {}
                    error = status.get("error") or {}
                    raise TaskError(error.get("message", "Task " + snapshot["state"]),
                                    task_id, reply, snapshot)
        return reply

    def __repr__(self):
        return "<patinae.Cmd>"
