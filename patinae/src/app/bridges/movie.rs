use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, VecModel};

use patinae_framework::kernel::AppKernel;
use patinae_framework::model::{
    MovieBlock as CoreMovieBlock, MovieLane as CoreMovieLane, MovieMarker as CoreMovieMarker,
    MovieTick as CoreMovieTick, MovieTimelineModel,
};

use crate::{AppWindow, MovieBlock, MovieLane, MovieMarker, MovieState, MovieTick};

// ---------------------------------------------------------------------------
// MovieBridge
// ---------------------------------------------------------------------------

pub struct MovieBridge {
    timeline: MovieTimelineModel,
    lane_model: Rc<VecModel<MovieLane>>,
    tick_model: Rc<VecModel<MovieTick>>,
}

impl MovieBridge {
    pub fn new() -> Self {
        Self {
            timeline: MovieTimelineModel::default(),
            lane_model: Rc::new(VecModel::default()),
            tick_model: Rc::new(VecModel::default()),
        }
    }

    /// Attach the timeline models to Slint globals (call once after window creation).
    pub fn attach(&self, window: &AppWindow) {
        let ms = window.global::<MovieState>();
        ms.set_lanes(ModelRc::from(self.lane_model.clone()));
        ms.set_ticks(ModelRc::from(self.tick_model.clone()));
    }

    /// Sync movie state and cached timeline lanes to Slint.
    pub fn sync(
        &mut self,
        kernel: &AppKernel,
        window: &AppWindow,
        selected_object: Option<String>,
    ) {
        let ms = window.global::<MovieState>();
        let snapshot = kernel.session.movie_state_snapshot();
        let frame_count = snapshot.frame_count.max(1);
        let current_frame = snapshot.current_frame.min(frame_count - 1) + 1;

        ms.set_frame_count(frame_count as i32);
        ms.set_current_frame(current_frame as i32);
        if !ms.get_frame_editing() {
            ms.set_frame_input(current_frame.to_string().into());
        }
        if !ms.get_fps_editing() {
            ms.set_fps(format_fps(kernel.session.settings.movie.movie_fps).into());
        }
        ms.set_is_playing(snapshot.is_playing);
        ms.set_loop_enabled(kernel.session.settings.movie.movie_loop);
        ms.set_rock_enabled(snapshot.rock_enabled);
        ms.set_auto_interpolate(kernel.session.settings.movie.movie_auto_interpolate);
        ms.set_can_store_scene(kernel.session.scenes.current().is_some());

        let can_store_object = selected_object
            .as_deref()
            .map(|name| kernel.session.registry.contains(name))
            .unwrap_or(false);
        ms.set_can_store_object(can_store_object);
        ms.set_target_object(selected_object.unwrap_or_default().into());

        if self.timeline.sync(&kernel.session) {
            let timeline = self.timeline.timeline();
            replace_model(
                &self.tick_model,
                timeline.ticks.iter().map(to_slint_tick).collect(),
            );
            replace_model(
                &self.lane_model,
                timeline.lanes.iter().map(to_slint_lane).collect(),
            );
        }
    }
}

impl Default for MovieBridge {
    fn default() -> Self {
        Self::new()
    }
}

fn to_slint_tick(tick: &CoreMovieTick) -> MovieTick {
    MovieTick {
        frame: tick.frame as i32,
        label: tick.label.clone().into(),
        pos: tick.position,
    }
}

fn to_slint_marker(marker: &CoreMovieMarker) -> MovieMarker {
    MovieMarker {
        frame: marker.frame as i32,
        pos: marker.position,
        kind: marker.kind.clone().into(),
        label: marker.label.clone().into(),
    }
}

fn to_slint_block(block: &CoreMovieBlock) -> MovieBlock {
    MovieBlock {
        start_frame: block.start_frame as i32,
        end_frame: block.end_frame as i32,
        start_pos: block.start_position,
        end_pos: block.end_position,
        kind: block.kind.clone().into(),
        label: block.label.clone().into(),
    }
}

fn to_slint_lane(lane: &CoreMovieLane) -> MovieLane {
    let markers: Vec<MovieMarker> = lane.markers.iter().map(to_slint_marker).collect();
    let blocks: Vec<MovieBlock> = lane.blocks.iter().map(to_slint_block).collect();

    MovieLane {
        id: lane.id.clone().into(),
        label: lane.label.clone().into(),
        markers: ModelRc::from(Rc::new(VecModel::from(markers))),
        blocks: ModelRc::from(Rc::new(VecModel::from(blocks))),
    }
}

fn replace_model<T: Clone + 'static>(model: &Rc<VecModel<T>>, rows: Vec<T>) {
    while model.row_count() > 0 {
        model.remove(model.row_count() - 1);
    }
    for row in rows {
        model.push(row);
    }
}

fn format_fps(fps: f32) -> String {
    if (fps - fps.round()).abs() < 0.01 {
        format!("{:.0}", fps)
    } else {
        format!("{:.1}", fps)
    }
}

fn quote_command_arg(s: &str) -> String {
    if s.chars()
        .any(|c| c.is_whitespace() || matches!(c, ',' | '"' | '\''))
    {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn parse_frame(text: &str, frame_count: usize) -> Option<usize> {
    let frame = text.trim().parse::<usize>().ok()?;
    Some(frame.clamp(1, frame_count.max(1)))
}

fn parse_fps(text: &str) -> Option<f32> {
    let fps = text.trim().parse::<f32>().ok()?;
    if fps.is_finite() && fps > 0.0 {
        Some(fps.clamp(1.0, 120.0))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Callbacks
// ---------------------------------------------------------------------------

pub(crate) mod events;
pub(crate) use events::setup_callbacks;
