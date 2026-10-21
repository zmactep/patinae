# Move an old plugin list to config.toml

Patinae no longer reads `plugins.toml`. To keep an old plugin list, close Patinae
and run the migration script from the repository root using [uv](https://docs.astral.sh/uv/):

```sh
uv run scripts/migrate_plugins_config.py ~/.patinae/plugins/plugins.toml
```

Pass your old file's path if it is elsewhere. The script writes to
`~/.patinae/config.toml`, or `$PATINAE_CONFIG_DIR/config.toml` when set.
To choose another destination:

```sh
uv run scripts/migrate_plugins_config.py /path/to/plugins.toml --config /path/to/config.toml
```

The script preserves existing settings and comments, keeps the plugin order,
and converts relative library paths to absolute paths. Libraries do not need
to exist during migration. It removes the old file only after saving and
verifying the new configuration.

If `config.toml` already contains a `plugin` list, the script leaves both files
unchanged. Errors leave the old file in place and print the path and reason.
Restart Patinae after migration.
