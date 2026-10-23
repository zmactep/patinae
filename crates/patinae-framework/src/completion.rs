//! Command-line tab completion engine and UI state
//!
//! Contains:
//! - [`CompletionResult`] / [`CompletionContext`] / [`generate_completions`] — the completion engine
//! - [`CompletionState`] — popup state machine (visible, selected, suggestions)
//! - [`CommandLineState`] — focus and completion state for the command input
//!
//! No egui dependency — pure business logic, testable and headless-compatible.
//!
//! Provides tab completion for:
//! - Command names (when typing the first word)
//! - File/directory paths (for I/O commands like load, save, png, cd, ls)
//! - Setting names (for set, get, unset)
//! - Color names (for color, bg_color)
//! - Representation names (for show, hide, as)
//! - Object/selection names (for zoom, center, delete, etc.)

use std::path::Path;

use patinae_cmd::{
    ArgHint, CommandRegistry, CommandSource, DynamicSettingRegistry, ResolvedSetting,
};

// =============================================================================
// Rich completion item
// =============================================================================

/// A single completion suggestion with metadata.
#[derive(Debug, Clone)]
pub struct CompletionItem {
    /// The completion text to insert.
    pub text: String,
    /// Human-readable description (e.g., command summary, setting type).
    pub description: String,
    /// Where this item comes from (built-in, plugin, or unspecified).
    pub source: CompletionSource,
}

/// Origin of a completion item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompletionSource {
    /// Non-command completions (settings, colors, objects, etc.)
    #[default]
    None,
    /// Built-in command
    BuiltIn,
    /// Plugin-provided command
    Plugin,
}

impl From<CommandSource> for CompletionSource {
    fn from(s: CommandSource) -> Self {
        match s {
            CommandSource::BuiltIn => CompletionSource::BuiltIn,
            CommandSource::Plugin => CompletionSource::Plugin,
        }
    }
}

impl CompletionItem {
    /// Create a plain item (no description, no source).
    fn plain(text: String) -> Self {
        Self {
            text,
            description: String::new(),
            source: CompletionSource::None,
        }
    }
}

/// Result of completion generation
#[derive(Debug)]
pub struct CompletionResult {
    /// Byte position in input where the completion starts
    pub start_pos: usize,
    /// List of completion suggestions
    pub suggestions: Vec<CompletionItem>,
}

impl CompletionResult {
    /// Create an empty result (no completions)
    pub fn empty(pos: usize) -> Self {
        Self {
            start_pos: pos,
            suggestions: Vec::new(),
        }
    }
}

/// Static list of representation names for completion
const REPRESENTATION_NAMES: &[&str] = &[
    "cartoon",
    "sticks",
    "lines",
    "spheres",
    "surface",
    "mesh",
    "ribbon",
    "dots",
    "nonbonded",
    "nb_spheres",
    "labels",
    "cell",
    "cgo",
    "everything",
];

/// Static list of label property names for completion
const LABEL_PROPERTY_NAMES: &[&str] = &[
    "name",
    "resn",
    "resi",
    "chain",
    "q",
    "b",
    "segi",
    "type",
    "formal_charge",
    "partial_charge",
];

/// Context for generating completions, holding all available name lists
pub struct CompletionContext<'a> {
    pub command_names: &'a [&'a str],
    pub registry: &'a CommandRegistry,
    pub setting_names: &'a [&'a str],
    pub color_names: &'a [String],
    pub object_names: &'a [String],
    pub selection_names: &'a [String],
    pub dynamic_settings: Option<&'a DynamicSettingRegistry>,
}

