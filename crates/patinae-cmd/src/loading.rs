//! Shared loading options, retained results and host application.

use crate::tasks::{TaskConfig, TaskData, TaskDiagnostic, TaskEffects, TaskOutcome};
use crate::{FetchFormatCode, FetchRequest, ViewerLike};

/// Scene preparation settings captured by a loading request.
#[derive(Debug, Clone, Copy)]
pub struct LoadOptions {
    pub bond_tolerance: f32,
    pub auto_dss: bool,
    pub dss_algorithm: patinae_settings::DssAlgorithm,
}

impl From<&patinae_settings::Settings> for LoadOptions {
    fn from(settings: &patinae_settings::Settings) -> Self {
        Self {
            bond_tolerance: settings.behavior.bonding_vdw_cutoff,
            auto_dss: settings.behavior.auto_dss,
            dss_algorithm: settings.behavior.dss_algorithm,
        }
    }
}

impl From<&FetchRequest> for LoadOptions {
    fn from(request: &FetchRequest) -> Self {
        Self {
            bond_tolerance: request.bond_tolerance,
            auto_dss: request.auto_dss,
            dss_algorithm: request.dss_algorithm,
        }
    }
}

/// Identifies loaded data consistently across native, Python and browser hosts.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LoadedData {
    pub object_name: String,
    pub format: String,
    pub pdb_id: Option<String>,
    pub atom_count: Option<usize>,
}

impl LoadedData {
    pub fn file(name: &str, format: &str) -> Self {
        Self {
            object_name: name.into(),
            format: format.to_ascii_lowercase(),
            pdb_id: None,
            atom_count: None,
        }
    }

    pub fn fetched(request: &FetchRequest) -> Self {
        let mut data = Self::file(
            &request.name,
            match request.format {
                FetchFormatCode::Pdb => "pdb",
                FetchFormatCode::Cif => "cif",
                FetchFormatCode::Bcif => "bcif",
            },
        );
        data.pdb_id = Some(request.code.to_ascii_lowercase());
        data
    }

    fn outcome(&self) -> TaskOutcome {
        TaskOutcome::success(
            Some(TaskData {
                kind: "loaded_data".into(),
                schema_version: 1,
                payload: serde_json::to_value(self).expect("serializable loading summary"),
            }),
            TaskEffects::Applied,
        )
    }

    /// Decode and apply supplied bytes using the session's outcome budget.
    pub fn apply_bytes(
        self,
        viewer: &mut dyn ViewerLike,
        bytes: &[u8],
        options: LoadOptions,
        config: &TaskConfig,
    ) -> TaskOutcome {
        let name = self.object_name.clone();
        let format = self.format.clone();
        self.apply(viewer, config, |viewer| {
            crate::commands::io::apply_loaded_data(
                viewer,
                bytes,
                &name,
                &format,
                options.bond_tolerance,
                options.auto_dss,
                options.dss_algorithm,
            )
        })
    }

    /// Apply a molecule decoded by a native worker with the same result contract.
    pub fn apply_molecule(
        self,
        viewer: &mut dyn ViewerLike,
        molecule: patinae_mol::ObjectMolecule,
        options: LoadOptions,
        config: &TaskConfig,
    ) -> TaskOutcome {
        let name = self.object_name.clone();
        self.apply(viewer, config, |viewer| {
            crate::commands::io::finalize_fetched_molecule(
                viewer,
                &name,
                molecule,
                options.auto_dss,
                options.dss_algorithm,
            );
            Ok(Vec::new())
        })
    }

