use super::*;
use patinae_framework::model::sequence::SeqResidue;

/// CPU-only scroll benchmark; excludes parsing, model construction and GPU upload.
/// Run explicitly with `cargo test -p patinae --profile dev-fast
/// measure_sequence_scroll -- --ignored --nocapture`.
#[test]
#[ignore = "manual performance measurement using bundled structure fixtures"]
fn measure_sequence_scroll() {
    slint::platform::set_platform(Box::new(OffscreenPlatform)).unwrap();
    let window = AppWindow::new().unwrap();
    let state = window.global::<SequenceState>();
    let metrics = window.global::<ComponentMetrics>().get_sequence();
    state.set_viewport_height(metrics.height * 5.0);
    for name in ["1fsd", "3J3Q"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../web/assets/{name}.cif"));
        let molecule = patinae_io::read_file(&path).unwrap();
        let mut kernel = AppKernel::new();
        kernel
            .session
            .registry
            .add(patinae_scene::MoleculeObject::with_name(molecule, name));
        let mut bridge = SequenceBridge::new();
        bridge.metrics = metrics.clone();
        bridge.atlas = GlyphAtlas::new(2.0, &metrics);
        bridge.model.rebuild_cache(
            &kernel.session.registry,
            &SequenceColorContext {
                named_palette: &kernel.session.named_palette,
                palette: &kernel.session.palette,
            },
        );
        state.set_viewport_y(0.0);
        bridge.push_to_slint(&window, &kernel.session.palette, 2.0, true);
        let mut samples = Vec::new();
        let mut changed_ranges = 0;
        let mut max_painted = 0;
        for step in 0..110 {
            let row = (step * 5) % bridge.slots.len();
            state.set_viewport_y(-bridge.slots[row].top);
            let old_range = bridge.rendered_rows.clone();
            let started = std::time::Instant::now();
            bridge.refresh_visible_rows(&window, 2.0, true);
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            if step >= 10 {
                samples.push(elapsed);
                changed_ranges += usize::from(old_range != bridge.rendered_rows);
            }
            let painted = state
                .get_chains()
                .iter()
                .filter(|row| row.strip_image.size().width > 0)
                .count();
            max_painted = max_painted.max(painted);
            assert!(painted <= 9, "invisible row images must be released");
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "sequence-benchmark {name}: chains={} samples={} changed_ranges={changed_ranges} p50_ms={:.3} p95_ms={:.3} max_painted={max_painted}",
            bridge.slots.len(), samples.len(), samples[50], samples[95]
        );
    }
}

#[test]
fn failed_old_commit_does_not_clear_a_newer_preview() {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().unwrap();
    let mut app = crate::app::App::new();
    app.sequence.sync(&app.kernel, &window);
    app.sequence.pending_commits.insert(1);
    app.sequence.gesture = events::Gesture::CommitPending { id: 2 };
    let generation = app.sequence.generation;
    window.global::<SequenceState>().set_commit_pending(true);
    events::dispatch(
        events::Event::Commit {
            id: 1,
            row: 999,
            start: 0,
            end: 10,
            exclude: false,
            generation,
        },
        &mut app,
        &window,
    );
    assert!(window.global::<SequenceState>().get_commit_pending());
    assert!(matches!(
        app.sequence.gesture,
        events::Gesture::CommitPending { id: 2 }
    ));
}

struct OffscreenPlatform;

impl slint::platform::Platform for OffscreenPlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(
            slint::platform::software_renderer::MinimalSoftwareWindow::new(
                slint::platform::software_renderer::RepaintBufferType::NewBuffer,
            ),
        )
    }
}

#[test]
fn sequence_images_follow_viewport_and_preserve_scrolled_selection() {
    slint::platform::set_platform(Box::new(OffscreenPlatform)).unwrap();
    let window = AppWindow::new().unwrap();
    let state = window.global::<SequenceState>();
    let mut bridge = SequenceBridge::new();
    bridge.metrics = window.global::<ComponentMetrics>().get_sequence();
    bridge.atlas = GlyphAtlas::new(1.0, &bridge.metrics);
    bridge
        .model
        .sequences
        .push(patinae_framework::model::sequence::SeqObject {
            object_name: "test".into(),
            chains: (0..24)
                .map(|i| SeqChain {
                    chain_id: i.to_string(),
                    residues: vec![test_residue(1), test_residue(2)],
                })
                .collect(),
        });
    state.set_viewport_height(bridge.metrics.height * 2.0);
    bridge.push_to_slint(&window, &ThemedPalette::dark(), 1.0, true);
    let rows = state.get_chains();
    let painted = || {
        rows.iter()
            .filter(|row| row.strip_image.size().width > 0)
            .count()
    };
    assert!((1..=5).contains(&painted()));
    assert_eq!(rows.row_data(12).unwrap().strip_image.size().width, 0);
    assert_eq!(rows.row_data(0).unwrap().highlight_image.size().width, 0);

    bridge.model.highlighted.insert(ResidueRef {
        object_name: "test".into(),
        chain_id: "12".into(),
        resv: 1,
    });
    state.set_viewport_y(-bridge.slots[12].top);
    bridge.refresh_visible_rows(&window, 1.0, true);
    assert!((1..=5).contains(&painted()));
    assert_eq!(rows.row_data(0).unwrap().strip_image.size().width, 0);
    assert!(rows.row_data(12).unwrap().strip_image.size().width > 0);
    let highlight = rows
        .row_data(12)
        .unwrap()
        .highlight_image
        .to_rgba8()
        .unwrap();
    let y = (bridge.metrics.ruler + 1.0) as usize;
    let first_x = (bridge.metrics.offset + bridge.metrics.cell * 0.5) as usize;
    let second_x = (bridge.metrics.offset + bridge.metrics.cell * 1.5) as usize;
    let alpha = |x| highlight.as_bytes()[(y * highlight.width() as usize + x) * 4 + 3];
    assert_eq!(alpha(first_x), 77);
    assert_eq!(alpha(second_x), 0);
    assert_eq!(bridge.hit_residue(12, 0.0).unwrap().0, 0);

    state.set_viewport_height(0.0);
    bridge.refresh_visible_rows(&window, 1.0, true);
    assert_eq!(painted(), 0);
}