/// Generate completions for the current command input
pub fn generate_completions(
    input: &str,
    cursor_pos: usize,
    ctx: &CompletionContext,
) -> CompletionResult {
    let cursor_pos = cursor_pos.min(input.len());
    let text_before_cursor = &input[..cursor_pos];

    // Parse into command + segments (comma-separated, bracket-aware)
    let parsed = parse_input(text_before_cursor);

    match parsed {
        ParsedInput::Empty => {
            // Show all commands
            complete_commands("", cursor_pos, ctx)
        }
        ParsedInput::CommandPrefix(prefix) => {
            // Completing command name
            complete_commands(prefix, cursor_pos - prefix.len(), ctx)
        }
        ParsedInput::Argument {
            command,
            arg_index,
            prefix,
            prefix_start,
        } => {
            // Look up arg hints for this command
            if let Some(cmd) = ctx.registry.get(&command.to_lowercase()) {
                let hints: &[ArgHint] = cmd.arg_hints();

                if let Some(hint) = hints.get(arg_index) {
                    match hint {
                        ArgHint::Path => complete_path(text_before_cursor, cursor_pos),
                        ArgHint::Setting => complete_settings(prefix, prefix_start, ctx),
                        ArgHint::SettingValue => {
                            complete_setting_value(text_before_cursor, prefix, prefix_start, ctx)
                        }
                        ArgHint::Color => {
                            let mut suggestions =
                                filter_prefix(prefix, ctx.color_names.iter().map(|s| s.as_str()));
                            suggestions.extend(filter_prefix(
                                prefix,
                                patinae_color::SCHEME_NAMES.iter().copied(),
                            ));
                            suggestions.sort_by_key(|a| a.text.to_lowercase());
                            suggestions.dedup_by(|a, b| a.text == b.text);
                            CompletionResult {
                                start_pos: prefix_start,
                                suggestions,
                            }
                        }
                        ArgHint::Representation => {
                            complete_from_static(prefix, prefix_start, REPRESENTATION_NAMES)
                        }
                        ArgHint::Object => complete_from_list(
                            prefix,
                            prefix_start,
                            &ctx.object_names
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>(),
                        ),
                        ArgHint::Selection => {
                            // Object names + selection names
                            let mut suggestions =
                                filter_prefix(prefix, ctx.object_names.iter().map(|s| s.as_str()));
                            suggestions.extend(filter_prefix(
                                prefix,
                                ctx.selection_names.iter().map(|s| s.as_str()),
                            ));
                            suggestions.sort_by_key(|a| a.text.to_lowercase());
                            suggestions.dedup_by(|a, b| a.text == b.text);
                            CompletionResult {
                                start_pos: prefix_start,
                                suggestions,
                            }
                        }
                        ArgHint::NamedSelection => complete_from_list(
                            prefix,
                            prefix_start,
                            &ctx.selection_names
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>(),
                        ),
                        ArgHint::LabelProperty => {
                            complete_from_static(prefix, prefix_start, LABEL_PROPERTY_NAMES)
                        }
                        ArgHint::Command => complete_commands(prefix, prefix_start, ctx),
                        ArgHint::Keywords(words) => {
                            complete_from_static(prefix, prefix_start, words)
                        }
                        ArgHint::None => CompletionResult::empty(cursor_pos),
                    }
                } else {
                    // No hint for this position - no completion
                    CompletionResult::empty(cursor_pos)
                }
            } else {
                // Unknown command
                CompletionResult::empty(cursor_pos)
            }
        }
    }
}

/// Parsed representation of the input before cursor
enum ParsedInput<'a> {
    /// Empty input
    Empty,
    /// Typing the first word (command name)
    CommandPrefix(&'a str),
    /// Typing an argument
    Argument {
        command: &'a str,
        arg_index: usize,
        prefix: &'a str,
        prefix_start: usize,
    },
}

/// Parse input into command and current argument position
fn parse_input(text: &str) -> ParsedInput<'_> {
    if text.is_empty() {
        return ParsedInput::Empty;
    }

    // Split into segments by commas (bracket-aware)
    let segments = split_by_commas(text);

    if segments.is_empty() {
        return ParsedInput::Empty;
    }

    // First segment: "command arg0_prefix" or just "command_prefix"
    let first_seg = segments[0];
    let first_words: Vec<&str> = first_seg.split_whitespace().collect();

    if first_words.is_empty() {
        return ParsedInput::Empty;
    }

    let ends_with_space = first_seg.ends_with(char::is_whitespace);

    if segments.len() == 1 && first_words.len() == 1 && !ends_with_space {
        // Still typing the command name
        return ParsedInput::CommandPrefix(first_words[0]);
    }

    let command = first_words[0];

    if segments.len() == 1 {
        // Still in first segment, after command name
        // arg_index = 0, prefix is everything after command
        let cmd_end = first_seg
            .find(char::is_whitespace)
            .unwrap_or(first_seg.len());
        let after_cmd = &first_seg[cmd_end..];
        let prefix = after_cmd.trim_start();
        let prefix_start = text.len() - prefix.len();
        return ParsedInput::Argument {
            command,
            arg_index: 0,
            prefix,
            prefix_start,
        };
    }

    // We're in segment N (N >= 1), which means arg_index = N - 1... wait:
    // segment 0 = "command arg0", segment 1 = " arg1", segment 2 = " arg2"
    // So arg_index = segments.len() - 1
    let arg_index = segments.len() - 1;
    let last_seg = segments[arg_index];
    let prefix = last_seg.trim_start();
    let prefix_start = text.len() - prefix.len();

    ParsedInput::Argument {
        command,
        arg_index,
        prefix,
        prefix_start,
    }
}

