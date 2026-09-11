//! Movie commands: mplay, mstop, mpause, mset, mview, mpng, mproduce, madd, frame, forward, backward, rewind, ending, rock

use crate::args::{ArgValue, ParsedCommand};
use crate::command::{ArgHint, Command, CommandContext, CommandRegistry, ViewerLike};
use crate::command_help;
use crate::error::{CmdError, CmdResult};

/// Register movie commands
pub fn register(registry: &mut CommandRegistry) {
    registry.register(MplayCommand);
    registry.register(MstopCommand);
    registry.register(MpauseCommand);
    registry.register(MtoggleCommand);
    registry.register(MsetCommand);
    registry.register(MviewCommand);
    registry.register(MpngCommand);
    registry.register(ProduceCommand);
    registry.register(MaddCommand);
    registry.register(FrameCommand);
    registry.register(ForwardCommand);
    registry.register(BackwardCommand);
    registry.register(RewindCommand);
    registry.register(MiddleCommand);
    registry.register(EndingCommand);
    registry.register(RockCommand);
}

// ============================================================================
// mplay command
// ============================================================================

struct MplayCommand;

impl Command for MplayCommand {
    fn name(&self) -> &str {
        "mplay"
    }

    command_help! {
        CMD "mplay"
        DESCRIPTION [
            "starts movie playback.",
        ]
        REQUIRED []
        OPTIONAL []
        EXAMPLES [
            "mplay",
        ]
        SEE ALSO [
            "mstop, mpause, mset, frame",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        _args: &ParsedCommand,
    ) -> CmdResult {
        ctx.viewer.movie_play();
        if !ctx.quiet {
            ctx.print(" Movie playing.");
        }
        Ok(())
    }
}

// ============================================================================
// mstop command
// ============================================================================

struct MstopCommand;

impl Command for MstopCommand {
    fn name(&self) -> &str {
        "mstop"
    }

    command_help! {
        CMD "mstop"
        DESCRIPTION [
            "stops movie playback and resets to frame 1.",
        ]
        REQUIRED []
        OPTIONAL []
        EXAMPLES [
            "mstop",
        ]
        SEE ALSO [
            "mplay, mpause, frame",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        _args: &ParsedCommand,
    ) -> CmdResult {
        ctx.viewer.movie_stop();
        if !ctx.quiet {
            ctx.print(" Movie stopped.");
        }
        Ok(())
    }
}

// ============================================================================
// mpause command
// ============================================================================

struct MpauseCommand;

impl Command for MpauseCommand {
    fn name(&self) -> &str {
        "mpause"
    }

    command_help! {
        CMD "mpause"
        DESCRIPTION [
            "pauses movie playback at the current frame.",
        ]
        REQUIRED []
        OPTIONAL []
        EXAMPLES [
            "mpause",
        ]
        SEE ALSO [
            "mplay, mstop, frame",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        _args: &ParsedCommand,
    ) -> CmdResult {
        ctx.viewer.movie_pause();
        if !ctx.quiet {
            ctx.print(" Movie paused.");
        }
        Ok(())
    }
}

// ============================================================================
// mtoggle command
// ============================================================================

struct MtoggleCommand;

impl Command for MtoggleCommand {
    fn name(&self) -> &str {
        "mtoggle"
    }

