//! Typed movie events and their UI-thread handler.

use super::*;
use crate::app::{
    events::{EventSender, GuiEvent},
    App,
};

#[derive(Debug)]
pub(crate) enum Event {
    TogglePlay,
    Stop,
    GotoFrame(i32),
    Scrub(f32),
    SetFrame(slint::SharedString),
    StepFrame(i32),
    SetFps(slint::SharedString),
    ToggleLoop,
    ToggleRock,
    ToggleAutoInterpolate,
    StoreCameraKeyframe,
    StoreSceneKeyframe,
    StoreObjectKeyframe,
    Interpolate,
    RequestExport,
}

pub(crate) fn setup_callbacks(sender: EventSender, window: &AppWindow) {
    let state = window.global::<MovieState>();
    {
        let sender = sender.clone();
        state.on_toggle_play(move || sender.send(GuiEvent::Movie(Event::TogglePlay)));
    }
    {
        let sender = sender.clone();
        state.on_stop(move || sender.send(GuiEvent::Movie(Event::Stop)));
    }
    {
        let sender = sender.clone();
        state.on_goto_frame(move |arg0| sender.send(GuiEvent::Movie(Event::GotoFrame(arg0))));
    }
    {
        let sender = sender.clone();
        state.on_scrub(move |arg0| sender.send(GuiEvent::Movie(Event::Scrub(arg0))));
    }
    {
        let sender = sender.clone();
        state.on_set_frame(move |arg0| sender.send(GuiEvent::Movie(Event::SetFrame(arg0))));
    }
    {
        let sender = sender.clone();
        state.on_step_frame(move |arg0| sender.send(GuiEvent::Movie(Event::StepFrame(arg0))));
    }
    {
        let sender = sender.clone();
        state.on_set_fps(move |arg0| sender.send(GuiEvent::Movie(Event::SetFps(arg0))));
    }
    {
        let sender = sender.clone();
        state.on_toggle_loop(move || sender.send(GuiEvent::Movie(Event::ToggleLoop)));
    }
    {
        let sender = sender.clone();
        state.on_toggle_rock(move || sender.send(GuiEvent::Movie(Event::ToggleRock)));
    }
    {
        let sender = sender.clone();
        state.on_toggle_auto_interpolate(move || {
            sender.send(GuiEvent::Movie(Event::ToggleAutoInterpolate))
        });
    }
    {
        let sender = sender.clone();
        state.on_store_camera_keyframe(move || {
            sender.send(GuiEvent::Movie(Event::StoreCameraKeyframe))
        });
    }
    {
        let sender = sender.clone();
        state.on_store_scene_keyframe(move || {
            sender.send(GuiEvent::Movie(Event::StoreSceneKeyframe))
        });
    }
    {
        let sender = sender.clone();
        state.on_store_object_keyframe(move || {
            sender.send(GuiEvent::Movie(Event::StoreObjectKeyframe))
        });
    }
    {
        let sender = sender.clone();
        state.on_interpolate(move || sender.send(GuiEvent::Movie(Event::Interpolate)));
    }
    {
        let sender = sender.clone();
        state.on_request_export(move || sender.send(GuiEvent::Movie(Event::RequestExport)));
    }
}

