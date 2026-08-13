//! Session — pure scene state
//!
//! [`Session`] holds all domain/scene data that can exist independently of
//! rendering: object registry, camera, selections, settings, colors, etc.
//!
//! This is the single source of truth that GUI and headless adapters can own.
//! GPU resources live behind a host-provided render target.

use patinae_color::{NamedPalette, ThemedPalette};
use patinae_select::{
    evaluate, parse, EvalContext, MacroSpec, Pattern, ResiItem, SelectionExpr, SelectionOptions,
    SelectionResult,
};
use patinae_settings::Settings;
use serde::{Deserialize, Serialize};

use crate::camera::Camera;
use crate::error::SceneResult;
use crate::highlight_state::HighlightState;
use crate::movie::Movie;
use crate::object::{DirtyFlags, Object, ObjectRegistry, ObjectRegistrySnapshot};
use crate::recent_atoms::RecentAtoms;
use crate::scene::SceneManager;
use crate::selection::SelectionManager;
use crate::view::ViewManager;
use crate::viewer_trait::ViewportImage;

/// Pure scene state — no GPU resources, no window, no event loop.
///
/// Owns all molecular objects, camera state, named selections, scenes,
/// views, animation, settings, and color tables.
///
/// Implements `Serialize` and `Deserialize` via a proxy that converts
/// the [`ObjectRegistry`] to/from an [`ObjectRegistrySnapshot`].
pub struct Session {
    // =========================================================================
    // Scene
    // =========================================================================
    /// Object registry (molecules, surfaces, maps, CGO, etc.)
    pub registry: ObjectRegistry,
    /// Camera for view control
    pub camera: Camera,
    /// Named selections manager
    pub selections: SelectionManager,
    /// Scene manager for named snapshots (camera + object state)
    pub scenes: SceneManager,
    /// Named views (camera state only — simpler than scenes)
    pub views: ViewManager,
    /// Movie player for frame-based animation
    pub movie: Movie,

    // =========================================================================
    // Settings and Colors
    // =========================================================================
    /// Global rendering settings
    pub settings: Settings,
    /// Named colors table (e.g., "red", "carbon")
    pub named_palette: NamedPalette,
    /// Theme-aware palette (element, chain, SS, residue, gradients, etc.)
    pub palette: ThemedPalette,
    /// Ordered canonical paths for atoms collected from the viewport.
    pub recent_atoms: RecentAtoms,

    // =========================================================================
    // Visual Properties
    // =========================================================================
    /// Background (clear) color as linear RGB floats
    pub clear_color: [f32; 3],
    /// Whether clear_color has been explicitly set by the user
    pub clear_color_set: bool,

    // =========================================================================
    // Viewport Image Overlay
    // =========================================================================
    /// Image overlay for display in the viewport (e.g. from `ray` command or plugins)
    pub viewport_image: Option<ViewportImage>,

    // =========================================================================
    // Highlight state (transient — not serialized)
    // =========================================================================
    /// GPU selection / hover bitmask state for the screen-space highlight pass.
    /// Rebuilt every frame in `prepare_scene` from `selections.evaluate_visible`
    /// and `hover_target`.
    pub highlight_state: HighlightState,
    /// Active hover target. `None` when nothing is hovered.
    pub hover_target: Option<HoverTarget>,
}

/// Atoms currently under the cursor, fed into the screen-space highlight pass.
///
/// The bridge layer (patinae / web) sets this on cursor-move; the highlight
/// state reads `(object, selection)` to set hover bits in its bitmap.
#[derive(Debug, Clone)]
pub struct HoverTarget {
    /// Object whose coords resolve `selection`'s indices.
    pub object: String,
    /// Atom indices to mark.
    pub selection: SelectionResult,
}

/// Result of advancing session-owned animations for one host frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnimationUpdate {
    /// Movie playback advanced to a different frame.
    pub movie_frame_changed: bool,
    /// The current movie frame was applied to scene objects/camera.
    pub movie_synced: bool,
    /// Rock animation changed the camera.
    pub rock_changed: bool,
    /// Camera interpolation changed the camera.
    pub camera_changed: bool,
    /// Any visible state changed and the host should render.
    pub needs_redraw: bool,
}