    command_help! {
        CMD "mtoggle"
        DESCRIPTION [
            "toggles movie playback between play and pause.",
        ]
        REQUIRED []
        OPTIONAL []
        EXAMPLES [
            "mtoggle",
        ]
        SEE ALSO [
            "mplay, mpause, mstop",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        _args: &ParsedCommand,
    ) -> CmdResult {
        ctx.viewer.movie_toggle();
        let state = if ctx.viewer.movie_is_playing() {
            "playing"
        } else {
            "paused"
        };
        if !ctx.quiet {
            ctx.print(&format!(" Movie {}.", state));
        }
        Ok(())
    }
}

// ============================================================================
// mset command
// ============================================================================

struct MsetCommand;

/// Parse an mset specification string into a Vec of state indices.
///
/// Syntax:
///   `1 x60`         → 60 frames, all state 1
///   `1 -30`         → 30 frames for states 1..30
///   `1 x30 1 -60`   → 30 frames of state 1, then 30 frames ramping 1..60
///   `1 x10 2 x10`   → 10 frames of state 1, then 10 of state 2
///
/// A simple integer N is shorthand for `1 xN`.
fn parse_mset_spec(args: &ParsedCommand) -> Result<Vec<usize>, CmdError> {
    // Reconstruct raw spec string from parsed args
    let mut spec = String::new();
    for (_, val) in &args.args {
        if !spec.is_empty() {
            spec.push(' ');
        }
        match val {
            ArgValue::Int(i) => spec.push_str(&i.to_string()),
            ArgValue::Float(f) => spec.push_str(&f.to_string()),
            ArgValue::String(s) => spec.push_str(s),
            _ => {}
        }
    }
    let spec = spec.trim();

    if spec.is_empty() {
        return Err(CmdError::missing_argument("specification".to_string()));
    }

    // Tokenize: split by whitespace
    let tokens: Vec<&str> = spec.split_whitespace().collect();

    // Simple case: single integer with no operators
    if tokens.len() == 1 {
        if let Ok(n) = tokens[0].parse::<usize>() {
            // Just a number: create n frames of state 1
            return Ok(vec![1; n]);
        }
    }

    let mut states: Vec<usize> = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        let token = tokens[i];

        // Parse a state number
        let state: usize = token.parse().map_err(|_| {
            CmdError::invalid_arg(
                "specification",
                format!("expected state number, got '{}'", token),
            )
        })?;

        i += 1;

        if i >= tokens.len() {
            // Trailing state number with no operator: single frame
            states.push(state);
            break;
        }

        let next = tokens[i];

        if let Some(count_str) = next.strip_prefix('x').or_else(|| next.strip_prefix('X')) {
            // Repeat: state xN
            let count: usize = count_str.parse().map_err(|_| {
                CmdError::invalid_arg(
                    "specification",
                    format!("expected count after 'x', got '{}'", next),
                )
            })?;
            for _ in 0..count {
                states.push(state);
            }
            i += 1;
        } else if next.starts_with('-') || next.starts_with("−") {
            // Range: state_start -state_end (or negative number)
            let end_str = next.trim_start_matches('-').trim_start_matches('−');
            let end_state: usize = end_str.parse().map_err(|_| {
                CmdError::invalid_arg(
                    "specification",
                    format!("expected end state after '-', got '{}'", next),
                )
            })?;
            if end_state >= state {
                for s in state..=end_state {
                    states.push(s);
                }
            } else {
                for s in (end_state..=state).rev() {
                    states.push(s);
                }
            }
            i += 1;
        } else {
            // No operator following this state: single frame of this state,
            // and the next token starts a new clause
            states.push(state);
        }
    }

    if states.is_empty() {
        return Err(CmdError::invalid_arg(
            "specification",
            "empty specification",
        ));
    }

    Ok(states)
}

impl Command for MsetCommand {
    fn name(&self) -> &str {
        "mset"
    }

    command_help! {
        CMD "mset"
        DESCRIPTION [
            "sets the number of frames in the movie.",
        ]
        REQUIRED [
            { "specification", "string", "frame specification string" } => [
                "\"1 x30\" creates 30 copies of state 1",
                "\"1 -30\" creates frames for states 1 through 30",
                "\"1 x30 1 -60\" 30 frames of state 1, then ramp 1..60",
                "Simple number N is shorthand for \"1 xN\"",
            ],
        ]
        OPTIONAL []
        EXAMPLES [
            "mset 100            # 100 frames of state 1",
            "mset 1 x60          # 60 frames of state 1",
            "mset 1 -60          # Frames for states 1-60",
            "mset 1 x30 1 -60    # 30 of state 1, then ramp states 1-60",
        ]
        SEE ALSO [
            "mplay, madd, frame, mview",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let states = parse_mset_spec(args)?;
        let count = states.len();
        ctx.viewer.movie_set_from_spec(states);

        if !ctx.quiet {
            ctx.print(&format!(" Movie: {} frames.", count));
        }
        Ok(())
    }
}

// ============================================================================
// frame command
// ============================================================================

struct FrameCommand;

impl Command for FrameCommand {
    fn name(&self) -> &str {
        "frame"
    }

    command_help! {
        CMD "frame"
        DESCRIPTION [
            "goes to a specific frame in the movie.",
        ]
        REQUIRED [
            { "frame_number", "int", "frame number (1-indexed)" },
        ]
        OPTIONAL []
        EXAMPLES [
            "frame 1             # Go to first frame",
            "frame 50            # Go to frame 50",
        ]
        SEE ALSO [
            "mplay, mset, forward, backward",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let frame = args.int_arg_or(0, "frame", 1) as usize;

        // User-facing frame numbers are 1-based.
        ctx.viewer.movie_goto(frame.saturating_sub(1));

        if !ctx.quiet {
            ctx.print(&format!(" Frame {}.", frame));
        }
        Ok(())
    }
}

// ============================================================================
// forward command
// ============================================================================

struct ForwardCommand;

impl Command for ForwardCommand {
    fn name(&self) -> &str {
        "forward"
    }