pub(in crate::app) fn dispatch(event: Event, app: &mut App, _window: &AppWindow) {
    match event {
        Event::TogglePlay => {
            let a = &mut *app;
            let cmd = if a.kernel.session.movie.is_playing() {
                "mpause"
            } else {
                "mplay"
            };
            a.kernel.bus.execute_command_silent(cmd);
        }
        Event::Stop => {
            app.kernel.bus.execute_command_silent("mstop");
        }
        Event::GotoFrame(frame) => {
            let a = &mut *app;
            let count = a.kernel.session.movie.effective_frame_count().max(1);
            let frame = (frame as usize).clamp(1, count);
            a.kernel
                .bus
                .execute_command_silent(format!("frame {frame}"));
        }
        Event::Scrub(pos) => {
            let a = &mut *app;
            let count = a.kernel.session.movie.effective_frame_count().max(1);
            let pos = pos.clamp(0.0, 1.0);
            let frame = ((pos * count as f32).floor() as usize + 1).clamp(1, count);
            a.kernel
                .bus
                .execute_command_silent(format!("frame {frame}"));
        }
        Event::SetFrame(text) => {
            let a = &mut *app;
            let count = a.kernel.session.movie.effective_frame_count().max(1);
            if let Some(frame) = parse_frame(&text, count) {
                a.kernel
                    .bus
                    .execute_command_silent(format!("frame {frame}"));
            } else {
                a.kernel
                    .bus
                    .print_warning(format!("Invalid movie frame '{}'.", text.trim()));
            }
        }
        Event::StepFrame(delta) => {
            let a = &mut *app;
            if delta > 0 {
                a.kernel.bus.execute_command_silent("forward");
            } else if delta < 0 {
                a.kernel.bus.execute_command_silent("backward");
            }
        }
        Event::SetFps(text) => {
            let a = &mut *app;
            if let Some(fps) = parse_fps(&text) {
                a.kernel
                    .bus
                    .execute_command_silent(format!("set movie_fps, {fps:.3}"));
            } else {
                a.kernel
                    .bus
                    .print_warning(format!("Invalid movie FPS '{}'.", text.trim()));
            }
        }
        Event::ToggleLoop => {
            let a = &mut *app;
            let next = if a.kernel.session.settings.movie.movie_loop {
                "off"
            } else {
                "on"
            };
            a.kernel
                .bus
                .execute_command_silent(format!("set movie_loop, {next}"));
        }
        Event::ToggleRock => {
            let a = &mut *app;
            let next = if a.kernel.session.movie.is_rock_enabled() {
                "off"
            } else {
                "on"
            };
            a.kernel.bus.execute_command_silent(format!("rock {next}"));
        }
        Event::ToggleAutoInterpolate => {
            let a = &mut *app;
            let next = if a.kernel.session.settings.movie.movie_auto_interpolate {
                "off"
            } else {
                "on"
            };
            a.kernel
                .bus
                .execute_command_silent(format!("set movie_auto_interpolate, {next}"));
        }
        Event::StoreCameraKeyframe => {
            let a = &mut *app;
            let frame = a.kernel.session.movie.current_frame() + 1;
            a.kernel
                .bus
                .execute_command(format!("mview store, {frame}"));
        }
        Event::StoreSceneKeyframe => {
            let a = &mut *app;
            let frame = a.kernel.session.movie.current_frame() + 1;
            if let Some(scene) = a.kernel.session.scenes.current().map(ToOwned::to_owned) {
                a.kernel.bus.execute_command(format!(
                    "mview store, {frame}, scene={}",
                    quote_command_arg(&scene)
                ));
            } else {
                a.kernel
                    .bus
                    .print_warning("No active scene to store as a movie keyframe.");
            }
        }
        Event::StoreObjectKeyframe => {
            let a = &mut *app;
            let frame = a.kernel.session.movie.current_frame() + 1;
            let Some(object) = a.objects.single_movie_keyframe_object() else {
                a.kernel
                    .bus
                    .print_warning("Select exactly one object to store an object movie keyframe.");
                return;
            };
            let state = a
                .kernel
                .session
                .registry
                .get(&object)
                .map(|obj| obj.current_state() + 1)
                .unwrap_or(1);
            a.kernel.bus.execute_command(format!(
                "mview store, {frame}, object={}, state={state}",
                quote_command_arg(&object)
            ));
        }
        Event::Interpolate => {
            app.kernel.bus.execute_command("mview interpolate");
        }
        Event::RequestExport => {
            app.kernel.bus.print_info(
                "Movie export UI is planned for the next panel. For now use `mproduce filename.mp4` in the REPL.",
            );
        }
    }
}
