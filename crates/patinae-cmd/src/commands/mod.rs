//! Command implementations
//!
//! This module contains all built-in command implementations organized by category.

pub mod align;
pub mod control;
pub mod crystal;
pub mod display;
pub mod io;
pub mod map;
pub mod measuring;
pub mod movie;
pub mod objects;
pub mod picking;
pub mod scene;
pub mod selecting;
pub mod settings;
pub mod transform;
pub mod viewing;

use crate::command::CommandRegistry;

/// Register all built-in commands with the registry
pub fn register_all(registry: &mut CommandRegistry) {
    // File I/O commands
    io::register(registry);

    // Viewing commands
    viewing::register(registry);

    // Display commands
    display::register(registry);

    // Selection commands
    selecting::register(registry);
    picking::register(registry);

    // Object commands
    objects::register(registry);

    // Settings commands
    settings::register(registry);

    // Control commands
    control::register(registry);

    // Scene commands
    scene::register(registry);

    // Movie commands
    movie::register(registry);

    // Transform commands (translate, rotate, transform_selection)
    transform::register(registry);

    // Alignment commands
    align::register(registry);

    // Crystal symmetry commands
    crystal::register(registry);

    // Measurement commands (distance, angle, dihedral)
    measuring::register(registry);

    // Map commands (isomesh, isodot, isosurface)
    map::register(registry);
}