    command_help! {
        CMD "forward"
        DESCRIPTION [
            "advances to the next frame.",
        ]
        REQUIRED []
        OPTIONAL []
        SEE ALSO [
            "backward, frame, mplay",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        _args: &ParsedCommand,
    ) -> CmdResult {
        ctx.viewer.movie_next();
        let frame = ctx.viewer.movie_current_frame() + 1; // 1-indexed for display
        if !ctx.quiet {
            ctx.print(&format!(" Frame {}.", frame));
        }
        Ok(())
    }
}

// ============================================================================
// backward command
// ============================================================================

struct BackwardCommand;

impl Command for BackwardCommand {
    fn name(&self) -> &str {
        "backward"
    }

    command_help! {
        CMD "backward"
        DESCRIPTION [
            "goes back to the previous frame.",
        ]
        REQUIRED []
        OPTIONAL []
        SEE ALSO [
            "forward, frame, mplay",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        _args: &ParsedCommand,
    ) -> CmdResult {
        ctx.viewer.movie_prev();
        let frame = ctx.viewer.movie_current_frame() + 1; // 1-indexed for display
        if !ctx.quiet {
            ctx.print(&format!(" Frame {}.", frame));
        }
        Ok(())
    }
}

// ============================================================================
// rewind command
// ============================================================================

struct RewindCommand;

impl Command for RewindCommand {
    fn name(&self) -> &str {
        "rewind"
    }

    command_help! {
        CMD "rewind"
        DESCRIPTION [
            "goes to the first frame of the movie.",
        ]
        REQUIRED []
        OPTIONAL []
        SEE ALSO [
            "ending, middle, frame",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        _args: &ParsedCommand,
    ) -> CmdResult {
        ctx.viewer.movie_goto(0);
        if !ctx.quiet {
            ctx.print(" Frame 1.");
        }
        Ok(())
    }
}

// ============================================================================
// middle command
// ============================================================================

struct MiddleCommand;

impl Command for MiddleCommand {
    fn name(&self) -> &str {
        "middle"
    }

    command_help! {
        CMD "middle"
        DESCRIPTION [
            "goes to the middle frame of the movie.",
        ]
        REQUIRED []
        OPTIONAL []
        SEE ALSO [
            "rewind, ending, frame",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        _args: &ParsedCommand,
    ) -> CmdResult {
        let count = ctx.viewer.movie_frame_count();
        if count > 0 {
            let middle = count / 2;
            ctx.viewer.movie_goto(middle);
            if !ctx.quiet {
                ctx.print(&format!(" Frame {}.", middle + 1));
            }
        } else if !ctx.quiet {
            ctx.print(" No movie frames.");
        }
        Ok(())
    }
}

// ============================================================================
// ending command
// ============================================================================

struct EndingCommand;

impl Command for EndingCommand {
    fn name(&self) -> &str {
        "ending"
    }

    command_help! {
        CMD "ending"
        DESCRIPTION [
            "goes to the last frame of the movie.",
        ]
        REQUIRED []
        OPTIONAL []
        SEE ALSO [
            "rewind, middle, frame",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        _args: &ParsedCommand,
    ) -> CmdResult {
        let count = ctx.viewer.movie_frame_count();
        if count > 0 {
            ctx.viewer.movie_goto(count - 1);
            if !ctx.quiet {
                ctx.print(&format!(" Frame {}.", count));
            }
        } else if !ctx.quiet {
            ctx.print(" No movie frames.");
        }
        Ok(())
    }
}

// ============================================================================
// rock command
// ============================================================================

struct RockCommand;

impl Command for RockCommand {
    fn name(&self) -> &str {
        "rock"
    }

