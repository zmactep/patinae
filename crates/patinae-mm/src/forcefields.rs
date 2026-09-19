use std::path::{Path, PathBuf};

pub const DEFAULT_FORCE_FIELD: &str = "AMBER";
pub const FORCE_FIELD_HINT: &str = "AMBER, CHARMM, OPLS-AA, or /path/to/custom.ff";

const ENV_FORCEFIELDS_DIR: &str = "PATINAE_DESIGN_TOOLBOX_FORCEFIELDS_DIR";
const SOURCE_FORCEFIELDS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/forcefields");

pub fn resolve_force_field_path(input: &str, plugin_dirs: &[PathBuf]) -> Result<PathBuf, String> {
    let trimmed = input.trim();
    let exact = Path::new(trimmed);
    if exact.exists() {
        return Ok(exact.to_path_buf());
    }
    if looks_like_path(trimmed) {
        return Ok(exact.to_path_buf());
    }

    let Some(alias_dir) = alias_dir(trimmed) else {
        return Ok(exact.to_path_buf());
    };

    let search_dirs = search_dirs(plugin_dirs);
    for dir in &search_dirs {
        let candidate = dir.join(alias_dir);
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(format!(
        "bundled force field '{trimmed}' ({alias_dir}) was not found; searched: {}",
        search_dirs
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn alias_dir(input: &str) -> Option<&'static str> {
    match normalized_alias(input).as_str() {
        "amber" | "amber19sb" => Some("amber19sb.ff"),
        "charmm" | "charmm27" => Some("charmm27.ff"),
        "opls" | "oplsaa" => Some("oplsaa.ff"),
        _ => None,
    }
}

fn normalized_alias(input: &str) -> String {
    input
        .trim()
        .trim_end_matches(".ff")
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn looks_like_path(input: &str) -> bool {
    input.contains('/') || input.contains('\\') || input == "." || input == ".."
}

fn search_dirs(plugin_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(value) = std::env::var_os(ENV_FORCEFIELDS_DIR) {
        for dir in std::env::split_paths(&value) {
            push_unique(&mut dirs, dir);
        }
    }
    for dir in plugin_dirs {
        push_unique(&mut dirs, dir.join("forcefields"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            push_unique(&mut dirs, parent.join("plugins").join("forcefields"));
            push_unique(&mut dirs, parent.join("forcefields"));
            if parent.file_name().and_then(|name| name.to_str()) == Some("MacOS") {
                if let Some(contents) = parent.parent() {
                    push_unique(&mut dirs, contents.join("PlugIns").join("forcefields"));
                }
            }
        }
    }
    push_unique(&mut dirs, PathBuf::from(SOURCE_FORCEFIELDS_DIR));
    push_unique(&mut dirs, PathBuf::from("crates/patinae-mm/forcefields"));
    dirs
}

fn push_unique(dirs: &mut Vec<PathBuf>, dir: PathBuf) {
    if !dirs.iter().any(|existing| existing == &dir) {
        dirs.push(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_alias_from_plugin_dir() {
        let dir = tempfile::tempdir().unwrap();
        let bundled = dir.path().join("forcefields").join("oplsaa.ff");
        std::fs::create_dir_all(&bundled).unwrap();

        let resolved = resolve_force_field_path("OPLS-AA", &[dir.path().to_path_buf()]).unwrap();
        assert_eq!(resolved, bundled);
    }
}
