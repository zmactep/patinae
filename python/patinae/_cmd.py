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

    def load(self, filename, object=None, state=0, format=None, quiet=True):
        """Load a molecular file."""
        cmd_str = f"load {filename}"
        if object:
            cmd_str += f", {object}"
        if state:
            cmd_str += f", state={state}"
        if format:
            cmd_str += f", format={format}"
        self._backend.execute(cmd_str, quiet)

    def save(self, filename, selection="all", state=-1, format=None, quiet=True):
        """Save molecular data to a file."""
        cmd_str = f"save {filename}, {selection}"
        if state != -1:
            cmd_str += f", state={state}"
        if format:
            cmd_str += f", format={format}"
        self._backend.execute(cmd_str, quiet)

    def fetch(self, code, name=None, state=0, type_="cif", sync=False, quiet=True):
        """Fetch a structure from the PDB."""
        cmd_str = f"fetch {code}"
        if name:
            cmd_str += f", {name}"
        if type_ != "cif":
            cmd_str += f", type={type_}"
        if sync:
            cmd_str += ", async=0"
        self._backend.execute(cmd_str, quiet)

    # =====================================================================
    # Display
    # =====================================================================

    def show(self, representation, selection="all"):
        """Show a representation."""
        self._backend.execute(f"show {representation}, {selection}")

    def hide(self, representation, selection="all"):
        """Hide a representation."""
        self._backend.execute(f"hide {representation}, {selection}")

    def show_as(self, representation, selection="all"):
        """Show only the specified representation (hide others)."""
        self._backend.execute(f"as {representation}, {selection}")

    def color(self, color, selection="all"):
        """Color a selection."""
        self._backend.execute(f"color {color}, {selection}")

    def bg_color(self, color):
        """Set background color."""
        self._backend.execute(f"bg_color {color}")

    def label(self, selection, expression, object=None, quiet=True):
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
        self._backend.execute(cmd_str, quiet)

    # =====================================================================
    # Selections
    # =====================================================================

    def select(self, name, selection):
        """Create a named selection."""
        self._backend.execute(f"select {name}, {selection}")

    def deselect(self):
        """Deselect all."""
        self._backend.execute("deselect")

    def count_atoms(self, selection="all"):
        """Count atoms in a selection."""
        return self._backend.count_atoms(selection)

    # =====================================================================
    # Viewing
    # =====================================================================

    def zoom(self, selection="all", buffer=0.0, complete=0):
        """Zoom to a selection."""
        self._backend.execute(f"zoom {selection}, {buffer}, {complete}")

    def center(self, selection="all"):
        """Center on a selection."""
        self._backend.execute(f"center {selection}")

    def orient(self, selection="all"):
        """Orient on a selection."""
        self._backend.execute(f"orient {selection}")

    def reset(self):
        """Reset the view."""
        self._backend.execute("reset")

    # =====================================================================
    # Movie / animation
    # =====================================================================

    def mset(self, specification, quiet=True):
        """Set movie frames from a PyMOL-style frame specification."""
        self._backend.execute(f"mset {specification}", quiet)

    def madd(self, specification, quiet=True):
        """Append movie frames using the mset specification syntax."""
        self._backend.execute(f"madd {specification}", quiet)

    def mplay(self, quiet=True):
        """Start movie playback."""
        self._backend.execute("mplay", quiet)

    def mstop(self, quiet=True):
        """Stop movie playback and rewind to frame 1."""
        self._backend.execute("mstop", quiet)

    def mpause(self, quiet=True):
        """Pause movie playback."""
        self._backend.execute("mpause", quiet)

    def mtoggle(self, quiet=True):
        """Toggle movie playback."""
        self._backend.execute("mtoggle", quiet)

    def frame(self, frame_number, quiet=True):
        """Go to a 1-based movie frame."""
        self._backend.execute(f"frame {frame_number}", quiet)

    def forward(self, quiet=True):
        """Advance one movie frame."""
        self._backend.execute("forward", quiet)

    def backward(self, quiet=True):
        """Go back one movie frame."""
        self._backend.execute("backward", quiet)

    def rewind(self, quiet=True):
        """Go to the first movie frame."""
        self._backend.execute("rewind", quiet)

    def ending(self, quiet=True):
        """Go to the last movie frame."""
        self._backend.execute("ending", quiet)

    def rock(self, mode=None, quiet=True):
        """Toggle or explicitly set rock animation."""
        cmd_str = "rock" if mode is None else f"rock {mode}"
        self._backend.execute(cmd_str, quiet)

    def mview(self, action, frame=None, scene=None, object=None, state=None, quiet=True):
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
        self._backend.execute(", ".join(parts), quiet)

    def mpng(self, prefix, first=None, last=None, preserve=0, width=0, height=0, quiet=True):
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
        self._backend.execute("mpng " + ", ".join(args), quiet)

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
        self._backend.execute("mproduce " + ", ".join(args), quiet)

    def get_movie_state(self):
        """Return the backend movie state snapshot."""
        return self._backend.get_movie_state()

    def update_animations(self, dt):
        """Explicitly advance movie, rock, and camera animations by dt seconds."""
        return self._backend.update_animations(dt)

    # =====================================================================
    # Objects
    # =====================================================================

    def delete(self, name):
        """Delete an object or selection."""
        self._backend.execute(f"delete {name}")

    def get_names(self):
        """Get the names of all loaded objects."""
        return self._backend.get_names()

    def enable(self, name):
        """Enable (show) an object."""
        self._backend.execute(f"enable {name}")

    def disable(self, name):
        """Disable (hide) an object."""
        self._backend.execute(f"disable {name}")

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

    def set(self, name, value, selection=None, quiet=True):
        """Set a setting."""
        cmd_str = f"set {name}, {value}"
        if selection:
            cmd_str += f", {selection}"
        self._backend.execute(cmd_str, quiet)

    def get(self, name, selection=None, quiet=True):
        """Get a setting value."""
        cmd_str = f"get {name}"
        if selection:
            cmd_str += f", {selection}"
        self._backend.execute(cmd_str, quiet)

    # =====================================================================
    # Image output
    # =====================================================================

    def png(self, filename, width=0, height=0, ray=0, quiet=True):
        """Save a PNG image."""
        cmd_str = f"png {filename}"
        if width:
            cmd_str += f", {width}"
        if height:
            cmd_str += f", {height}"
        if ray:
            cmd_str += f", ray={ray}"
        self._backend.execute(cmd_str, quiet)

    def ray(self, width=0, height=0, quiet=True):
        """Ray trace the scene."""
        cmd_str = "ray"
        if width:
            cmd_str += f" {width}"
        if height:
            cmd_str += f", {height}"
        self._backend.execute(cmd_str, quiet)

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

    def refresh(self):
        """Refresh the scene."""
        self._backend.execute("refresh")

    def rebuild(self, selection="all"):
        """Rebuild representations."""
        self._backend.execute(f"rebuild {selection}")

    def reinitialize(self, what="everything"):
        """Reinitialize the scene."""
        self._backend.execute(f"reinitialize {what}")

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

    def do(self, command, quiet=False):
        """Execute an arbitrary command string."""
        self._backend.execute(command, quiet)

    def __repr__(self):
        return "<patinae.Cmd>"