    command_help! {
        CMD "rock"
        DESCRIPTION [
            "toggles Y-axis rocking (oscillation) animation.",
            "",
            "When enabled, the view continuously rocks back and forth around",
            "the Y-axis. This is useful for presentations and visual inspection.",
        ]
        REQUIRED []
        OPTIONAL [
            { "mode", "string", "on/off/1/0", "toggle" } => [
                "If omitted, toggles the current state.",
                "\"on\" or \"1\" enables rock mode.",
                "\"off\" or \"0\" disables rock mode.",
            ],
        ]
        EXAMPLES [
            "rock            # Toggle rock mode",
            "rock on         # Enable rock mode",
            "rock off        # Disable rock mode",
            "rock 1          # Enable rock mode",
            "rock 0          # Disable rock mode",
        ]
        SEE ALSO [
            "mplay, turn, rotate",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Keywords(&["on", "off"])]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        // Check for optional mode argument. The parser turns on/off into Bool
        // and 1/0 into Int, so do not read only string arguments here.
        if let Some(mode) = args.get_arg(0) {
            let enabled = mode.as_bool().or_else(|| {
                mode.to_string_repr()
                    .and_then(|mode_str| match mode_str.to_lowercase().as_str() {
                        "on" | "1" | "true" | "yes" => Some(true),
                        "off" | "0" | "false" | "no" => Some(false),
                        _ => None,
                    })
            });
            let enabled = enabled.ok_or_else(|| {
                CmdError::invalid_arg(
                    "mode",
                    format!(
                        "Invalid mode '{}'. Use on/off, 1/0, true/false, or yes/no.",
                        mode
                    ),
                )
            })?;

            ctx.viewer.rock_set(enabled);
            if !ctx.quiet {
                let state = if enabled { "on" } else { "off" };
                ctx.print(&format!(" Rock {}.", state));
            }
        } else {
            // Toggle mode
            ctx.viewer.rock_toggle();
            if !ctx.quiet {
                let state = if ctx.viewer.rock_is_enabled() {
                    "on"
                } else {
                    "off"
                };
                ctx.print(&format!(" Rock {}.", state));
            }
        }
        Ok(())
    }
}

// ============================================================================
// mview command
// ============================================================================

struct MviewCommand;

impl Command for MviewCommand {
    fn name(&self) -> &str {
        "mview"
    }

    command_help! {
        CMD "mview"
        DESCRIPTION [
            "stores, recalls, or clears camera keyframes in the movie.",
        ]
        USAGE [
            "mview action [, frame [, scene=name [, object=name [, state=N]]]]",
        ]
        REQUIRED [
            { "action", "string", "store, recall, clear, interpolate, reinterpolate" },
        ]
        OPTIONAL [
            { "frame", "int", "frame number (1-indexed)", "current frame" },
            { "scene", "string", "scene name for scene keyframes", "" },
            { "object", "string", "object name for object keyframes", "" },
            { "state", "int", "coordinate set state", "" },
        ]
        EXAMPLES [
            "mview store              # Store keyframe at current frame",
            "mview store, 30          # Store keyframe at frame 30",
            "mview store, 1, scene=001   # Store scene keyframe at frame 1",
            "mview interpolate        # Recompute interpolation",
            "mview clear              # Clear all keyframes",
        ]
        SEE ALSO [
            "mset, mplay, mpng, scene",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Keywords(&[
            "store",
            "recall",
            "clear",
            "interpolate",
            "reinterpolate",
        ])]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let action = args
            .get_str(0)
            .ok_or_else(|| CmdError::missing_argument("action".to_string()))?;

        match action.to_lowercase().as_str() {
            "store" => {
                // Get frame (1-indexed), default to current frame
                let frame = args
                    .int_arg(1, "frame")
                    .map(|f| (f as usize).saturating_sub(1))
                    .unwrap_or_else(|| ctx.viewer.movie_current_frame());

                let scene = args.get_named("scene").and_then(|v| v.to_string_repr());
                let object = args.get_named("object").and_then(|v| v.to_string_repr());
                let state = args.get_named_int("state").map(|s| s as usize);

                if let Some(ref scene_name) = scene {
                    ctx.viewer.movie_store_scene(frame, scene_name);
                    if !ctx.quiet {
                        ctx.print(&format!(
                            " Scene \"{}\" keyframe stored at frame {}.",
                            scene_name,
                            frame + 1
                        ));
                    }
                } else if let Some(ref obj_name) = object {
                    ctx.viewer.movie_store_object(frame, obj_name, state);
                    if !ctx.quiet {
                        ctx.print(&format!(
                            " Object \"{}\" keyframe stored at frame {}.",
                            obj_name,
                            frame + 1
                        ));
                    }
                } else {
                    ctx.viewer.movie_store_view(frame);
                    if !ctx.quiet {
                        ctx.print(&format!(" Camera keyframe stored at frame {}.", frame + 1));
                    }
                }
            }

            "recall" => {
                let frame = args
                    .int_arg(1, "frame")
                    .map(|f| (f as usize).saturating_sub(1))
                    .unwrap_or_else(|| ctx.viewer.movie_current_frame());

                ctx.viewer.movie_goto(frame);
                if !ctx.quiet {
                    ctx.print(&format!(" Recalled keyframe at frame {}.", frame + 1));
                }
            }

            "clear" => {
                let frame = args
                    .int_arg(1, "frame")
                    .map(|f| (f as usize).saturating_sub(1));

                ctx.viewer.movie_clear_view(frame);
                if !ctx.quiet {
                    match frame {
                        Some(f) => ctx.print(&format!(" Cleared keyframe at frame {}.", f + 1)),
                        None => ctx.print(" All keyframes cleared."),
                    }
                }
            }

            "interpolate" | "reinterpolate" => {
                ctx.viewer.movie_interpolate();
                if !ctx.quiet {
                    ctx.print(" Keyframes interpolated.");
                }
            }

            _ => {
                return Err(CmdError::invalid_arg(
                    "action",
                    format!(
                        "Unknown action '{}'. Use store, recall, clear, or interpolate.",
                        action
                    ),
                ));
            }
        }

        Ok(())
    }
}

// ============================================================================
// mpng command
// ============================================================================

/// Parsed frame rendering parameters shared by mpng and mproduce.
struct FrameRenderParams {
    first: usize,
    last: usize,
    width: Option<u32>,
    height: Option<u32>,
}

impl FrameRenderParams {
    /// Parse first, last, width, height from command args.
    /// `first_idx`/`last_idx`/`width_idx`/`height_idx` are positional arg indices.
    fn parse(
        args: &ParsedCommand,
        total: usize,
        first_idx: usize,
        last_idx: usize,
        width_idx: usize,
        height_idx: usize,
    ) -> Self {
        let first = args
            .int_arg(first_idx, "first")
            .map(|f| (f as usize).saturating_sub(1))
            .unwrap_or(0);

        let last = args
            .int_arg(last_idx, "last")
            .map(|f| (f as usize).saturating_sub(1))
            .unwrap_or(total - 1);

        let width = args.int_arg(width_idx, "width").map(|w| w as u32);

        let height = args.int_arg(height_idx, "height").map(|h| h as u32);

        Self {
            first,
            last,
            width,
            height,
        }
    }