/// Lightweight movie state for host UI/API layers.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct MovieStateSnapshot {
    /// Effective frame count: explicit movie frames or max object states.
    pub frame_count: usize,
    /// Current 0-based movie frame.
    pub current_frame: usize,
    /// Whether playback is active.
    pub is_playing: bool,
    /// Whether rock animation is active.
    pub rock_enabled: bool,
}

/// Serializable proxy for [`Session`] (for deserialization).
#[derive(Deserialize)]
struct SessionProxy {
    registry: ObjectRegistrySnapshot,
    camera: Camera,
    selections: SelectionManager,
    scenes: SceneManager,
    views: ViewManager,
    movie: Movie,
    settings: Settings,
    #[serde(alias = "named_colors")]
    named_palette: NamedPalette,
    #[serde(alias = "element_palette", alias = "element_colors")]
    palette: ThemedPalette,
    clear_color: [f32; 3],
    #[serde(default)]
    clear_color_set: bool,
    #[serde(default)]
    recent_atoms: RecentAtoms,
}

impl Serialize for Session {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("Session", 12)?;
        s.serialize_field("registry", &self.registry.to_snapshot())?;
        s.serialize_field("camera", &self.camera)?;
        s.serialize_field("selections", &self.selections)?;
        s.serialize_field("scenes", &self.scenes)?;
        s.serialize_field("views", &self.views)?;
        s.serialize_field("movie", &self.movie)?;
        s.serialize_field("settings", &self.settings)?;
        s.serialize_field("named_palette", &self.named_palette)?;
        s.serialize_field("palette", &self.palette)?;
        s.serialize_field("clear_color", &self.clear_color)?;
        s.serialize_field("clear_color_set", &self.clear_color_set)?;
        s.serialize_field("recent_atoms", &self.recent_atoms)?;
        s.end()
    }
}

impl<'de> Deserialize<'de> for Session {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let proxy = SessionProxy::deserialize(deserializer)?;
        let mut recent_atoms = proxy.recent_atoms;
        recent_atoms.enforce_limit(proxy.settings.behavior.recent_pick_limit());
        let mut session = Session {
            registry: ObjectRegistry::from_snapshot(proxy.registry),
            camera: proxy.camera,
            selections: proxy.selections,
            scenes: proxy.scenes,
            views: proxy.views,
            movie: proxy.movie,
            settings: proxy.settings,
            named_palette: proxy.named_palette,
            palette: proxy.palette,
            recent_atoms,
            clear_color: proxy.clear_color,
            clear_color_set: proxy.clear_color_set,
            viewport_image: None,
            highlight_state: HighlightState::new(),
            hover_target: None,
        };
        session.reconcile_recent_atoms();
        Ok(session)
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// Create a new session with default values.
    pub fn new() -> Self {
        Self {
            registry: ObjectRegistry::new(),
            camera: Camera::new(),
            selections: SelectionManager::new(),
            scenes: SceneManager::new(),
            views: ViewManager::new(),
            movie: Movie::new(),
            settings: Settings::default(),
            named_palette: NamedPalette::default(),
            palette: ThemedPalette::dark(),
            recent_atoms: RecentAtoms::new(),
            clear_color: [0.0, 0.0, 0.0],
            clear_color_set: false,
            viewport_image: None,
            highlight_state: HighlightState::new(),
            hover_target: None,
        }
    }

    /// Reconciles durable recent paths against the current molecular registry.
    pub fn reconcile_recent_atoms(&mut self) -> bool {
        self.reconcile_recent_atoms_with_model_rename(None)
    }

    /// Inserts an object and reconciles recent atom paths.
    pub fn insert_object(&mut self, object: Box<dyn Object>) {
        self.insert_objects(std::iter::once(object));
    }

    /// Inserts objects sequentially and reconciles recent paths once.
    pub(crate) fn insert_objects(&mut self, objects: impl IntoIterator<Item = Box<dyn Object>>) {
        let mut inserted = false;
        for object in objects {
            let name = object.name().to_string();
            self.registry.insert_boxed(&name, object);
            inserted = true;
        }
        if inserted {
            self.reconcile_recent_atoms();
        }
    }

    /// Returns whether a canonical recent-atom path resolves to exactly one atom.
    pub fn recent_atom_path_is_singleton(&self, path: &str) -> bool {
        self.resolve_recent_atom(path).is_some()
    }