/// Split text by commas, respecting brackets and quotes
fn split_by_commas(text: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut depth = 0i32;
    let mut in_quote = false;
    let mut last_start = 0;

    for (i, ch) in text.char_indices() {
        match ch {
            '"' | '\'' => in_quote = !in_quote,
            '(' | '[' | '{' if !in_quote => depth += 1,
            ')' | ']' | '}' if !in_quote => depth -= 1,
            ',' if !in_quote && depth <= 0 => {
                segments.push(&text[last_start..i]);
                last_start = i + 1;
            }
            _ => {}
        }
    }
    segments.push(&text[last_start..]);
    segments
}

// =============================================================================
// Command completion (with descriptions and source badges)
// =============================================================================

/// Complete command names with description and built-in/plugin source.
fn complete_commands(
    prefix: &str,
    prefix_start: usize,
    ctx: &CompletionContext,
) -> CompletionResult {
    let prefix_lower = prefix.to_lowercase();
    let mut suggestions: Vec<CompletionItem> = Vec::new();

    for &name in ctx.command_names {
        if name.to_lowercase().starts_with(&prefix_lower) {
            let (description, source) = if let Some(cmd) = ctx.registry.get(name) {
                let desc = cmd.description().to_string();
                let src: CompletionSource = ctx.registry.source(name).into();
                (desc, src)
            } else {
                (String::new(), CompletionSource::None)
            };
            suggestions.push(CompletionItem {
                text: name.to_string(),
                description,
                source,
            });
        }
    }

    suggestions.sort_by_key(|a| a.text.to_lowercase());
    CompletionResult {
        start_pos: prefix_start,
        suggestions,
    }
}

// =============================================================================
// Setting name completion (with type labels)
// =============================================================================

/// Complete setting names with type labels as descriptions.
fn complete_settings(
    prefix: &str,
    prefix_start: usize,
    ctx: &CompletionContext,
) -> CompletionResult {
    let prefix_lower = prefix.to_lowercase();
    let mut suggestions: Vec<CompletionItem> = Vec::new();

    for &name in ctx.setting_names {
        if name.to_lowercase().starts_with(&prefix_lower) {
            let description = setting_type_label(name, ctx);
            suggestions.push(CompletionItem {
                text: name.to_string(),
                description,
                source: CompletionSource::None,
            });
        }
    }

    if let Some(dyn_reg) = ctx.dynamic_settings {
        for name in dyn_reg.names() {
            if name.to_lowercase().starts_with(&prefix_lower) {
                let description = setting_type_label(name, ctx);
                suggestions.push(CompletionItem {
                    text: name.clone(),
                    description,
                    source: CompletionSource::Plugin,
                });
            }
        }
    }

    suggestions.sort_by_key(|a| a.text.to_lowercase());
    CompletionResult {
        start_pos: prefix_start,
        suggestions,
    }
}

/// Look up a human-readable type label for a setting name.
///
/// Consults only the typed registry and dynamic (plugin) registry — the
/// legacy `definitions.rs` table is `.pse`-import compat and is not a source
/// of runtime truth.
fn setting_type_label(name: &str, ctx: &CompletionContext) -> String {
    ResolvedSetting::lookup(name, ctx.dynamic_settings)
        .map(|setting| setting.setting_type().to_string())
        .unwrap_or_default()
}

