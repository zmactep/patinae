use super::*;
use crate::{ReplState, SequenceState};

fn fixture() -> (Rc<RefCell<App>>, AppWindow, EventSender) {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap();
    let app = Rc::new(RefCell::new(App::new()));
    let sender = app.borrow().events.clone();
    sender.bind(&app, &window);
    bridges::repl::setup_callbacks(sender.clone(), &window);
    (app, window, sender)
}

#[test]
fn representation_popover_reads_the_requested_kind_on_first_open_and_switch() {
    let (app, window, sender) = fixture();
    let molecule = patinae_mol::MoleculeBuilder::new("mol")
        .add_atom(
            patinae_mol::AtomBuilder::new()
                .name("CA")
                .element_symbol("C")
                .resn("ALA")
                .resv(1)
                .chain("A")
                .flags(patinae_mol::AtomFlags::PROTEIN | patinae_mol::AtomFlags::POLYMER)
                .build(),
            lin_alg::f32::Vec3::new(0.0, 0.0, 0.0),
        )
        .build();
    {
        let mut host = app.borrow_mut();
        host.kernel
            .session
            .registry
            .add(patinae_scene::MoleculeObject::with_name(
                molecule.clone(),
                "mol",
            ));
        host.kernel
            .session
            .registry
            .add(patinae_scene::MoleculeObject::with_name(molecule, "other"));
        for command in [
            "hide everything, all",
            "show cartoon, all",
            "select named, mol",
        ] {
            host.kernel
                .execute_command(command, false, None, (800, 600))
                .unwrap();
        }
    }
    bridges::objects::setup_callbacks(sender.clone(), &window);
    let objects = window.global::<crate::ObjectsState>();
    for (source, target, entry, clause) in [
        ("object", "mol", -1, ""),
        ("object", "mol", 0, "chain A"),
        ("selection", "named", -1, ""),
    ] {
        for previous in ["", "C"] {
            objects.set_popover_kind(previous.into());
            objects.set_popover_rep_cartoon(false);
            objects.set_popover_rep_sticks(true);
            objects.invoke_row_btn_clicked(
                "R".into(),
                source.into(),
                target.into(),
                "A".into(),
                "".into(),
                "biopolymer".into(),
                entry,
                clause.into(),
            );
            sender.drain();
            assert_eq!(objects.get_popover_kind(), "R");
            assert!(
                objects.get_popover_rep_cartoon(),
                "{source}/{entry} from {previous}"
            );
            assert!(!objects.get_popover_rep_sticks());
        }
    }
    // The next row may already be queued while an action still belongs to the
    // old visible popover. Execute that action without projecting it onto other.
    objects.set_popover_cmd_preview("hide cartoon, named".into());
    objects.invoke_row_btn_clicked(
        "R".into(),
        "object".into(),
        "other".into(),
        "".into(),
        "".into(),
        "".into(),
        -1,
        "".into(),
    );
    objects.invoke_popover_execute();
    sender.drain();
    assert_eq!(objects.get_popover_target(), "other");
    assert!(objects.get_popover_rep_cartoon());
    assert!(!app
        .borrow()
        .kernel
        .session
        .registry
        .get_molecule("mol")
        .unwrap()
        .effective_reps()
        .is_visible(patinae_mol::RepMask::CARTOON));
}

#[test]
fn fifo_is_bounded_and_commands_work_without_frames() {
    let (app, window, sender) = fixture();
    for index in 1..=300 {
        window
            .global::<ReplState>()
            .invoke_submit(format!("set stick_radius, {}", index as f32 / 100.0).into());
    }
    assert!(app.borrow().kernel.command_line.history.is_empty());
    sender.drain();
    assert_eq!(
        app.borrow().kernel.command_line.history.len(),
        EVENTS_PER_TURN
    );
    assert_eq!(sender.0.pending.borrow().len(), 44);
    sender.drain();
    let app = app.borrow();
    assert_eq!(app.kernel.command_line.history.len(), 300);
    for (index, command) in app.kernel.command_line.history.iter().enumerate() {
        assert_eq!(
            command,
            &format!("set stick_radius, {}", (index + 1) as f32 / 100.0)
        );
    }
}

#[test]
fn busy_and_reentrant_dispatch_preserve_pending_input() {
    let (app, window, sender) = fixture();
    window
        .global::<ReplState>()
        .invoke_submit("set stick_radius, 0.2".into());
    let borrow = app.borrow_mut();
    sender.drain();
    assert_eq!(sender.0.pending.borrow().len(), 1);
    drop(borrow);
    sender.0.draining.set(true);
    sender.drain();
    assert_eq!(sender.0.pending.borrow().len(), 1);
    sender.0.draining.set(false);
    sender.drain();
    assert_eq!(app.borrow().kernel.command_line.history.len(), 1);
}

#[test]
fn close_cancels_pending_work_and_future_sends() {
    let (app, window, sender) = fixture();
    sender.send(GuiEvent::Close);
    window
        .global::<ReplState>()
        .invoke_submit("set stick_radius, 0.2".into());
    sender.drain();
    window
        .global::<ReplState>()
        .invoke_submit("set stick_radius, 0.3".into());
    sender.drain();
    assert!(sender.0.closed.get());
    assert!(sender.0.pending.borrow().is_empty());
    assert!(app.borrow().kernel.command_line.history.is_empty());
}

