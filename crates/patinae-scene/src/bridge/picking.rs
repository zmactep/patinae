//! Picking bridge — translates a `patinae_render::PickHit` (`(rep_kind,
//! ObjectId, atom_id)`) into a `patinae_scene::PickHit` so the host can
//! route through `expand_pick_to_selection` / `pick_expression_for_hit`
//! unchanged.

use patinae_mol::AtomIndex;
use patinae_render::PickHit as RenderPickHit;

use crate::object::RenderObjectId;
use crate::pick::PickHit;
use crate::session::Session;

/// Translate a `patinae_render::PickHit` into a `patinae_scene::PickHit`.
/// `names` must be indexed by stable render `ObjectId` minus one.
/// Empty slots correspond to disabled or removed objects.
///
/// Returns `None` for the background sentinel (`ObjectId(0)`),
/// out-of-range slots, or hits on objects not in the registry.
pub fn resolve_pick(
    hit: RenderPickHit,
    names: &[Option<String>],
    session: &Session,
) -> Option<PickHit> {
    let idx = render_id_slot_index(hit.object_id.0)?;
    let name = names.get(idx)?.as_ref()?;
    let obj = session.registry.get(name)?;
    if !obj.is_enabled() {
        return None;
    }
    if !obj.object_type().is_pickable() {
        return None;
    }

    let molecule = session.registry.get_molecule(name)?;
    let atom_index = AtomIndex::from(hit.atom_id as usize);
    let position = molecule.instance_world_coord(atom_index, hit.instance)?;

    Some(PickHit {
        object_name: name.clone(),
        object_type: obj.object_type(),
        atom_index: Some(atom_index),
        instance: hit.instance,
        position,
        distance: 0.0,
    })
}

pub(super) fn render_id_slot_index(raw_id: u32) -> Option<usize> {
    RenderObjectId::new(raw_id).map(RenderObjectId::slot_index)
}