    /// Resolves every valid recent path to an object-local atom index.
    pub fn resolved_recent_atoms(&self) -> Vec<(String, patinae_mol::AtomIndex)> {
        let contexts =
            recent_atom_evaluation_contexts(&self.registry, &self.selections, &self.settings);
        self.recent_atoms
            .paths()
            .filter_map(|path| resolve_recent_atom_in_contexts(path, &contexts))
            .collect()
    }

    fn resolve_recent_atom(&self, path: &str) -> Option<(String, patinae_mol::AtomIndex)> {
        let contexts =
            recent_atom_evaluation_contexts(&self.registry, &self.selections, &self.settings);
        resolve_recent_atom_in_contexts(path, &contexts)
    }

    fn reconcile_recent_atoms_with_model_rename(&mut self, rename: Option<(&str, &str)>) -> bool {
        if self.recent_atoms.is_empty() {
            return false;
        }

        let contexts =
            recent_atom_evaluation_contexts(&self.registry, &self.selections, &self.settings);

        let mut changed = self.recent_atoms.reconcile_paths(|path| {
            let SelectionExpr::Macro(mut spec) = parse(path).ok()? else {
                return None;
            };
            if let Some((old_name, new_name)) = rename {
                if exact_pattern(&spec.model) == Some(old_name) {
                    spec.model = Some(Pattern::Exact(new_name.to_string()));
                }
            }
            if !is_exact_atom_macro(&spec) {
                return None;
            }
            let canonical_path = format_exact_atom_macro(&spec)?;
            let expression = SelectionExpr::Macro(spec);
            exact_singleton_in_contexts(&expression, &contexts).then_some(canonical_path)
        });
        changed |= self
            .recent_atoms
            .enforce_limit(self.settings.behavior.recent_pick_limit());
        changed
    }

    /// Renames an object and rewrites matching recent path model components.
    ///
    /// # Errors
    ///
    /// Returns an error when the registry rejects the rename.
    pub fn rename_object(&mut self, old_name: &str, new_name: &str) -> SceneResult<()> {
        self.registry.rename(old_name, new_name)?;
        self.reconcile_recent_atoms_with_model_rename(Some((old_name, new_name)));
        Ok(())
    }

    /// Removes an object and reconciles recent atom paths.
    pub fn remove_object(&mut self, name: &str) -> bool {
        if self.registry.remove(name).is_none() {
            return false;
        }
        self.reconcile_recent_atoms();
        true
    }

    /// Removes molecule atoms and reconciles recent atom paths.
    ///
    /// # Errors
    ///
    /// Returns an error when `source_name` is not a molecule object.
    pub fn remove_molecule_atoms(
        &mut self,
        source_name: &str,
        indices: &[patinae_mol::AtomIndex],
    ) -> SceneResult<usize> {
        let removed = self.registry.remove_molecule_atoms(source_name, indices)?;
        self.reconcile_recent_atoms();
        Ok(removed)
    }

    /// Clears all objects and reconciles recent atom paths.
    pub fn clear_objects(&mut self) {
        self.registry.clear();
        self.reconcile_recent_atoms();
    }

    /// Applies immediate recent-atom effects for one changed global setting.
    pub fn reconcile_recent_atom_setting(&mut self, setting_name: &str) {
        match setting_name {
            "max_recent_picks" => {
                self.recent_atoms
                    .enforce_limit(self.settings.behavior.recent_pick_limit());
            }
            "ignore_case" | "ignore_case_chain" => {
                self.reconcile_recent_atoms();
            }
            _ => {}
        };
    }

    /// Replaces all session state and invalidates transient observer tokens.
    pub fn replace_contents(&mut self, mut replacement: Session) {
        let old_registry_generation = self.registry.generation();
        let old_selection_generation = self.selections.generation();
        let old_recent_generation = self.recent_atoms.generation();
        let old_recent_incarnation = self.recent_atoms.incarnation();
        replacement.reconcile_recent_atoms();

        *self = replacement;
        self.recent_atoms
            .mark_replaced_after(old_recent_incarnation);
        self.registry.mark_all_dirty();
        if self.registry.generation() == old_registry_generation {
            self.registry.invalidate();
        }
        if self.selections.generation() == old_selection_generation {
            self.selections.invalidate();
        }
        if self.recent_atoms.generation() == old_recent_generation {
            self.recent_atoms.invalidate();
        }
    }

