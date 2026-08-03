# PRS Upgrade

`prs-upgrade` converts legacy Patinae `.prs` sessions to the current PRS
format. Supported migrations cover raw positional sessions written by
PyMOL-RS v0.3.3, plus Patinae v0.4.0 through v0.4.2 sessions in either raw
positional or PRS v2 named-field form.

Run it from the repository root:

```bash
cargo run -p prs-upgrade -- old.prs upgraded.prs
```

The output path must not exist. The tool leaves the source untouched, writes a
current PRS v3 document, and loads the result again before reporting success.
Malformed current and future PRS envelopes are rejected instead of being
reinterpreted as legacy sessions. Legacy atom-local labels are migrated into
first-class label collections.
