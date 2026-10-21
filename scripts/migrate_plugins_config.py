#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Move an explicitly selected plugins.toml into the application config.toml."""

import argparse
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import tomllib


def read_document(path: Path, *, optional: bool = False) -> tuple[bytes | None, dict]:
    if optional and not os.path.lexists(path):
        return None, {}
    raw = path.read_bytes()
    try:
        return raw, tomllib.loads(raw.decode("utf-8"))
    except ValueError as error:
        raise ValueError(f"Invalid {path}: {error}") from error


def validate_plugins(path: Path, entries: object) -> None:
    if not isinstance(entries, list):
        raise ValueError(f"Invalid {path}: plugin must be a list")
    for index, entry in enumerate(entries, 1):
        if (
            not isinstance(entry, dict)
            or set(entry) != {"path"}
            or not isinstance(entry["path"], str)
            or not entry["path"].strip()
        ):
            raise ValueError(f"Invalid {path}: plugin {index} needs a nonempty path only")


def atomic_write(path: Path, content: bytes, previous: bytes | None) -> None:
    # Follow an existing config symlink, keeping the link itself intact.
    target = path.resolve(strict=True) if path.is_symlink() else path
    target.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=target.parent, delete=False) as output:
            temporary = Path(output.name)
            if previous is not None:
                os.chmod(temporary, stat.S_IMODE(target.stat().st_mode))
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
        if previous is None:
            # A new config must not overwrite one created while the script ran.
            os.link(temporary, target)
        else:
            if target.read_bytes() != previous:
                raise ValueError(f"Configuration changed during migration: {path}")
            os.replace(temporary, target)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def migrate(source: Path, destination: Path) -> bool:
    """Return False when an existing plugin list takes precedence."""
    original, config = read_document(destination, optional=True)
    if "plugins" in config or config.get("repl_position", "bottom") not in ("top", "bottom"):
        raise ValueError(f"Invalid {destination}: check repl_position and use plugin, not plugins")
    if "plugin" in config:
        validate_plugins(destination, config["plugin"])
        return False
    if source.resolve() == destination.resolve() or (
        original is not None and source.samefile(destination)
    ):
        raise ValueError(f"Source and destination must differ: {source}")

    old_bytes, manifest = read_document(source)
    if set(manifest) - {"plugin"}:
        raise ValueError(f"Invalid {source}: expected only [[plugin]] entries")
    entries = manifest.get("plugin", [])
    validate_plugins(source, entries)
    # Do not resolve symlinks or '..': preserve the original library path semantics.
    entries = [{"path": os.path.join(str(source.absolute().parent), e["path"])} for e in entries]
    text = original.decode("utf-8") if original is not None else 'repl_position = "bottom"\n'
    if entries:
        text += "\n" + "\n".join(
            f'[[plugin]]\npath = {json.dumps(e["path"])}\n' for e in entries
        )
    else:
        # The empty array must be at the root, before any existing TOML tables.
        text = "plugin = []\n" + text
    content = text.encode("utf-8")
    tomllib.loads(text)
    atomic_write(destination, content, original)
    saved, document = read_document(destination)
    if saved != content or document["plugin"] != entries:
        raise ValueError(f"Verification failed for {destination}; source retained: {source}")
    if source.read_bytes() != old_bytes:
        raise ValueError(f"Source changed during migration; source retained: {source}")
    try:
        source.unlink()
    except OSError as error:
        raise OSError(f"Saved {destination}, but could not remove {source}: {error}") from error
    return True


def main() -> int:
    config_dir = Path(os.environ.get("PATINAE_CONFIG_DIR") or Path.home() / ".patinae")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, help="path to the old plugins.toml")
    parser.add_argument(
        "--config", type=Path, default=config_dir / "config.toml",
        help="destination config.toml",
    )
    args = parser.parse_args()
    try:
        changed = migrate(args.source, args.config)
    except (OSError, ValueError) as error:
        print(f"Migration failed ({args.source} -> {args.config}): {error}", file=sys.stderr)
        return 1
    if changed:
        print(f"Migrated plugins to {args.config}; removed {args.source}")
    else:
        print(f"Unchanged: {args.config} already has a plugin list; source left untouched")
    return 0


if __name__ == "__main__":
    sys.exit(main())