    /// Set the active hover target. Renders next frame.
    pub fn set_hover(&mut self, target: HoverTarget) {
        self.hover_target = Some(target);
    }

    /// Clear the active hover target.
    pub fn clear_hover(&mut self) {
        self.hover_target = None;
    }

    /// Recompute the effective trajectory/movie frame count from loaded objects.
    pub fn refresh_movie_state_count(&mut self) {
        let max_states = self
            .registry
            .iter()
            .map(|obj| obj.n_states())
            .max()
            .unwrap_or(1);
        self.movie.set_n_object_states(max_states);
    }

    /// Apply the current movie frame to scenes, object states, transforms, and camera.
    ///
    /// Frame state is applied after scene recall, and movie camera view is applied
    /// last so explicit movie keyframes win over scene camera snapshots.
    pub fn sync_movie_frame(&mut self) -> bool {
        let current_frame = self.movie.current_frame();
        let state_index = self.movie.frame_to_state(current_frame);
        let scene_name = self.movie.current_scene_name().map(ToOwned::to_owned);
        let view = self.movie.interpolated_view();
        let (object_states, object_keyframe_states) = self
            .movie
            .frame()
            .map(|frame| {
                let object_states = frame
                    .object_states
                    .iter()
                    .map(|(name, state)| (name.clone(), *state))
                    .collect::<Vec<_>>();
                let object_keyframe_states = frame
                    .object_keyframes
                    .iter()
                    .filter_map(|(name, keyframe)| {
                        keyframe.state.map(|state| (name.clone(), state))
                    })
                    .collect::<Vec<_>>();
                (object_states, object_keyframe_states)
            })
            .unwrap_or_default();

        let mut changed = false;

        if let Some(scene_name) = scene_name {
            if let Some(scene) = self.scenes.get(&scene_name) {
                scene.apply(&mut self.camera, &mut self.registry, false, 0.0);
                changed = true;
            }
        }

        let names: Vec<String> = self.registry.names().map(ToOwned::to_owned).collect();
        for name in &names {
            if let Some(obj) = self.registry.get_molecule_mut(name) {
                let before = obj.display_state();
                if obj.set_display_state(state_index) && obj.display_state() != before {
                    changed = true;
                }
            }
        }

        for (name, state) in object_states.into_iter().chain(object_keyframe_states) {
            if let Some(obj) = self.registry.get_mut(&name) {
                let before = obj.current_state();
                let state = state.saturating_sub(1);
                if obj.set_current_state(state) && obj.current_state() != before {
                    changed = true;
                }
            }
        }

        changed |= self.apply_movie_object_transforms();

        if let Some(view) = view {
            self.camera.set_view(view);
            changed = true;
        }

        changed
    }

    /// Advance movie playback, rock animation, and camera interpolation.
    pub fn update_animations(&mut self, dt: f32) -> AnimationUpdate {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 0.25)
        } else {
            0.0
        };

        self.movie.set_fps(self.settings.movie.movie_fps);

        let was_playing = self.movie.is_playing();
        let movie_frame_changed = self.movie.update(dt);
        let movie_synced = if movie_frame_changed {
            self.sync_movie_frame()
        } else {
            false
        };
        let playback_state_changed = was_playing != self.movie.is_playing();

        let rock_delta = if self.movie.is_rock_enabled() {
            let amplitude = 45.0_f32.to_radians();
            let speed = 5.0;
            self.movie.update_rock(dt, amplitude, speed)
        } else {
            0.0
        };
        let rock_changed = rock_delta != 0.0;
        if rock_changed {
            self.camera.rotate_y(rock_delta);
        }

        let camera_changed = self.camera.update(dt);

        AnimationUpdate {
            movie_frame_changed,
            movie_synced,
            rock_changed,
            camera_changed,
            needs_redraw: movie_frame_changed
                || movie_synced
                || playback_state_changed
                || rock_changed
                || camera_changed,
        }
    }

    /// Get a host-facing snapshot of the current movie state.
    pub fn movie_state_snapshot(&self) -> MovieStateSnapshot {
        MovieStateSnapshot {
            frame_count: self.movie.effective_frame_count(),
            current_frame: self.movie.current_frame(),
            is_playing: self.movie.is_playing(),
            rock_enabled: self.movie.is_rock_enabled(),
        }
    }

    /// Apply interpolated object transforms from the movie's current frame to the registry.
    pub fn apply_movie_object_transforms(&mut self) -> bool {
        let mut changed = false;
        for (name, transform) in self.movie.objects_with_transforms() {
            if let Some(obj) = self.registry.get_molecule_mut(&name) {
                obj.state_mut().set_transform(transform);
                obj.invalidate(DirtyFlags::COORDS);
                changed = true;
            }
        }
        changed
    }
}