    fn apply(
        mut self,
        viewer: &mut dyn ViewerLike,
        config: &TaskConfig,
        apply: impl FnOnce(&mut dyn ViewerLike) -> Result<Vec<String>, crate::CmdError>,
    ) -> TaskOutcome {
        // Reserve the largest possible count before any scene mutation.
        self.atom_count = Some(usize::MAX);
        if let Err(error) = config.validate_outcome(&self.outcome()) {
            return TaskOutcome::failure(error.code, error.message);
        }
        match apply(viewer) {
            Ok(warnings) => {
                self.atom_count = viewer
                    .objects()
                    .get_molecule(&self.object_name)
                    .map(|m| m.molecule().atom_count());
                let mut outcome = self.outcome();
                outcome.diagnostics = warnings
                    .into_iter()
                    .map(|message| TaskDiagnostic {
                        level: "warning".into(),
                        message,
                    })
                    .collect();
                outcome
            }
            Err(error) => TaskOutcome::failure("apply_failed", error.to_string()),
        }
    }
}

/// Infer a file format independently of query strings and gzip suffixes.
pub fn infer_format(path: &str) -> String {
    let path = path
        .split(['?', '#'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase();
    let path = path.strip_suffix(".gz").unwrap_or(&path);
    std::path::Path::new(path)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("pdb")
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::TaskState;
    use patinae_scene::{Session, SessionAdapter};

    const CIF: &str = "data_sample\nloop_\n_atom_site.id\n_atom_site.type_symbol\n_atom_site.label_atom_id\n_atom_site.label_comp_id\n_atom_site.label_asym_id\n_atom_site.label_seq_id\n_atom_site.Cartn_x\n_atom_site.Cartn_y\n_atom_site.Cartn_z\n1 C CA GLY A 1 0.0 0.0 0.0\n";

    #[test]
    fn worker_decoded_and_browser_bytes_produce_identical_results() {
        let mut results = Vec::new();
        for decoded in [false, true] {
            let mut session = Session::new();
            let mut redraw = false;
            let mut options = LoadOptions::from(&session.settings);
            options.auto_dss = false;
            let mut viewer = SessionAdapter {
                session: &mut session,
                render_context: None,
                default_size: (1, 1),
                needs_redraw: &mut redraw,
            };
            let data = LoadedData::file("sample", "CIF");
            let outcome = if decoded {
                let molecule =
                    patinae_io::cif::read_cif_str_with_bond_tolerance(CIF, options.bond_tolerance)
                        .unwrap();
                data.apply_molecule(&mut viewer, molecule, options, &TaskConfig::default())
            } else {
                data.apply_bytes(&mut viewer, CIF.as_bytes(), options, &TaskConfig::default())
            };
            assert_eq!(outcome.state(), TaskState::Succeeded);
            assert_eq!(
                viewer
                    .objects()
                    .get_molecule("sample")
                    .unwrap()
                    .molecule()
                    .atom_count(),
                1
            );
            results.push(outcome);
        }
        assert_eq!(results[0], results[1]);
        let crate::tasks::TaskOutcomeStatus::Success { data: Some(data) } = &results[0].status
        else {
            panic!("expected a loading result");
        };
        assert_eq!(data.kind, "loaded_data");
        assert_eq!(data.schema_version, 1);
        assert_eq!(
            data.payload,
            serde_json::json!({
                "object_name": "sample", "format": "cif", "pdb_id": null, "atom_count": 1,
            })
        );
    }

    #[test]
    fn oversized_result_is_rejected_before_scene_application() {
        let mut session = Session::new();
        let mut redraw = false;
        let options = LoadOptions::from(&session.settings);
        let mut viewer = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (1, 1),
            needs_redraw: &mut redraw,
        };
        let data = LoadedData::file(&"x".repeat(65536), "cif");
        let outcome =
            data.apply_bytes(&mut viewer, CIF.as_bytes(), options, &TaskConfig::default());
        assert_eq!(outcome.state(), TaskState::Failed);
        assert_eq!(outcome.effects, TaskEffects::None);
        assert_eq!(viewer.objects().names().count(), 0);
        assert!(!redraw);
    }

    #[test]
    fn format_inference_handles_queries_compression_and_case() {
        assert_eq!(
            infer_format("https://example.org/sample.CIF.GZ?token=x#part"),
            "cif"
        );
        assert_eq!(infer_format("sample.PDB"), "pdb");
        assert_eq!(infer_format("sample"), "pdb");
    }
}
