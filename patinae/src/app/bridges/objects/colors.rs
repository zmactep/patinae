//! Representation color assignments for the current color popover target.

use super::*;
use patinae_cmd::commands::selecting::evaluate_selection;
use patinae_color::ColorValue;
use patinae_scene::ViewerLike;

/// Find local assignments using the same selection semantics as `unset`.
pub(super) fn color_assignments(viewer: &dyn ViewerLike, target: &str) -> RepMask {
    let mut assigned = RepMask::NONE;
    let Ok(selections) = evaluate_selection(viewer, target) else {
        return assigned;
    };
    for (name, selected) in selections {
        let Some(object) = viewer.objects().get_molecule(&name) else {
            continue;
        };
        for index in selected.indices() {
            let Some(atom) = object.molecule().get_atom(index) else {
                continue;
            };
            let colors = &atom.repr.colors;
            for (rep, color) in [
                (RepMask::LINES, colors.line),
                (RepMask::STICKS, colors.stick),
                (RepMask::SPHERES, colors.sphere),
                (RepMask::CARTOON, colors.cartoon),
                (RepMask::RIBBON, colors.ribbon),
                (RepMask::SURFACE, colors.surface),
                (RepMask::MESH, colors.mesh),
                (RepMask::DOTS, colors.dot),
            ] {
                if color != ColorValue::INHERIT {
                    assigned.set_visible(rep);
                }
            }
        }
    }
    assigned
}

/// Replace all flags on open and after scene or selection changes.
pub(super) fn sync_color_assignments(os: &ObjectsState, kernel: &mut AppKernel) {
    let assigned = if os.get_popover_solid_only() {
        RepMask::NONE
    } else {
        kernel.mutate_viewer(None, (0, 0), |viewer| {
            color_assignments(viewer, os.get_popover_target().as_str())
        })
    };
    os.set_popover_color_lines(assigned.is_visible(RepMask::LINES));
    os.set_popover_color_sticks(assigned.is_visible(RepMask::STICKS));
    os.set_popover_color_spheres(assigned.is_visible(RepMask::SPHERES));
    os.set_popover_color_cartoon(assigned.is_visible(RepMask::CARTOON));
    os.set_popover_color_ribbon(assigned.is_visible(RepMask::RIBBON));
    os.set_popover_color_surface(assigned.is_visible(RepMask::SURFACE));
    os.set_popover_color_mesh(assigned.is_visible(RepMask::MESH));
    os.set_popover_color_dots(assigned.is_visible(RepMask::DOTS));
}