// =============================================================================
// Setting value completion
// =============================================================================

/// Complete setting value (for `ArgHint::SettingValue`)
fn complete_setting_value(
    text: &str,
    prefix: &str,
    prefix_start: usize,
    ctx: &CompletionContext,
) -> CompletionResult {
    // Extract setting name from arg 0
    let segments = split_by_commas(text);
    if segments.is_empty() {
        return CompletionResult::empty(prefix_start);
    }

    let first_seg = segments[0];
    let first_words: Vec<&str> = first_seg.split_whitespace().collect();
    let setting_name = if first_words.len() >= 2 {
        first_words[1]
    } else {
        return CompletionResult::empty(prefix_start);
    };

    if let Some(setting) = ResolvedSetting::lookup(setting_name, ctx.dynamic_settings) {
        return value_completion_for(
            setting.has_value_hints(),
            setting.hint_names(),
            setting.setting_type(),
            prefix,
            prefix_start,
            ctx,
        );
    }

    CompletionResult::empty(prefix_start)
}

fn value_completion_for(
    has_hints: bool,
    hints: Vec<&str>,
    ty: patinae_settings::SettingType,
    prefix: &str,
    prefix_start: usize,
    ctx: &CompletionContext,
) -> CompletionResult {
    if has_hints {
        return complete_from_static(prefix, prefix_start, &hints);
    }
    match ty {
        patinae_settings::SettingType::Bool => {
            complete_from_static(prefix, prefix_start, &["on", "off"])
        }
        patinae_settings::SettingType::Color => {
            let mut suggestions = filter_prefix(prefix, ctx.color_names.iter().map(|s| s.as_str()));
            suggestions.extend(filter_prefix(
                prefix,
                patinae_color::SCHEME_NAMES.iter().copied(),
            ));
            suggestions.sort_by_key(|a| a.text.to_lowercase());
            suggestions.dedup_by(|a, b| a.text == b.text);
            CompletionResult {
                start_pos: prefix_start,
                suggestions,
            }
        }
        _ => CompletionResult::empty(prefix_start),
    }
}

// =============================================================================
// Generic helpers
// =============================================================================

/// Filter items by case-insensitive prefix, producing plain CompletionItems.
fn filter_prefix<'a>(prefix: &str, items: impl Iterator<Item = &'a str>) -> Vec<CompletionItem> {
    let prefix_lower = prefix.to_lowercase();
    items
        .filter(|item| item.to_lowercase().starts_with(&prefix_lower))
        .map(|s| CompletionItem::plain(s.to_string()))
        .collect()
}

/// Complete from a dynamic list of names
fn complete_from_list(prefix: &str, prefix_start: usize, names: &[&str]) -> CompletionResult {
    let prefix_lower = prefix.to_lowercase();
    let mut matches: Vec<CompletionItem> = names
        .iter()
        .filter(|name| name.to_lowercase().starts_with(&prefix_lower))
        .map(|s| CompletionItem::plain(s.to_string()))
        .collect();
    matches.sort_by_key(|a| a.text.to_lowercase());
    CompletionResult {
        start_pos: prefix_start,
        suggestions: matches,
    }
}

/// Complete from a static list
fn complete_from_static(prefix: &str, start_pos: usize, names: &[&str]) -> CompletionResult {
    let prefix_lower = prefix.to_lowercase();
    let mut matches: Vec<CompletionItem> = names
        .iter()
        .filter(|name| name.to_lowercase().starts_with(&prefix_lower))
        .map(|s| CompletionItem::plain(s.to_string()))
        .collect();
    matches.sort_by_key(|a| a.text.to_lowercase());
    CompletionResult {
        start_pos,
        suggestions: matches,
    }
}

