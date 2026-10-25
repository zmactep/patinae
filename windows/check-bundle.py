"""Check the distributed interpreter and native libraries without uv or system Python."""

import ctypes
import importlib.metadata
import os
import pathlib
import platform
import sys

import patinae

bundle = pathlib.Path(sys.argv[1]).resolve()
expected_arch = sys.argv[2]
expected_version = sys.argv[3]
runtime = bundle / "python"
assert pathlib.Path(sys.prefix).resolve() == runtime, sys.prefix
assert pathlib.Path(sys.base_prefix).resolve() == runtime, sys.base_prefix
assert pathlib.Path(patinae.__file__).resolve().is_relative_to(runtime)
assert importlib.metadata.version("patinae") == expected_version
assert sys.version_info[:2] == (3, 13), sys.version
assert platform.machine().lower() == {"x86_64": "amd64", "arm64": "arm64"}[expected_arch]
for entry in sys.path:
    if entry:
        assert pathlib.Path(entry).resolve().is_relative_to(runtime), entry

# Match LoadLibrary's search behavior used by the native host. Loading alone
# checks architecture and DLL dependencies; GUI/plugin registration is a separate
# acceptance check on a Windows desktop.
libraries = []
for name in ("hello", "raytracer", "python", "ai"):
    library = ctypes.WinDLL(str(bundle / "plugins" / f"{name}_plugin.dll"), winmode=0)
    ctypes.c_uint32.in_dll(library, "PATINAE_PLUGIN_DECLARATION")
    libraries.append(library)
print("Bundled Python, Patinae wheel and all four plugin DLLs passed.")
if len(sys.argv) == 5:
    # In the launcher check, Python temporarily stands in for patinae.exe.
    # This exercises real CreateProcess argument forwarding and environment setup.
    assert not ctypes.windll.kernel32.GetConsoleWindow(), "Unexpected console window"
    marker = pathlib.Path(sys.argv[4])
    temporary_marker = marker.with_suffix(".tmp")
    temporary_marker.write_text(f"passed:{os.getpid()}", encoding="utf-8")
    temporary_marker.replace(marker)