fn test_residue(resv: i32) -> SeqResidue {
    SeqResidue {
        chain: String::new(),
        resn: "THR".to_string(),
        resv,
        display_char: 'T',
        display_label: "T".to_string(),
        char_width: 1,
        atom_range: 0..1,
        named_color: [0, 0, 0],
        viewer_color: [0, 0, 0],
        kind: ResidueKind::AminoAcidCanonical,
    }
}

fn test_slot(chain_id: &str, residues: Vec<SeqResidue>) -> ChainSlot {
    ChainSlot {
        object_name: "test".to_string(),
        chain: SeqChain {
            chain_id: chain_id.to_string(),
            residues,
        },
        cell_widths: Vec::new(),
        cell_prefix: Vec::new(),
        strip_w_phys: 0,
        strip_h_phys: 0,
        chain_rgb: [0, 0, 0],
        top: 0.0,
    }
}

#[test]
fn viewer_color_toggle_and_invalidate_mark_cache_dirty() {
    let mut bridge = SequenceBridge::new();
    bridge.cache_dirty = false;

    bridge.toggle_viewer_colors();
    assert!(bridge.viewer_colors);
    assert!(bridge.cache_dirty);

    bridge.cache_dirty = false;
    bridge.invalidate();
    assert!(bridge.cache_dirty);
}

#[test]
fn hover_rejects_outside_cells_while_drag_clamps() {
    let mut bridge = SequenceBridge::new();
    let mut slot = test_slot("A", vec![test_residue(1), test_residue(2)]);
    slot.cell_widths = vec![11.0, 55.0];
    slot.cell_prefix = vec![0.0, 11.0, 66.0];
    bridge.slots.push(slot);
    assert_eq!(bridge.hit_residue(0, 0.0), Some((0, 0.0, 11.0)));
    assert_eq!(bridge.hit_residue(0, 11.0), Some((1, 11.0, 66.0)));
    assert_eq!(bridge.hit_residue(0, 65.9), Some((1, 11.0, 66.0)));
    for x in [-1.0, 66.0, f32::NAN, f32::INFINITY] {
        assert_eq!(bridge.hit_residue(0, x), None);
    }
    assert_eq!(bridge.hit_residue(-1, 0.0), None);
    assert_eq!(bridge.hit_residue(1, 0.0), None);
    assert_eq!(bridge.x_to_residue(0, -1.0), (0, true));
    assert_eq!(bridge.x_to_residue(0, 66.0), (1, true));
}

#[test]
fn hit_testing_uses_actual_variable_width_cells() {
    let mut bridge = SequenceBridge::new();
    let mut slot = test_slot("A", vec![test_residue(1), test_residue(2), test_residue(3)]);
    // Mixed polymer and bracketed ligand labels have different widths.
    slot.cell_prefix = vec![0.0, 11.0, 66.0, 77.0];
    slot.cell_widths = vec![11.0, 55.0, 11.0];
    bridge.slots.push(slot);

    assert_eq!(bridge.x_to_residue(0, -5.0), (0, true));
    assert_eq!(bridge.x_to_residue(0, 10.9), (0, true));
    assert_eq!(bridge.x_to_residue(0, 11.0), (1, true));
    assert_eq!(bridge.x_to_residue(0, 65.9), (1, true));
    assert_eq!(bridge.x_to_residue(0, 66.0), (2, true));
    assert_eq!(bridge.x_to_residue(0, 100.0), (2, true));
    assert_eq!(bridge.x_to_residue(1, 0.0), (0, false));
}

#[test]
fn build_drag_command_quotes_blank_chain() {
    let mut bridge = SequenceBridge::new();
    bridge
        .slots
        .push(test_slot("", vec![test_residue(4), test_residue(5)]));

    let cmd = bridge.build_drag_command(0, 0, 1, false, false).unwrap();
    assert_eq!(cmd, "select sele, model test and chain \"\" and resi 4-5");
}
