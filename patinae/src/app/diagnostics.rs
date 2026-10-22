//! Diagnostics for the native application.

use super::*;

impl App {
    /// Refresh the `PATINAE_PERF=1` HUD overlay. Throttled to ~12 Hz so
    /// the Slint property writes don't fight render-loop priority.
    pub(super) fn update_perf_hud(
        &self,
        vp: &ViewportState,
        gpu_stats: Option<&patinae_render::FrameStats>,
    ) {
        const REFRESH_EVERY: u32 = 10;
        let n = self.diagnostics.perf_update_counter.get().wrapping_add(1);
        self.diagnostics.perf_update_counter.set(n);
        if !n.is_multiple_of(REFRESH_EVERY) {
            return;
        }
        let history = self.diagnostics.perf_history.borrow();
        if history.is_empty() {
            return;
        }
        let avg_ms = history.avg_ms();
        let low_1 = history.low_1pct_ms();
        let low_01 = history.low_0_1pct_ms();
        vp.set_perf_visible(true);
        // Labels ("avg" / "1% low" / "0.1% low") live in the HUD; feed values only.
        vp.set_perf_avg_text(format!("{:.2} ms", avg_ms).into());
        vp.set_perf_low_1pct_text(format!("{:.2} ms", low_1).into());
        vp.set_perf_low_0_1pct_text(format!("{:.2} ms", low_01).into());
        if let Some(renderer) = self.rendering.renderer.as_ref() {
            let profile = renderer.memory_profile();
            let memory = renderer.memory_snapshot();
            vp.set_perf_memory_profile_label(memory_profile_hud_label(profile).into());
            vp.set_perf_memory_total_text(
                memory_total_hud_text(memory.total_capacity_bytes(), profile.budget_bytes()).into(),
            );
            vp.set_perf_memory_live_text(format_hud_mib(memory.total_live_bytes()).into());
            vp.set_perf_memory_fill(memory_budget_fill(
                memory.total_capacity_bytes(),
                profile.budget_bytes(),
            ));
        }
        if let Some(s) = gpu_stats {
            // Sentinel `-1.0` means the pass didn't run this frame.
            let fmt = |label: &str, ms: f32| -> String {
                if ms < 0.0 {
                    String::new()
                } else {
                    format!("{label} {ms:.2}")
                }
            };
            let mut parts: Vec<String> = Vec::with_capacity(6);
            for (label, ms) in [
                ("cull", s.gpu_cull_ms),
                ("shadow", s.gpu_shadow_ms),
                ("atlas_ao", s.gpu_atlas_ao_ms),
                ("opaque", s.gpu_opaque_ms),
                ("mark", s.gpu_marking_ms),
                ("translucent", s.gpu_translucent_ms),
            ] {
                let s = fmt(label, ms);
                if !s.is_empty() {
                    parts.push(s);
                }
            }
            vp.set_perf_gpu_text(parts.join(" · ").into());
        }
    }
}

pub(super) fn format_mib(bytes: u64) -> String {
    format!("{:.2} MiB", bytes_to_mib(bytes))
}

pub(super) fn format_hud_mib(bytes: u64) -> String {
    let mib = bytes_to_mib(bytes);
    if mib >= 100.0 {
        format!("{mib:.0} MiB")
    } else {
        format!("{mib:.1} MiB")
    }
}

pub(super) fn memory_profile_hud_label(profile: RenderMemoryProfile) -> &'static str {
    match profile {
        RenderMemoryProfile::Performance => "PERF",
        RenderMemoryProfile::Balanced => "BAL",
        RenderMemoryProfile::Lite => "LITE",
        RenderMemoryProfile::Manual { .. } => "MAN",
    }
}

pub(super) fn memory_total_hud_text(total_bytes: u64, budget_bytes: Option<u64>) -> String {
    let total = format_hud_mib(total_bytes);
    match budget_bytes {
        Some(budget_bytes) => format!("{total} / {}", format_hud_mib(budget_bytes)),
        None => total,
    }
}

/// Fraction of the memory budget currently allocated, clamped to `0..=1`.
/// Returns `-1.0` when the profile has no budget (unbounded) so the HUD can
/// hide the usage bar.
pub(super) fn memory_budget_fill(total_bytes: u64, budget_bytes: Option<u64>) -> f32 {
    match budget_bytes {
        Some(budget) if budget > 0 => (total_bytes as f32 / budget as f32).clamp(0.0, 1.0),
        _ => -1.0,
    }
}

/// Owns diagnostics state.
pub(super) struct Diagnostics {
    pub(super) perf_enabled: bool,
    pub(super) last_frame_instant: Cell<Option<Instant>>,
    pub(super) perf_history: RefCell<FrameStatsHistory>,
    pub(super) perf_update_counter: Cell<u32>,
}