    fn frame_count(&self) -> usize {
        self.last - self.first + 1
    }

    /// Ensure width/height are even (required for H.264/VP9).
    fn make_even(&mut self) {
        self.width = self.width.map(|w| if w % 2 != 0 { w - 1 } else { w });
        self.height = self.height.map(|h| if h % 2 != 0 { h - 1 } else { h });
    }
}

/// Render movie frames to PNG files: `{prefix}0001.png`, `{prefix}0002.png`, ...
///
/// Frames are numbered starting from 1 relative to `first`.
/// Creates the parent directory of `prefix` if it doesn't exist.
fn render_frames_to_png(
    ctx: &mut CommandContext<'_, '_, dyn ViewerLike + '_>,
    prefix: &str,
    params: &FrameRenderParams,
) -> CmdResult {
    let parent = std::path::Path::new(prefix).parent();
    if let Some(dir) = parent {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).map_err(|e| {
                CmdError::invalid_arg("prefix", format!("Cannot create directory: {}", e))
            })?;
        }
    }

    for frame in params.first..=params.last {
        let idx = frame - params.first + 1;
        let path = format!("{}{:04}.png", prefix, idx);
        ctx.viewer
            .capture_frame_png(
                frame,
                std::path::Path::new(&path),
                params.width,
                params.height,
            )
            .map_err(|e| CmdError::invalid_arg("render", format!("Frame {}: {}", frame + 1, e)))?;

        if !ctx.quiet {
            ctx.print(&format!(
                " Ray: frame {} of {} ({}).",
                idx,
                params.frame_count(),
                path
            ));
        }
    }
    Ok(())
}

/// Validate that the movie has frames, returning the total count.
fn require_movie_frames(
    ctx: &CommandContext<'_, '_, dyn ViewerLike + '_>,
) -> Result<usize, CmdError> {
    let total = ctx.viewer.movie_frame_count();
    if total == 0 {
        return Err(CmdError::invalid_arg(
            "movie",
            "No movie frames. Use mset first.",
        ));
    }
    Ok(total)
}

struct MpngCommand;

impl Command for MpngCommand {
    fn name(&self) -> &str {
        "mpng"
    }

