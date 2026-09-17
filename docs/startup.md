# Native startup

The first Slint frame precedes plugin registration and viewport pipeline preparation.
The rendering callback only captures owned GPU handles and signals the first frame.
The host timer then starts preparation on a worker, polls without waiting, and attaches
the GPU resources on the main thread. Live scene representations are created there;
they are never transferred between threads. The synchronous `RenderState` constructor
uses the same preparation and attachment stages.

While the notification says “Preparing graphics…”, commands that do not need a
renderer and built-in file loading remain available. Rendering uses the current scene
when preparation completes. Commands requiring a renderer can report it unavailable;
they are not replayed. Startup `patinaerc`, argument files, and deferred opens wait
for both plugin loading and graphics preparation. Closing or recreating the window
discards its pending result without joining the worker.

Info logs report first-frame time, GPU preparation stages, and attachment time.
Pipeline preparation can vary substantially between launches; the logs distinguish
that work from window creation and plugin registration. They do not by themselves
prove a driver cache miss.

For a responsiveness check, launch an isolated source build with
`PATINAE_DEBUG_RENDER_PREPARE_DELAY_MS=30000`. This diagnostic delay affects only the
worker, is capped at 30 seconds, and is disabled by default. Check that the first
frame and an IPC built-in command complete before `Viewport attached`. Also verify
that a scene loaded during preparation renders after attachment and that closing
during preparation exits promptly. Do not replace the installed application for
these checks.