/// Complete file/directory paths
fn complete_path(input: &str, _cursor_pos: usize) -> CompletionResult {
    // Find where the path argument starts
    let path_start = find_path_start(input);
    let path_portion = &input[path_start..];

    // Expand tilde if present
    let expanded = expand_tilde(path_portion);
    let expanded_path = Path::new(&expanded);

    // Determine the directory to search and the prefix to match
    let (search_dir, prefix) = if expanded_path.is_dir() && path_portion.ends_with('/') {
        (expanded_path.to_path_buf(), String::new())
    } else if let Some(parent) = expanded_path.parent() {
        let prefix = expanded_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        // Empty parent means bare filename (e.g. "1" without "./" prefix)
        // Use cwd so we get relative paths
        let dir = if parent.as_os_str().is_empty() {
            Path::new(".").to_path_buf()
        } else {
            parent.to_path_buf()
        };
        (dir, prefix)
    } else {
        (Path::new(".").to_path_buf(), expanded.clone())
    };

    let mut suggestions = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&search_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();

            if name.starts_with('.') && !prefix.starts_with('.') {
                continue;
            }

            if name.to_lowercase().starts_with(&prefix.to_lowercase()) {
                let mut suggestion = if path_portion.starts_with('~') {
                    #[cfg(not(target_arch = "wasm32"))]
                    let home = dirs::home_dir().unwrap_or_default();
                    #[cfg(target_arch = "wasm32")]
                    let home = std::path::PathBuf::new();
                    format!(
                        "~/{}",
                        search_dir
                            .strip_prefix(&home)
                            .unwrap_or(&search_dir)
                            .join(&name)
                            .display()
                    )
                } else if search_dir.as_os_str().is_empty() || search_dir == Path::new(".") {
                    name.clone()
                } else {
                    search_dir.join(&name).display().to_string()
                };

                if entry.path().is_dir() {
                    suggestion.push('/');
                }

                suggestions.push(CompletionItem::plain(suggestion));
            }
        }
    }

    suggestions.sort_by(|a, b| {
        let a_is_dir = a.text.ends_with('/');
        let b_is_dir = b.text.ends_with('/');
        match (a_is_dir, b_is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.text.to_lowercase().cmp(&b.text.to_lowercase()),
        }
    });

    CompletionResult {
        start_pos: path_start,
        suggestions,
    }
}

/// Find the byte position where the path argument starts in the input
fn find_path_start(input: &str) -> usize {
    let mut chars = input.char_indices().peekable();

    // Skip command (first word)
    while let Some((_, c)) = chars.peek() {
        if c.is_whitespace() {
            break;
        }
        chars.next();
    }

    // Skip whitespace after command
    while let Some((_, c)) = chars.peek() {
        if !c.is_whitespace() {
            break;
        }
        chars.next();
    }

    chars.peek().map(|(i, _)| *i).unwrap_or(input.len())
}

