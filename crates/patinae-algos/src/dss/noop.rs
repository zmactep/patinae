//! No-op secondary structure assignment — returns Loop for every residue.

use super::{BackboneResidue, SecondaryStructureAssigner, SsType};

/// No-op assigner that assigns [`SsType::Loop`] to all residues.
#[derive(Default)]
pub struct NoOp;

impl SecondaryStructureAssigner for NoOp {
    fn assign(&self, residues: &[BackboneResidue]) -> Vec<SsType> {
        vec![SsType::Loop; residues.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_returns_all_loop() {
        let noop = NoOp;
        assert!(noop.assign(&[]).is_empty());
        let point = lin_alg::f32::Vec3::new(1.0, 2.0, 3.0);
        let residues: Vec<_> = (1..=3)
            .map(|resv| BackboneResidue {
                ca: point,
                n: point,
                c: point,
                o: point,
                chain: "A".into(),
                resv,
                nh_direction: None,
                bonded_to_prev: false,
            })
            .collect();
        assert_eq!(noop.assign(&residues), vec![SsType::Loop; 3]);
    }
}
