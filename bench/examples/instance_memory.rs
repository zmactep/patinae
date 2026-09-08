//! CPU storage probe; run each mode/count in a fresh process under `/usr/bin/time -l`.

use std::hint::black_box;

use patinae_mol::{
    materialize_molecule, Atom, CoordSet, Element, InstanceGroup, InstanceTable, ObjectInstance,
    ObjectMolecule, IDENTITY_INSTANCE,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().ok_or("expected instanced or explicit")?;
    let copies: usize = args.next().ok_or("expected copy count")?.parse()?;
    if !matches!(mode.as_str(), "instanced" | "explicit") {
        return Err("expected instanced or explicit".into());
    }
    let source_count = 2_000;
    let mut source = ObjectMolecule::new("memory_probe");
    let mut coords = Vec::with_capacity(source_count * 3);
    for atom in 0..source_count {
        source.add_atom(Atom::new("CA", Element::Carbon));
        coords.extend_from_slice(&[atom as f32, 0., 0.]);
    }
    source.add_coord_set(CoordSet::from_coords(coords));
    let table = InstanceTable {
        groups: vec![InstanceGroup::default()],
        copies: (0..copies)
            .map(|copy| {
                let mut transform = IDENTITY_INSTANCE;
                transform[3][1] = copy as f32 * 10.;
                ObjectInstance {
                    group: 0,
                    transform,
                }
            })
            .collect(),
    };
    table.validate(source_count)?;
    let stored = if mode == "explicit" {
        materialize_molecule(&source, 0, &table)?
    } else {
        source
    };
    println!("{mode}: copies={copies}, stored_atoms={}, displayed_atoms={}, atom_record_bytes={}, coordinate_bytes={}, copy_matrix_bytes={}",
        stored.atom_count(), source_count * copies,
        stored.atom_count() * std::mem::size_of::<Atom>(),
        stored.atom_count() * 3 * std::mem::size_of::<f32>(),
        if mode == "instanced" { copies * std::mem::size_of::<ObjectInstance>() } else { 0 });
    black_box((&stored, &table));
    Ok(())
}
