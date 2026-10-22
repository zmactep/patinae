//! Converts native events into owned, event-time input packets.
use super::input_events::Event;
use super::*;

pub(super) fn install(sender: events::EventSender, window: &AppWindow) {
    use slint::winit_030::winit::{
        event::{ElementState, WindowEvent},
        keyboard::{Key, NamedKey, PhysicalKey},
    };
    use slint::winit_030::{EventResult, WinitWindowAccessor};
    let weak = window.as_weak();
    // Capture metadata is local to ingress; application state changes only in dispatch.
    let mut cursor = (0.0f32, 0.0f32);
    window
        .window()
        .on_winit_window_event(move |_native, event| {
            let Some(w) = weak.upgrade() else {
                return EventResult::Propagate;
            };
            let mut modifiers = sender.captured_modifiers().unwrap_or_default();
            let sf = w.window().scale_factor().max(f32::EPSILON);
            let viewport = App::viewport_size(&w);
            let modal = w.global::<crate::PluginListState>().get_visible()
                || w.global::<MenuState>().get_fetch_visible();
            if modal && matches!(event, WindowEvent::DroppedFile(_) | WindowEvent::Ime(_)) {
                return EventResult::PreventDefault;
            }
            let input = match event {
                WindowEvent::CursorMoved { position, .. } => {
                    cursor = (position.x as f32, position.y as f32);
                    let local = (
                        cursor.0 / sf - w.get_scene_x(),
                        cursor.1 / sf - w.get_scene_y(),
                    );
                    Some(Event::Cursor {
                        blocked: modal
                            || local.0 < 0.0
                            || local.1 < 0.0
                            || local.0 * sf >= viewport.0 as f32
                            || local.1 * sf >= viewport.1 as f32
                            || w.invoke_overlay_contains_point(local.0, local.1),
                        point: cursor,
                        local: (
                            cursor.0 / sf - w.get_scene_x(),
                            cursor.1 / sf - w.get_scene_y(),
                        ),
                        scale: sf,
                        viewport,
                    })
                }
                WindowEvent::CursorLeft { .. } => Some(Event::Focus(false)),
                WindowEvent::Focused(value) => {
                    if !value {
                        modifiers = patinae_scene::Modifiers::default();
                    }
                    Some(Event::Focus(*value))
                }
                WindowEvent::Resized(_)
                | WindowEvent::ScaleFactorChanged { .. }
                | WindowEvent::Occluded(false) => Some(Event::Display),
                WindowEvent::ModifiersChanged(value) => {
                    let value = value.state();
                    modifiers = patinae_scene::Modifiers {
                        shift: value.shift_key(),
                        ctrl: value.control_key(),
                        alt: value.alt_key(),
                        logo: value.super_key(),
                    };
                    Some(Event::Modifiers(modifiers))
                }
                WindowEvent::DroppedFile(path) => Some(Event::Drop(path.clone())),
                WindowEvent::PinchGesture { delta, .. } => {
                    sender.send(events::GuiEvent::Input(Event::Pinch(*delta)));
                    return EventResult::PreventDefault;
                }
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button,
                    ..
                } => {
                    // Release outside the viewport still terminates its held gesture.
                    // A subsequent Slint release is idempotent in InputBridge.
                    let button = match button {
                        slint::winit_030::winit::event::MouseButton::Left => 0,
                        slint::winit_030::winit::event::MouseButton::Middle => 1,
                        slint::winit_030::winit::event::MouseButton::Right => 2,
                        _ => return EventResult::Propagate,
                    };
                    Some(Event::Pointer(input_events::PointerEvent {
                        kind: 2,
                        button,
                        point: (
                            cursor.0 / sf - w.get_scene_x(),
                            cursor.1 / sf - w.get_scene_y(),
                        ),
                        modifiers,
                        scale: sf,
                        viewport,
                        suppress: modal,
                    }))
                }
                WindowEvent::KeyboardInput {
                    event,
                    is_synthetic: false,
                    ..
                } if !modal => {
                    if event.logical_key == Key::Named(NamedKey::Escape) {
                        sender.send(events::GuiEvent::Input(Event::Focus(false)));
                    }
                    if let PhysicalKey::Code(code) = event.physical_key {
                        if let Some(key) = map_winit_key(code) {
                            let text = normalized_slint_shortcut_text(
                                key,
                                modifiers.shift,
                                modifiers.ctrl,
                                modifiers.alt,
                                modifiers.logo,
                            )
                            .map(slint::SharedString::from)
                            .or_else(|| match &event.logical_key {
                                Key::Character(text) => Some(text.as_str().into()),
                                Key::Named(NamedKey::Space) => Some(" ".into()),
                                Key::Named(named) => native_key_text(*named),
                                _ => event.text.as_ref().map(|text| text.as_str().into()),
                            })
                            .filter(|text| !text.is_empty())
                            .or_else(|| {
                                event
                                    .text
                                    .as_ref()
                                    .filter(|text| !text.is_empty())
                                    .map(|text| text.as_str().into())
                            });
                            if let Some(text) = text {
                                sender.send(events::GuiEvent::Input(Event::Key {
                                    modifiers,
                                    binding: KeyBinding {
                                        key,
                                        ctrl: modifiers.ctrl || modifiers.logo,
                                        shift: modifiers.shift,
                                        alt: modifiers.alt,
                                    },
                                    pressed: event.state == ElementState::Pressed,
                                    repeat: event.repeat,
                                    text,
                                }));
                                return EventResult::PreventDefault;
                            }
                        }
                    }
                    None
                }
                _ => None,
            };
            if let Some(input) = input {
                sender.send(events::GuiEvent::Input(input));
            }
            EventResult::Propagate
        });
}

fn native_key_text(
    key: slint::winit_030::winit::keyboard::NamedKey,
) -> Option<slint::SharedString> {
    use slint::platform::Key as K;
    use slint::winit_030::winit::keyboard::NamedKey as N;
    Some(
        match key {
            N::Enter => K::Return,
            N::Tab => K::Tab,
            N::Backspace => K::Backspace,
            N::Delete => K::Delete,
            N::Escape => K::Escape,
            N::ArrowLeft => K::LeftArrow,
            N::ArrowRight => K::RightArrow,
            N::ArrowUp => K::UpArrow,
            N::ArrowDown => K::DownArrow,
            N::Home => K::Home,
            N::End => K::End,
            N::PageUp => K::PageUp,
            N::F1 => K::F1,
            N::F2 => K::F2,
            N::F3 => K::F3,
            N::F4 => K::F4,
            N::F5 => K::F5,
            N::F6 => K::F6,
            N::F7 => K::F7,
            N::F8 => K::F8,
            N::F9 => K::F9,
            N::F10 => K::F10,
            N::F11 => K::F11,
            N::F12 => K::F12,

            N::PageDown => K::PageDown,
            _ => return None,
        }
        .into(),
    )
}