#[test]
fn effect_callbacks_are_deferred_until_the_next_turn() {
    let (app, window, sender) = fixture();
    let weak = Rc::downgrade(&app);
    let next = sender.clone();
    window
        .global::<ReplState>()
        .on_request_keyboard_modifier_reset(move || {
            // This effect may synchronously call back into the application.
            let host = weak.upgrade().unwrap();
            let _borrow = host.borrow_mut();
            next.send(GuiEvent::Repl(bridges::repl::events::Event::Submit(
                "set stick_radius, 0.4".into(),
            )));
            next.drain();
        });
    window.global::<ReplState>().invoke_request_focus();
    sender.drain();
    assert!(app.borrow().kernel.command_line.history.is_empty());
    assert_eq!(sender.0.pending.borrow().len(), 1);
    sender.drain();
    assert_eq!(app.borrow().kernel.command_line.history.len(), 1);
}

#[test]
fn event_loop_delivers_input_and_allows_nested_timer_activation_without_frames() {
    // Slint's event-loop proxy is process-global. Other unit tests deliberately
    // exercise quit, so keep the real-loop regression in an isolated process.
    const CHILD: &str = "PATINAE_QUEUE_EVENT_LOOP_TEST";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "app::events::tests::event_loop_delivers_input_and_allows_nested_timer_activation_without_frames",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    i_slint_backend_testing::init_integration_test_with_mock_time();
    let window = AppWindow::new().unwrap();
    let app = Rc::new(RefCell::new(App::new()));
    let sender = app.borrow().events.clone();
    sender.bind(&app, &window);
    bridges::repl::setup_callbacks(sender, &window);
    let nested = Rc::new(Cell::new(false));
    let observed = nested.clone();
    window
        .global::<ReplState>()
        .on_request_keyboard_modifier_reset(move || {
            // Like an AppKit modal loop, process due timers inside a native effect.
            slint::Timer::single_shot(std::time::Duration::ZERO, || {});
            slint::platform::update_timers_and_animations();
            observed.set(true);
            slint::quit_event_loop().unwrap();
        });
    window
        .global::<ReplState>()
        .invoke_submit("set stick_radius, 0.2".into());
    window.global::<ReplState>().invoke_request_focus();
    slint::run_event_loop().unwrap();
    assert!(nested.get());
    assert_eq!(app.borrow().kernel.command_line.history.len(), 1);
}

#[test]
fn stale_sequence_commit_is_cancelled_without_leaving_preview() {
    let (app, window, sender) = fixture();
    let sequence = window.global::<SequenceState>();
    sequence.set_commit_pending(true);
    sender.send(GuiEvent::Sequence(bridges::sequence::events::Event::Cancel));
    sender.send(GuiEvent::Sequence(
        bridges::sequence::events::Event::Commit {
            id: 1,
            row: 0,
            start: 0,
            end: 10,
            exclude: false,
            generation: 0,
        },
    ));
    sender.drain();
    assert!(!sequence.get_commit_pending());
    assert!(!app.borrow().kernel.session.selections.contains("sele"));
}

#[test]
fn completion_uses_the_item_and_input_captured_by_the_callback() {
    let (_app, window, sender) = fixture();
    let repl = window.global::<ReplState>();
    repl.set_input_text("he".into());
    repl.set_completion_start_pos(0);
    repl.set_completion_items(slint::ModelRc::new(slint::VecModel::from(vec![
        crate::CompletionItem {
            text: "help".into(),
            ..Default::default()
        },
    ])));
    repl.invoke_completion_accept(0);
    repl.set_input_text("other".into());
    repl.set_completion_items(slint::ModelRc::default());
    sender.drain();
    assert_eq!(repl.get_input_text(), "help ");
}

#[test]
fn consecutive_movie_toggles_see_the_previous_command() {
    let (app, window, sender) = fixture();
    app.borrow_mut()
        .kernel
        .execute_command("mset 1 x10", false, None, (800, 600))
        .unwrap();
    bridges::movie::setup_callbacks(sender.clone(), &window);
    let movie = window.global::<crate::MovieState>();
    movie.invoke_toggle_play();
    movie.invoke_toggle_play();
    sender.drain();
    assert!(!app.borrow().kernel.session.movie.is_playing());
    movie.invoke_toggle_play();
    sender.drain();
    assert!(app.borrow().kernel.session.movie.is_playing());
}

#[test]
fn repl_focus_reset_clears_native_capture_metadata_too() {
    let (_app, window, sender) = fixture();
    sender.send(GuiEvent::Input(
        super::super::input_events::Event::Modifiers(patinae_scene::Modifiers::SUPER),
    ));
    window
        .global::<ReplState>()
        .invoke_request_keyboard_modifier_reset();
    sender.drain();
    let flags = sender.captured_modifiers().unwrap();
    assert!(!flags.shift && !flags.ctrl && !flags.alt && !flags.logo);
    sender.send(GuiEvent::Input(
        super::super::input_events::Event::Modifiers(patinae_scene::Modifiers::SUPER),
    ));
    sender.send(GuiEvent::Input(super::super::input_events::Event::Focus(
        false,
    )));
    // Capture for a following native event must be reset before dispatch runs.
    assert!(!sender.captured_modifiers().unwrap().logo);
}