fn is_exact_atom_macro(spec: &MacroSpec) -> bool {
    exact_pattern(&spec.model).is_some()
        && exact_pattern(&spec.segi).is_some()
        && exact_pattern(&spec.chain).is_some()
        && exact_pattern(&spec.resn).is_some()
        && exact_residue_identifier(spec).is_some()
        && exact_pattern(&spec.name).is_some()
        && exact_pattern(&spec.alt).is_some()
}

fn format_exact_atom_macro(spec: &MacroSpec) -> Option<String> {
    let model = exact_pattern(&spec.model)?;
    let segment = exact_pattern(&spec.segi)?;
    let chain = exact_pattern(&spec.chain)?;
    let residue_name = exact_pattern(&spec.resn)?;
    let residue_identifier = exact_residue_identifier(spec)?;
    let atom_name = exact_pattern(&spec.name)?;
    let alternate_location = exact_pattern(&spec.alt)?;
    Some(crate::pick::format_canonical_atom_path(
        model,
        segment,
        chain,
        residue_name,
        &residue_identifier,
        atom_name,
        alternate_location,
    ))
}

fn exact_pattern(pattern: &Option<Pattern>) -> Option<&str> {
    match pattern {
        Some(Pattern::Exact(value)) => Some(value),
        _ => None,
    }
}

fn exact_residue_identifier(spec: &MacroSpec) -> Option<String> {
    match spec.resi.as_ref()?.items.as_slice() {
        [ResiItem::Single(value)] => Some(value.to_string()),
        [ResiItem::InsCode(value, code)] => Some(format!("{value}{code}")),
        _ => None,
    }
}

fn recent_atom_evaluation_contexts<'a>(
    registry: &'a ObjectRegistry,
    selections: &SelectionManager,
    settings: &Settings,
) -> Vec<EvalContext<'a>> {
    let object_names = registry.names().map(ToOwned::to_owned).collect::<Vec<_>>();
    let options = SelectionOptions {
        ignore_case: settings.behavior.ignore_case,
        ignore_case_chain: settings.behavior.ignore_case_chain,
    };
    object_names
        .iter()
        .filter_map(|object_name| {
            let molecule = registry.get_molecule(object_name)?;
            Some(selections.build_eval_context(
                molecule.molecule(),
                molecule.display_state(),
                object_name,
                &object_names,
                options,
            ))
        })
        .collect()
}