    command_help! {
        CMD "mpng"
        DESCRIPTION [
            "renders movie frames to a PNG image sequence.",
        ]
        USAGE [
            "mpng prefix [, first [, last [, preserve [, width [, height]]]]]",
        ]
        REQUIRED [
            { "prefix", "string", "output path prefix (e.g., /tmp/movie/frame)" },
        ]
        OPTIONAL [
            { "first", "int", "first frame to render (1-indexed)", "1" },
            { "last", "int", "last frame to render (1-indexed)", "last frame" },
            { "preserve", "int", "unused (PyMOL compat)", "0" },
            { "width", "int", "image width in pixels", "viewport width" },
            { "height", "int", "image height in pixels", "viewport height" },
        ]
        EXAMPLES [
            "mpng /tmp/movie/frame           # Render all frames",
            "mpng /tmp/movie/frame, 1, 30    # Render frames 1-30",
            "mpng /tmp/movie/frame, width=1920, height=1080",
        ]
        SEE ALSO [
            "mset, mview, mplay",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Path]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let prefix = args
            .get_str(0)
            .ok_or_else(|| CmdError::missing_argument("prefix".to_string()))?;

        let total = require_movie_frames(ctx)?;
        // Positional: prefix(0), first(1), last(2), preserve(3), width(4), height(5)
        let params = FrameRenderParams::parse(args, total, 1, 2, 4, 5);

        if !ctx.quiet {
            ctx.print(&format!(
                " Rendering {} frames to {}NNNN.png...",
                params.frame_count(),
                prefix
            ));
        }

        render_frames_to_png(ctx, prefix, &params)?;

        if !ctx.quiet {
            ctx.print(&format!(
                " {} frames rendered to {}NNNN.png.",
                params.frame_count(),
                prefix
            ));
        }
        Ok(())
    }
}

// ============================================================================
// mproduce command
// ============================================================================

struct ProduceCommand;

impl Command for ProduceCommand {
    fn name(&self) -> &str {
        "mproduce"
    }

    command_help! {
        CMD "mproduce"
        DESCRIPTION [
            "exports a movie to a video file using ffmpeg.",
            "",
            "Renders each frame to a temporary PNG sequence, then encodes:",
            "  - MP4/MOV: H.264 codec, yuv420p pixel format",
            "  - WebM: VP9 codec",
            "",
            "Requires ffmpeg to be installed and available in $PATH.",
        ]
        USAGE [
            "mproduce filename [, first [, last [, preserve [, quality [, width [, height]]]]]]",
        ]
        REQUIRED [
            { "filename", "string", "output file path (e.g., movie.mp4, movie.webm)" },
        ]
        OPTIONAL [
            { "first", "int", "first frame to export (1-indexed)", "1" },
            { "last", "int", "last frame to export (1-indexed)", "last frame" },
            { "preserve", "int", "keep temporary PNG files (0 or 1)", "0" },
            { "quality", "int", "encoding quality 0-100", "90" },
            { "width", "int", "frame width in pixels", "viewport width" },
            { "height", "int", "frame height in pixels", "viewport height" },
        ]
        EXAMPLES [
            "mproduce movie.mp4",
            "mproduce movie.mp4, 1, 60, quality=95",
            "mproduce movie.webm, width=1920, height=1080",
            "mproduce movie.mov, width=1920, height=1080",
        ]
        SEE ALSO [
            "mpng, mset, mview, mplay",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Path]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        use std::process::Command as ProcessCommand;

        // Parse filename
        let filename_raw = args
            .get_str(0)
            .ok_or_else(|| CmdError::missing_argument("filename".to_string()))?;

        let filename = super::io::expand_path(filename_raw);

        // Validate extension
        let ext = filename.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "mp4" | "mov" | "webm") {
            return Err(CmdError::invalid_arg(
                "filename",
                format!("Unsupported format \".{}\". Use .mp4, .mov, or .webm.", ext),
            ));
        }

        // Check ffmpeg availability
        let ffmpeg_ok = ProcessCommand::new("ffmpeg")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if ffmpeg_ok.is_err() || !ffmpeg_ok.unwrap().success() {
            return Err(CmdError::invalid_arg(
                "ffmpeg",
                "ffmpeg not found in $PATH. Install ffmpeg to produce videos.",
            ));
        }

        let total = require_movie_frames(ctx)?;
        // Positional: filename(0), first(1), last(2), preserve(3), quality(4), width(5), height(6)
        let mut params = FrameRenderParams::parse(args, total, 1, 2, 5, 6);
        params.make_even();

        let preserve = args.int_arg_or(3, "preserve", 0) != 0;

        let quality = args.int_arg_or(4, "quality", 90).clamp(0, 100);

        // Get FPS from settings
        let fps = ctx.viewer.settings().movie.movie_fps;

        // Create temp directory
        let tmp_path = filename.with_extension("tmp");
        if tmp_path.exists() && !preserve {
            let _ = std::fs::remove_dir_all(&tmp_path);
        }
        std::fs::create_dir_all(&tmp_path).map_err(|e| {
            CmdError::invalid_arg("tmp_dir", format!("Cannot create temp directory: {}", e))
        })?;

        let prefix = tmp_path.join("mov");
        let prefix_str = prefix.display().to_string();

        if !ctx.quiet {
            ctx.print(&format!(
                " mproduce: rendering {} frames...",
                params.frame_count()
            ));
        }

        render_frames_to_png(ctx, &prefix_str, &params)?;