/// Expand tilde (~) to home directory
fn expand_tilde(path: &str) -> String {
    if path.starts_with('~') {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(home) = dirs::home_dir() {
            return path.replacen('~', &home.display().to_string(), 1);
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_by_commas() {
        assert_eq!(
            split_by_commas("set sphere_scale, 0.5"),
            vec!["set sphere_scale", " 0.5"]
        );
        assert_eq!(
            split_by_commas("color green, chain A"),
            vec!["color green", " chain A"]
        );
        assert_eq!(split_by_commas("show cartoon"), vec!["show cartoon"]);
        // Brackets
        assert_eq!(
            split_by_commas("set stick_color, [1.0, 0.5, 0.0]"),
            vec!["set stick_color", " [1.0, 0.5, 0.0]"]
        );
    }

    #[test]
    fn test_parse_input_empty() {
        assert!(matches!(parse_input(""), ParsedInput::Empty));
    }

    #[test]
    fn test_parse_input_command_prefix() {
        match parse_input("lo") {
            ParsedInput::CommandPrefix("lo") => {}
            _ => panic!("Expected CommandPrefix"),
        }
    }

    #[test]
    fn test_parse_input_argument() {
        match parse_input("set sphere_sc") {
            ParsedInput::Argument {
                command: "set",
                arg_index: 0,
                prefix: "sphere_sc",
                ..
            } => {}
            _ => panic!("Expected Argument for arg 0"),
        }

        match parse_input("set sphere_scale, ") {
            ParsedInput::Argument {
                command: "set",
                arg_index: 1,
                prefix: "",
                ..
            } => {}
            _ => panic!("Expected Argument for arg 1"),
        }

        match parse_input("color green, ") {
            ParsedInput::Argument {
                command: "color",
                arg_index: 1,
                prefix: "",
                ..
            } => {}
            _ => panic!("Expected Argument for arg 1"),
        }
    }

    #[test]
    fn test_find_path_start() {
        assert_eq!(find_path_start("load /path/to/file"), 5);
        assert_eq!(find_path_start("load  /path"), 6);
        assert_eq!(find_path_start("cd ~/"), 3);
        assert_eq!(find_path_start("ls"), 2);
    }

    #[test]
    fn test_expand_tilde() {
        let expanded = expand_tilde("~/test");
        assert!(!expanded.starts_with('~'));
        assert!(expanded.ends_with("/test"));
    }

    #[test]
    fn test_runtime_registry_settings_complete() {
        let registry = CommandRegistry::with_builtins();
        let setting_names = patinae_settings::setting_names();
        let setting_name_refs: Vec<&str> = setting_names.to_vec();
        let color_names = Vec::new();
        let object_names = Vec::new();
        let selection_names = Vec::new();
        let ctx = CompletionContext {
            command_names: &[],
            registry: &registry,
            setting_names: &setting_name_refs,
            color_names: &color_names,
            object_names: &object_names,
            selection_names: &selection_names,
            dynamic_settings: None,
        };
        let input = "set theme, sys";
        let result = generate_completions(input, input.len(), &ctx);
        assert_eq!(result.suggestions.len(), 1);
        assert_eq!(result.suggestions[0].text, "system");

        for (input, expected) in [
            ("set cartoon_smooth_l", "cartoon_smooth_loops"),
            ("set fxaa_", "fxaa_enabled"),
            ("set ssao_", "ssao_enabled"),
            ("set ssao_", "ssao_radius"),
            ("set ssao_", "ssao_intensity"),
            ("set ssao_", "ssao_bias"),
            ("set surface_sol", "surface_solvent"),
            ("set mesh_", "mesh_quality"),
            ("set mesh_", "mesh_solvent"),
            ("set mesh_", "mesh_solvent_radius"),
            ("set mesh_", "mesh_transparency"),
        ] {
            let result = generate_completions(input, input.len(), &ctx);
            assert!(
                result.suggestions.iter().any(|item| item.text == expected),
                "{expected} missing from completion for {input}"
            );
        }

        let result = generate_completions("set shading_mode, ", "set shading_mode, ".len(), &ctx);
        assert!(result.suggestions.iter().any(|item| item.text == "classic"));
        assert!(result
            .suggestions
            .iter()
            .any(|item| item.text == "skripkin"));
        assert!(result.suggestions.iter().any(|item| item.text == "full"));
    }

    #[test]
    fn test_dynamic_settings_complete_as_plugin_items() {
        use std::sync::{Arc, RwLock};

        use patinae_cmd::DynamicSettingRegistry;
        use patinae_settings::{
            DynamicSettingDescriptor, DynamicSettingStore, SettingType, SettingValue,
        };

        let registry = CommandRegistry::with_builtins();
        let setting_names = patinae_settings::setting_names();
        let setting_name_refs: Vec<&str> = setting_names.to_vec();
        let color_names = Vec::new();
        let object_names = Vec::new();
        let selection_names = Vec::new();
        let mut dynamic_settings = DynamicSettingRegistry::new();
        let store = Arc::new(RwLock::new(DynamicSettingStore::new()));

        dynamic_settings
            .register(
                DynamicSettingDescriptor {
                    name: "ray_max_passes".to_string(),
                    setting_type: SettingType::Int,
                    default: SettingValue::Int(25),
                    min: Some(1.0),
                    max: Some(100.0),
                    value_hints: Vec::new(),
                    side_effects: Vec::new(),
                    object_overridable: false,
                },
                store.clone(),
            )
            .unwrap();
        dynamic_settings
            .register(
                DynamicSettingDescriptor {
                    name: "rt_use_custom".to_string(),
                    setting_type: SettingType::Bool,
                    default: SettingValue::Bool(false),
                    min: None,
                    max: None,
                    value_hints: Vec::new(),
                    side_effects: Vec::new(),
                    object_overridable: false,
                },
                store,
            )
            .unwrap();

        let ctx = CompletionContext {
            command_names: &[],
            registry: &registry,
            setting_names: &setting_name_refs,
            color_names: &color_names,
            object_names: &object_names,
            selection_names: &selection_names,
            dynamic_settings: Some(&dynamic_settings),
        };

        let ray = generate_completions("set ray_", "set ray_".len(), &ctx);
        let ray_item = ray
            .suggestions
            .iter()
            .find(|item| item.text == "ray_max_passes")
            .expect("ray_max_passes missing from dynamic setting completion");
        assert_eq!(ray_item.source, CompletionSource::Plugin);
        assert_eq!(ray_item.description, "int");

        let rt = generate_completions("get rt_", "get rt_".len(), &ctx);
        let rt_item = rt
            .suggestions
            .iter()
            .find(|item| item.text == "rt_use_custom")
            .expect("rt_use_custom missing from dynamic setting completion");
        assert_eq!(rt_item.source, CompletionSource::Plugin);
        assert_eq!(rt_item.description, "bool");
    }
}

// =============================================================================
// Completion popup state machine
// =============================================================================

/// Autocomplete/completion state for command line.
///
/// Tracks the popup visibility, suggestion list, and selected index.
/// Pure state machine — no UI framework dependency.
#[derive(Debug, Clone, Default)]
pub struct CompletionState {
    /// List of current completion suggestions
    pub suggestions: Vec<CompletionItem>,
    /// Currently selected suggestion index
    pub selected: usize,
    /// Whether the completion popup is visible
    pub visible: bool,
    /// Position in input where completion starts (byte offset)
    pub start_pos: usize,
    /// Whether to scroll to the selected item
    pub scroll_to_selected: bool,
}

impl CompletionState {
    /// Create a new empty completion state
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset the completion state (hide popup and clear suggestions)
    pub fn reset(&mut self) {
        self.suggestions.clear();
        self.selected = 0;
        self.visible = false;
        self.start_pos = 0;
    }

    /// Set new suggestions and show the popup
    pub fn show(&mut self, start_pos: usize, suggestions: Vec<CompletionItem>) {
        if suggestions.is_empty() {
            self.reset();
        } else {
            self.start_pos = start_pos;
            self.suggestions = suggestions;
            self.selected = 0;
            self.visible = true;
        }
    }

    /// Move selection to next item (wrapping)
    pub fn select_next(&mut self) {
        if !self.suggestions.is_empty() {
            self.selected = (self.selected + 1) % self.suggestions.len();
            self.scroll_to_selected = true;
        }
    }

    /// Move selection to previous item (wrapping)
    pub fn select_previous(&mut self) {
        if !self.suggestions.is_empty() {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.suggestions.len() - 1);
            self.scroll_to_selected = true;
        }
    }

    /// Get the currently selected suggestion
    pub fn selected_suggestion(&self) -> Option<&str> {
        self.suggestions.get(self.selected).map(|s| s.text.as_str())
    }

    /// Apply the selected suggestion to the input string
    /// Returns true if a completion was applied
    pub fn apply_to_input(&mut self, input: &mut String) -> bool {
        if let Some(suggestion) = self.suggestions.get(self.selected).cloned() {
            input.truncate(self.start_pos);
            input.push_str(&suggestion.text);
            // Add space after command name completion only (not for arguments or paths)
            if self.start_pos == 0
                && !suggestion.text.ends_with('/')
                && !suggestion.text.contains('/')
            {
                input.push(' ');
            }
            self.reset();
            true
        } else {
            false
        }
    }
}

// =============================================================================
// Command line state
// =============================================================================

/// Command line UI state: focus tracking and completion popup.
///
/// This is the interaction state for the command input, independent of
/// the underlying UI framework.
pub struct CommandLineState {
    /// Whether the command input should request focus (one-shot flag)
    pub wants_focus: bool,
    /// Whether the command input currently has focus
    pub has_focus: bool,
    /// Current autocomplete/completion state
    pub completion: CompletionState,
}

impl Default for CommandLineState {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandLineState {
    pub fn new() -> Self {
        Self {
            wants_focus: true, // Focus on startup
            has_focus: false,
            completion: CompletionState::new(),
        }
    }
}