fn exact_singleton_in_contexts(expression: &SelectionExpr, contexts: &[EvalContext<'_>]) -> bool {
    exact_singleton_target_in_contexts(expression, contexts).is_some()
}

fn resolve_recent_atom_in_contexts(
    path: &str,
    contexts: &[EvalContext<'_>],
) -> Option<(String, patinae_mol::AtomIndex)> {
    let SelectionExpr::Macro(spec) = parse(path).ok()? else {
        return None;
    };
    if !is_exact_atom_macro(&spec) {
        return None;
    }
    exact_singleton_target_in_contexts(&SelectionExpr::Macro(spec), contexts)
}

fn exact_singleton_target_in_contexts(
    expression: &SelectionExpr,
    contexts: &[EvalContext<'_>],
) -> Option<(String, patinae_mol::AtomIndex)> {
    let mut target = None;
    for context in contexts {
        let selection = evaluate(expression, context).ok()?;
        if selection.count() > 1 {
            return None;
        }
        if let Some(atom_index) = selection.first() {
            if target.is_some() {
                return None;
            }
            let object_name = context.first_molecule()?.name.clone();
            target = Some((object_name, atom_index));
        }
    }
    target
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movie::LoopMode;
    use crate::object::MoleculeObject;
    use crate::scene::SceneStoreMask;
    use lin_alg::f32::Vec3;
    use patinae_mol::{Atom, CoordSet, Element, ObjectMolecule};
    use patinae_settings::groups::RecentPickLimit;

    fn single_atom_molecule(name: &str, atom_name: &str, chain: &str) -> ObjectMolecule {
        let mut molecule = ObjectMolecule::new(name);
        molecule.add_atom(
            patinae_mol::AtomBuilder::new()
                .name(atom_name)
                .element(Element::Carbon)
                .resn("GLY")
                .resv(1)
                .chain(chain)
                .build(),
        );
        molecule.add_coord_set(CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0)]));
        molecule
    }

    fn atom_path(session: &Session, object_name: &str, atom_index: usize) -> String {
        let molecule = session.registry.get_molecule(object_name).unwrap();
        crate::canonical_atom_path_for_hit(
            &crate::PickHit {
                object_name: object_name.to_string(),
                object_type: crate::ObjectType::Molecule,
                atom_index: Some(patinae_mol::AtomIndex(atom_index.try_into().unwrap())),
                position: Vec3::new(0.0, 0.0, 0.0),
                distance: 0.0,
            },
            molecule.molecule(),
        )
        .unwrap()
    }

    fn multi_state_molecule(name: &str, states: usize) -> ObjectMolecule {
        let mut mol = ObjectMolecule::new(name);
        mol.add_atom(Atom::new("C", Element::Carbon));
        for state in 0..states {
            mol.add_coord_set(CoordSet::from_vec3(&[Vec3::new(state as f32, 0.0, 0.0)]));
        }
        mol
    }

    #[test]
    fn sync_movie_frame_applies_mset_display_state() {
        let mut session = Session::new();
        session.registry.add(MoleculeObject::with_name(
            multi_state_molecule("mol", 3),
            "mol",
        ));
        session.movie.set_from_spec(vec![1, 2, 3]);
        session.movie.goto_frame(2);

        assert!(session.sync_movie_frame());

        let obj = session.registry.get_molecule("mol").unwrap();
        assert_eq!(obj.display_state(), 2);
    }

    #[test]
    fn update_animations_advances_and_syncs_state() {
        let mut session = Session::new();
        session.registry.add(MoleculeObject::with_name(
            multi_state_molecule("mol", 3),
            "mol",
        ));
        session.refresh_movie_state_count();
        session.movie.set_loop_mode(LoopMode::Loop);
        session.settings.movie.movie_fps = 10.0;
        session.movie.play();

        let update = session.update_animations(0.11);

        assert!(update.movie_frame_changed);
        assert!(update.movie_synced);
        assert!(update.needs_redraw);
        let obj = session.registry.get_molecule("mol").unwrap();
        assert_eq!(session.movie.current_frame(), 1);
        assert_eq!(obj.display_state(), 1);
    }

    #[test]
    fn sync_movie_frame_applies_movie_view_after_scene_view() {
        let mut session = Session::new();
        session.camera.set_fov(20.0);
        session.scenes.store(
            "scene_a",
            SceneStoreMask::VIEW,
            &session.camera,
            &session.registry,
        );

        session.movie.set_frame_count(1);
        let mut movie_view = session.camera.current_view();
        movie_view.fov = 35.0;
        let frame = session.movie.frame_mut(0).unwrap();
        frame.scene_name = Some("scene_a".to_string());
        frame.set_view(movie_view);

        session.camera.set_fov(10.0);
        assert!(session.sync_movie_frame());

        assert_eq!(session.camera.fov(), 35.0);
    }

    #[test]
    fn new_session_has_empty_unlimited_recent_atoms() {
        let session = Session::new();

        assert!(session.recent_atoms.is_empty());
        assert_eq!(
            session.settings.behavior.recent_pick_limit(),
            RecentPickLimit::Unlimited
        );
    }

    #[test]
    fn legacy_named_session_defaults_recent_atoms_and_limit() {
        let session = Session::new();
        let mut value = serde_json::to_value(&session).unwrap();
        let fields = value.as_object_mut().unwrap();
        fields.remove("recent_atoms");
        fields
            .get_mut("settings")
            .unwrap()
            .get_mut("behavior")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("max_recent_picks");

        let restored: Session = serde_json::from_value(value).unwrap();

        assert!(restored.recent_atoms.is_empty());
        assert_eq!(restored.settings.behavior.max_recent_picks, -1);
    }

    #[test]
    fn legacy_positional_session_defaults_recent_atoms() {
        let session = Session::new();
        let mut legacy_settings = serde_json::to_value(&session.settings).unwrap();
        legacy_settings["behavior"]
            .as_object_mut()
            .unwrap()
            .remove("max_recent_picks");
        let value = serde_json::Value::Array(vec![
            serde_json::to_value(session.registry.to_snapshot()).unwrap(),
            serde_json::to_value(&session.camera).unwrap(),
            serde_json::to_value(&session.selections).unwrap(),
            serde_json::to_value(&session.scenes).unwrap(),
            serde_json::to_value(&session.views).unwrap(),
            serde_json::to_value(&session.movie).unwrap(),
            legacy_settings,
            serde_json::to_value(&session.named_palette).unwrap(),
            serde_json::to_value(&session.palette).unwrap(),
            serde_json::to_value(session.clear_color).unwrap(),
            serde_json::to_value(session.clear_color_set).unwrap(),
        ]);

        let restored: Session = serde_json::from_value(value).unwrap();

        assert!(restored.recent_atoms.is_empty());
        assert_eq!(restored.settings.behavior.max_recent_picks, -1);
    }

    #[test]
    fn session_load_deduplicates_then_applies_recent_atom_limit() {
        let mut session = Session::new();
        for name in ["one", "two", "three"] {
            session.registry.add(MoleculeObject::with_name(
                single_atom_molecule(name, "CA", "A"),
                name,
            ));
        }
        let one = atom_path(&session, "one", 0);
        let two = atom_path(&session, "two", 0);
        let three = atom_path(&session, "three", 0);
        session.settings.behavior.max_recent_picks = 2;
        let mut value = serde_json::to_value(&session).unwrap();
        value["recent_atoms"] = serde_json::json!([one, two, one, three]);

        let restored: Session = serde_json::from_value(value).unwrap();

        assert_eq!(
            restored.recent_atoms.paths().collect::<Vec<_>>(),
            [two, three]
        );
    }

    #[test]
    fn reconcile_recent_atoms_keeps_only_exact_singletons_in_stable_order() {
        let mut session = Session::new();
        session.registry.add(MoleculeObject::with_name(
            single_atom_molecule("first", "CA", "A"),
            "first",
        ));
        session.registry.add(MoleculeObject::with_name(
            single_atom_molecule("second", "CA", "A"),
            "second",
        ));
        let valid = atom_path(&session, "first", 0);
        let zero = valid.replacen("/first/", "/missing/", 1);
        let multiple = r#"/*/""/A/GLY`"1 "/CA`" ""#.to_string();

        session
            .recent_atoms
            .insert("not a path", RecentPickLimit::Unlimited);
        session
            .recent_atoms
            .insert(valid.clone(), RecentPickLimit::Unlimited);
        let valid_id = session.recent_atoms.row_id(&valid).unwrap();
        session
            .recent_atoms
            .insert(zero, RecentPickLimit::Unlimited);
        session
            .recent_atoms
            .insert(multiple, RecentPickLimit::Unlimited);

        assert!(session.reconcile_recent_atoms());

        assert_eq!(session.recent_atoms.paths().collect::<Vec<_>>(), [valid]);
        assert_eq!(session.recent_atoms.rows()[0].id(), valid_id);
    }

    #[test]
    fn resolved_recent_atoms_return_object_local_atom_indices() {
        let mut session = Session::new();
        session.registry.add(MoleculeObject::with_name(
            single_atom_molecule("first", "CA", "A"),
            "first",
        ));
        session.registry.add(MoleculeObject::with_name(
            single_atom_molecule("second", "CB", "B"),
            "second",
        ));
        let first = atom_path(&session, "first", 0);
        let second = atom_path(&session, "second", 0);
        session
            .recent_atoms
            .insert(first, RecentPickLimit::Unlimited);
        session
            .recent_atoms
            .insert(second, RecentPickLimit::Unlimited);

        assert_eq!(
            session.resolved_recent_atoms(),
            vec![
                ("first".to_string(), patinae_mol::AtomIndex(0)),
                ("second".to_string(), patinae_mol::AtomIndex(0)),
            ]
        );
    }

    #[test]
    fn singleton_validation_keeps_case_insensitive_model_ambiguity() {
        let mut session = Session::new();
        for name in ["Obj", "obj"] {
            session.registry.add(MoleculeObject::with_name(
                single_atom_molecule(name, "CA", "A"),
                name,
            ));
        }
        let path = atom_path(&session, "Obj", 0);

        session.settings.behavior.ignore_case = false;
        assert!(session.recent_atom_path_is_singleton(&path));
        session.settings.behavior.ignore_case = true;
        assert!(!session.recent_atom_path_is_singleton(&path));
    }

    #[test]
    fn object_rename_rewrites_exact_model_and_preserves_row_identity() {
        let old_name = "old/model*";
        let new_name = "new?model";
        let mut session = Session::new();
        session.registry.add(MoleculeObject::with_name(
            single_atom_molecule(old_name, "CA", "A"),
            old_name,
        ));
        let old_path = atom_path(&session, old_name, 0);
        session
            .recent_atoms
            .insert(old_path.clone(), RecentPickLimit::Unlimited);
        let old_id = session.recent_atoms.row_id(&old_path).unwrap();
        let mut target = Session::new();
        target.registry.add(MoleculeObject::with_name(
            single_atom_molecule(new_name, "CA", "A"),
            new_name,
        ));
        let colliding_path = atom_path(&target, new_name, 0);
        session
            .recent_atoms
            .insert(colliding_path, RecentPickLimit::Unlimited);

        session.rename_object(old_name, new_name).unwrap();
        let rewritten_path = atom_path(&session, new_name, 0);

        assert_eq!(
            session.recent_atoms.paths().collect::<Vec<_>>(),
            [rewritten_path]
        );
        assert_eq!(session.recent_atoms.rows()[0].id(), old_id);
    }

    #[test]
    fn object_and_atom_removal_prune_only_affected_recent_rows() {
        let mut session = Session::new();
        for name in ["removed", "trimmed", "kept"] {
            session.registry.add(MoleculeObject::with_name(
                single_atom_molecule(name, "CA", "A"),
                name,
            ));
        }
        let removed = atom_path(&session, "removed", 0);
        let trimmed = atom_path(&session, "trimmed", 0);
        let kept = atom_path(&session, "kept", 0);
        for path in [&removed, &trimmed, &kept] {
            session
                .recent_atoms
                .insert(path.clone(), RecentPickLimit::Unlimited);
        }

        session.remove_object("removed");
        session
            .remove_molecule_atoms("trimmed", &[patinae_mol::AtomIndex(0)])
            .unwrap();

        assert_eq!(session.recent_atoms.paths().collect::<Vec<_>>(), [kept]);
    }

    #[test]
    fn inserting_same_named_non_molecule_prunes_recent_atom_path() {
        let mut session = Session::new();
        session.registry.add(MoleculeObject::with_name(
            single_atom_molecule("picked", "CA", "A"),
            "picked",
        ));
        let path = atom_path(&session, "picked", 0);
        session
            .recent_atoms
            .insert(path, RecentPickLimit::Unlimited);

        session.insert_object(Box::new(crate::object::GroupObject::new("picked")));

        assert!(session.recent_atoms.is_empty());
        assert!(session.registry.get_group("picked").is_some());
    }

    #[test]
    fn batch_insertion_reconciles_recent_atoms_once_after_all_objects() {
        let mut session = Session::new();
        for name in ["first", "second"] {
            session.registry.add(MoleculeObject::with_name(
                single_atom_molecule(name, "CA", "A"),
                name,
            ));
            let path = atom_path(&session, name, 0);
            session
                .recent_atoms
                .insert(path, RecentPickLimit::Unlimited);
        }
        let recent_generation = session.recent_atoms.generation();
        let registry_generation = session.registry.generation();

        session.insert_objects(
            ["first", "second"]
                .into_iter()
                .map(|name| Box::new(crate::object::GroupObject::new(name)) as Box<dyn Object>),
        );

        assert!(session.recent_atoms.is_empty());
        assert_eq!(session.recent_atoms.generation(), recent_generation + 1);
        assert_eq!(session.registry.generation(), registry_generation + 2);
    }
}