        if !ctx.quiet {
            ctx.print(" mproduce: encoding with ffmpeg...");
        }

        // Build ffmpeg arguments based on output format
        let input_pattern = format!("{}%04d.png", prefix_str);
        let fps_str = format!("{:.3}", fps);

        let mut ffmpeg_args: Vec<&str> = vec![
            "-y",
            "-f",
            "image2",
            "-framerate",
            &fps_str,
            "-i",
            &input_pattern,
        ];

        let crf_str;
        if ext == "webm" {
            // VP9 CRF derived from the requested quality.
            crf_str = format!("{:.0}", 65.0 - (quality as f64 / 2.0));
            ffmpeg_args.extend_from_slice(&["-c:v", "libvpx-vp9", "-b:v", "0", "-crf", &crf_str]);
        } else {
            // H.264 for MP4/MOV
            crf_str = if quality > 90 {
                "10".to_string()
            } else if quality > 80 {
                "15".to_string()
            } else {
                "20".to_string()
            };
            ffmpeg_args.extend_from_slice(&["-crf", &crf_str, "-pix_fmt", "yuv420p"]);
        }

        // Run ffmpeg
        let output = ProcessCommand::new("ffmpeg")
            .args(&ffmpeg_args)
            .arg(filename.as_os_str())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output()
            .map_err(|e| CmdError::invalid_arg("ffmpeg", format!("Failed to run ffmpeg: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !preserve {
                let _ = std::fs::remove_dir_all(&tmp_path);
            }
            return Err(CmdError::invalid_arg(
                "ffmpeg",
                format!("ffmpeg exited with status {}:\n{}", output.status, stderr),
            ));
        }

        // Clean up temp dir
        if !preserve {
            let _ = std::fs::remove_dir_all(&tmp_path);
        }

        if !ctx.quiet {
            ctx.print(&format!(
                " mproduce: finished. Saved to \"{}\".",
                filename.display()
            ));
        }

        Ok(())
    }
}

// ============================================================================
// madd command
// ============================================================================

struct MaddCommand;

impl Command for MaddCommand {
    fn name(&self) -> &str {
        "madd"
    }

    command_help! {
        CMD "madd"
        DESCRIPTION [
            "appends frames to the movie using the same specification",
            "syntax as mset.",
        ]
        REQUIRED [
            { "specification", "string", "frame specification (same syntax as mset)" },
        ]
        OPTIONAL []
        EXAMPLES [
            "madd 1 x30          # Append 30 frames of state 1",
            "madd 1 -60          # Append frames for states 1-60",
        ]
        SEE ALSO [
            "mset, mview, mplay",
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let states = parse_mset_spec(args)?;
        let count = states.len();
        ctx.viewer.movie_append_from_spec(states);

        if !ctx.quiet {
            ctx.print(&format!(
                " Movie: appended {} frames ({} total).",
                count,
                ctx.viewer.movie_frame_count()
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::ParsedCommand;
    use crate::parser::parse_command;
    use crate::CommandExecutor;
    use patinae_scene::{Session, SessionAdapter};

    #[test]
    fn test_mset_simple_count() {
        let cmd = ParsedCommand::new("mset").with_arg(ArgValue::Int(60));
        let states = parse_mset_spec(&cmd).unwrap();
        assert_eq!(states.len(), 60);
        assert!(states.iter().all(|&s| s == 1));
    }

    #[test]
    fn test_mset_1_x60() {
        // "mset 1 x60" → parser produces Int(1), String("x60")
        let cmd = ParsedCommand::new("mset")
            .with_arg(ArgValue::Int(1))
            .with_arg(ArgValue::String("x60".to_string()));
        let states = parse_mset_spec(&cmd).unwrap();
        assert_eq!(states.len(), 60);
        assert!(states.iter().all(|&s| s == 1));
    }

    #[test]
    fn test_mset_range() {
        // "mset 1 -30" → parser may produce Int(1), Int(-30) or Int(1), String("-30")
        let cmd = ParsedCommand::new("mset")
            .with_arg(ArgValue::Int(1))
            .with_arg(ArgValue::Int(-30));
        let states = parse_mset_spec(&cmd).unwrap();
        assert_eq!(states.len(), 30);
        assert_eq!(states[0], 1);
        assert_eq!(states[29], 30);
    }

    #[test]
    fn test_mset_combined() {
        // "mset 1 x10 2 x10" → 10 of state 1, then 10 of state 2
        let cmd = ParsedCommand::new("mset")
            .with_arg(ArgValue::Int(1))
            .with_arg(ArgValue::String("x10".to_string()))
            .with_arg(ArgValue::Int(2))
            .with_arg(ArgValue::String("x10".to_string()));
        let states = parse_mset_spec(&cmd).unwrap();
        assert_eq!(states.len(), 20);
        assert!(states[..10].iter().all(|&s| s == 1));
        assert!(states[10..].iter().all(|&s| s == 2));
    }

    #[test]
    fn test_mset_mixed() {
        // "mset 1 x30 1 -60" → 30 of state 1, then states 1..60
        let cmd = ParsedCommand::new("mset")
            .with_arg(ArgValue::Int(1))
            .with_arg(ArgValue::String("x30".to_string()))
            .with_arg(ArgValue::Int(1))
            .with_arg(ArgValue::Int(-60));
        let states = parse_mset_spec(&cmd).unwrap();
        assert_eq!(states.len(), 90); // 30 + 60
        assert!(states[..30].iter().all(|&s| s == 1));
        assert_eq!(states[30], 1);
        assert_eq!(states[89], 60);
    }

    #[test]
    fn test_parse_mset_1_x60_from_parser() {
        let cmd = parse_command("mset 1 x60").unwrap();
        assert_eq!(cmd.name, "mset");
        let states = parse_mset_spec(&cmd).unwrap();
        assert_eq!(states.len(), 60);
        assert!(states.iter().all(|&s| s == 1));
    }

    #[test]
    fn test_parse_mview_store() {
        let cmd = parse_command("mview store").unwrap();
        assert_eq!(cmd.name, "mview");
        assert_eq!(cmd.get_str(0), Some("store"));
    }

    #[test]
    fn test_parse_mview_store_frame() {
        let cmd = parse_command("mview store, 30").unwrap();
        assert_eq!(cmd.name, "mview");
        assert_eq!(cmd.get_str(0), Some("store"));
        assert_eq!(cmd.get_int(1), Some(30));
    }

    #[test]
    fn test_parse_mview_store_scene() {
        let cmd = parse_command("mview store, 1, scene=001").unwrap();
        assert_eq!(cmd.name, "mview");
        assert_eq!(cmd.get_str(0), Some("store"));
        assert_eq!(cmd.get_int(1), Some(1));
        // scene=001 may parse as Int(1) or String("001") depending on parser
        let scene = cmd.get_named("scene").and_then(|v| v.to_string_repr());
        assert_eq!(scene.as_deref(), Some("1"));
    }

    #[test]
    fn test_parse_mpng() {
        let cmd = parse_command("mpng /tmp/movie_test/frame").unwrap();
        assert_eq!(cmd.name, "mpng");
        assert_eq!(cmd.get_str(0), Some("/tmp/movie_test/frame"));
    }

    #[test]
    fn test_parse_mview_interpolate() {
        let cmd = parse_command("mview interpolate").unwrap();
        assert_eq!(cmd.name, "mview");
        assert_eq!(cmd.get_str(0), Some("interpolate"));
    }

    fn execute_rock(command: &str, initial: bool) -> Result<bool, CmdError> {
        let mut session = Session::new();
        session.movie.set_rock(initial);
        let mut needs_redraw = false;
        {
            let mut adapter = SessionAdapter {
                session: &mut session,
                render_context: None,
                default_size: (800, 600),
                needs_redraw: &mut needs_redraw,
            };
            let mut executor = CommandExecutor::new();
            executor.do_with_options(&mut adapter, command, true)?;
        }
        Ok(session.movie.is_rock_enabled())
    }

    #[test]
    fn test_rock_modes_are_idempotent() {
        assert!(execute_rock("rock on", false).unwrap());
        assert!(execute_rock("rock 1", false).unwrap());
        assert!(!execute_rock("rock off", true).unwrap());
        assert!(!execute_rock("rock 0", true).unwrap());
        assert!(!execute_rock("rock off", false).unwrap());
        assert!(execute_rock("rock on", true).unwrap());
    }

    #[test]
    fn test_rock_without_mode_toggles() {
        assert!(execute_rock("rock", false).unwrap());
        assert!(!execute_rock("rock", true).unwrap());
    }

    #[test]
    fn test_rock_invalid_mode_errors() {
        let err = execute_rock("rock sideways", false).unwrap_err();
        assert!(err.to_string().contains("Invalid mode"));
    }

    #[test]
    fn test_parse_mview_store_scene_002() {
        let cmd = parse_command("mview store, 30, scene=002").unwrap();
        assert_eq!(cmd.name, "mview");
        assert_eq!(cmd.get_str(0), Some("store"));
        assert_eq!(cmd.get_int(1), Some(30));
        let scene = cmd.get_named("scene").and_then(|v| v.to_string_repr());
        assert_eq!(scene.as_deref(), Some("2"));
    }
}
