//! Async fetch producers using the host's common task lifecycle.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use patinae_cmd::loading::{infer_format, LoadOptions, LoadedData};
use patinae_cmd::tasks::{TaskData, TaskEffects, TaskError, TaskId, TaskOutcome};
use patinae_cmd::{AsyncCommandRequest, FetchFormatCode, FetchRequest};
use patinae_framework::kernel::AppKernel;
use patinae_framework::tasks::{AsyncTask, TaskResult};
use patinae_mol::ObjectMolecule;

// Bound network waits; synchronous parsing can finish after the timeout deadline.
const FETCH_TIMEOUT_SECS: u64 = 10;
const METADATA_TIMEOUT_SECS: u64 = 6;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PdbMetadataMessage {
    pub pdb_id: String,
    pub title: String,
    pub details: String,
    pub method: String,
    pub deposit_date: String,
    pub release_date: String,
    pub doi: String,
}

/// Convert a command request into a task bound to the current scene.
pub fn task_from_request(
    request: AsyncCommandRequest,
    scene_epoch: u64,
) -> Option<Box<dyn AsyncTask>> {
    match request {
        AsyncCommandRequest::Fetch(request) => Some(Box::new(FetchTask {
            request,
            scene_epoch,
        })),
        AsyncCommandRequest::LoadUrl { url, name, format } => Some(Box::new(UrlTask {
            url,
            name,
            format,
            scene_epoch,
        })),
        _ => None,
    }
}

struct UrlTask {
    url: String,
    name: String,
    format: Option<String>,
    scene_epoch: u64,
}

impl AsyncTask for UrlTask {
    fn kind(&self) -> &str {
        "load"
    }
    fn scene_epoch(&self) -> Option<u64> {
        Some(self.scene_epoch)
    }
    fn notification_message(&self) -> String {
        format!("Loading {}...", self.name)
    }
    fn execute(self: Box<Self>) -> Pin<Box<dyn Future<Output = Box<dyn TaskResult>> + Send>> {
        Box::pin(async move {
            let result = patinae_io::fetch::fetch_url_bytes(
                &self.url,
                Duration::from_secs(FETCH_TIMEOUT_SECS),
            )
            .await
            .map_err(io_task_error);
            let format = self.format.unwrap_or_else(|| infer_format(&self.url));
            Box::new(UrlResult {
                name: self.name,
                format,
                result,
            }) as Box<dyn TaskResult>
        })
    }
}

struct UrlResult {
    name: String,
    format: String,
    result: Result<Vec<u8>, TaskError>,
}

impl TaskResult for UrlResult {
    fn apply(self: Box<Self>, kernel: &mut AppKernel, _id: TaskId) -> TaskOutcome {
        let data = match self.result {
            Ok(data) => data,
            Err(error) => return TaskOutcome::failure(error.code, error.message),
        };
        let options = LoadOptions::from(&kernel.session.settings);
        let config = kernel.tasks.config().clone();
        let outcome = kernel.mutate_viewer(None, (1, 1), |viewer| {
            LoadedData::file(&self.name, &self.format).apply_bytes(viewer, &data, options, &config)
        });
        for warning in &outcome.diagnostics {
            kernel.bus.print_warning(&warning.message);
        }
        outcome
    }
}

struct FetchTask {
    request: FetchRequest,
    scene_epoch: u64,
}

pub struct PdbMetadataTask {
    pdb_id: String,
}

impl PdbMetadataTask {
    pub fn new(pdb_id: impl Into<String>) -> Self {
        Self {
            pdb_id: pdb_id.into(),
        }
    }
}

impl AsyncTask for FetchTask {
    fn kind(&self) -> &str {
        "fetch"
    }

    fn scene_epoch(&self) -> Option<u64> {
        Some(self.scene_epoch)
    }

    fn notification_message(&self) -> String {
        format!("Fetching {}...", self.request.code)
    }

    fn execute(self: Box<Self>) -> Pin<Box<dyn Future<Output = Box<dyn TaskResult>> + Send>> {
        Box::pin(async move {
            let request = self.request;
            let result = match tokio::time::timeout(
                Duration::from_secs(FETCH_TIMEOUT_SECS),
                patinae_io::fetch_async_with_bond_tolerance(
                    &request.code,
                    to_io_format(request.format),
                    request.bond_tolerance,
                ),
            )
            .await
            {
                Ok(result) => result.map_err(io_task_error),
                Err(_) => Err(TaskError::new(
                    "timeout",
                    format!("timeout after {FETCH_TIMEOUT_SECS} seconds"),
                )),
            };
            Box::new(FetchResult { request, result }) as Box<dyn TaskResult>
        })
    }
}

struct FetchResult {
    request: FetchRequest,
    result: Result<ObjectMolecule, TaskError>,
}

impl TaskResult for FetchResult {
    fn apply(self: Box<Self>, kernel: &mut AppKernel, _id: TaskId) -> TaskOutcome {
        match self.result {
            Ok(molecule) => {
                let options = LoadOptions::from(&self.request);
                let config = kernel.tasks.config().clone();
                let outcome = kernel.mutate_viewer(None, (1, 1), |viewer| {
                    LoadedData::fetched(&self.request)
                        .apply_molecule(viewer, molecule, options, &config)
                });
                if outcome.state() == patinae_cmd::tasks::TaskState::Succeeded {
                    kernel.output.print_info(format!(
                        " Fetched {} as \"{}\"",
                        self.request.code, self.request.name
                    ));
                }
                outcome
            }
            Err(error) => {
                kernel.print_fetch_error(&self.request, &error.message);
                TaskOutcome::failure(error.code, error.message)
            }
        }
    }
}

impl AsyncTask for PdbMetadataTask {
    fn kind(&self) -> &str {
        "pdb_metadata"
    }

    fn origin(&self) -> String {
        "ui.fetch_preview".into()
    }

    fn notification_message(&self) -> String {
        format!("Looking up {} metadata...", self.pdb_id)
    }

    fn execute(self: Box<Self>) -> Pin<Box<dyn Future<Output = Box<dyn TaskResult>> + Send>> {
        Box::pin(async move {
            let result = match tokio::time::timeout(
                Duration::from_secs(METADATA_TIMEOUT_SECS),
                patinae_io::fetch_pdb_metadata(&self.pdb_id),
            )
            .await
            {
                Ok(result) => result.map_err(io_task_error),
                Err(_) => Err(TaskError::new(
                    "timeout",
                    format!("timeout after {METADATA_TIMEOUT_SECS} seconds"),
                )),
            };
            let outcome = match result {
                Ok(preview) => TaskOutcome::success(
                    Some(TaskData {
                        kind: "pdb_metadata".into(),
                        schema_version: 1,
                        payload: serde_json::json!(PdbMetadataMessage {
                            pdb_id: self.pdb_id,
                            title: format!("{} - {}", preview.rcsb_id, preview.title),
                            details: String::new(),
                            method: preview.methods.join(", "),
                            deposit_date: preview.deposit_date.unwrap_or_default(),
                            release_date: preview.release_date.unwrap_or_default(),
                            doi: preview.doi.unwrap_or_default(),
                        }),
                    }),
                    TaskEffects::None,
                ),
                Err(error) => TaskOutcome::failure(error.code, error.message),
            };
            Box::new(MetadataResult(outcome)) as Box<dyn TaskResult>
        })
    }
}

struct MetadataResult(TaskOutcome);

impl TaskResult for MetadataResult {
    fn preflight(&self, kernel: &AppKernel) -> Result<(), TaskError> {
        kernel.tasks.config().validate_outcome(&self.0)
    }

    fn apply(self: Box<Self>, _kernel: &mut AppKernel, _id: TaskId) -> TaskOutcome {
        self.0
    }
}

fn io_task_error(error: patinae_io::IoError) -> TaskError {
    let code = if error.is_invalid_pdb_id() {
        "invalid_pdb_id"
    } else if error.is_fetch() || error.is_io() {
        "network_error"
    } else {
        "parse_error"
    };
    TaskError::new(code, error.to_string())
}

fn to_io_format(format: FetchFormatCode) -> patinae_io::FetchFormat {
    match format {
        FetchFormatCode::Pdb => patinae_io::FetchFormat::Pdb,
        FetchFormatCode::Cif => patinae_io::FetchFormat::Cif,
        FetchFormatCode::Bcif => patinae_io::FetchFormat::Bcif,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_cmd::tasks::{TaskOutcomeStatus, TaskState};
    use patinae_mol::{Atom, CoordSet, Element};
    use std::time::Instant;

    struct ReadyFetch(FetchResult);
    impl AsyncTask for ReadyFetch {
        fn notification_message(&self) -> String {
            "fixture fetch".into()
        }
        fn execute(self: Box<Self>) -> Pin<Box<dyn Future<Output = Box<dyn TaskResult>> + Send>> {
            Box::pin(async move { Box::new(self.0) as Box<dyn TaskResult> })
        }
    }

    fn request(name: &str) -> FetchRequest {
        FetchRequest {
            code: "1ubq".into(),
            name: name.into(),
            format: FetchFormatCode::Cif,
            bond_tolerance: 0.4,
            auto_dss: false,
            dss_algorithm: patinae_settings::DssAlgorithm::default(),
        }
    }

    fn finish(kernel: &mut AppKernel, id: TaskId) -> patinae_cmd::tasks::TaskSnapshot {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            kernel.process_async_tasks();
            let snapshot = kernel.tasks.get(id).unwrap();
            if snapshot.state.is_terminal() {
                return snapshot;
            }
            assert!(Instant::now() < deadline, "task did not finish");
            std::thread::yield_now();
        }
    }

    #[test]
    fn fetch_success_contains_result_after_scene_and_movie_finalization() {
        let mut kernel = AppKernel::new();
        let mut molecule = ObjectMolecule::new("source");
        molecule.add_atom(Atom::new("CA", Element::Carbon));
        molecule.add_coord_set(CoordSet::from_vec3(&[lin_alg::f32::Vec3::new(
            0.0, 0.0, 0.0,
        )]));
        let id = kernel
            .spawn_task(ReadyFetch(FetchResult {
                request: request("target"),
                result: Ok(molecule),
            }))
            .unwrap();
        let snapshot = finish(&mut kernel, id);
        assert_eq!(snapshot.state, TaskState::Succeeded);
        assert!(kernel.session.registry.get("target").is_some());
        let outcome = snapshot.outcome.unwrap();
        assert_eq!(outcome.effects, TaskEffects::Applied);
        let TaskOutcomeStatus::Success { data: Some(data) } = outcome.status else {
            panic!("expected structure result");
        };
        assert_eq!(data.payload["object_name"], "target");
        assert_eq!(data.payload["atom_count"], 1);
        assert_eq!(data.schema_version, 1);
    }

    #[test]
    fn late_error_is_retained_without_console_parsing() {
        let mut kernel = AppKernel::new();
        let id = kernel
            .spawn_task(ReadyFetch(FetchResult {
                request: request("missing"),
                result: Err(TaskError::new("network_error", "fixture unavailable")),
            }))
            .unwrap();
        let snapshot = finish(&mut kernel, id);
        assert_eq!(snapshot.state, TaskState::Failed);
        let TaskOutcomeStatus::Failure { error } = snapshot.outcome.unwrap().status else {
            panic!("expected failure");
        };
        assert_eq!(error.code, "network_error");
        assert_eq!(error.message, "fixture unavailable");
        assert!(kernel.session.registry.get("missing").is_none());
    }

    #[test]
    fn oversized_result_is_rejected_before_insertion() {
        let mut kernel = AppKernel::new();
        let name = "x".repeat(70_000);
        let id = kernel
            .spawn_task(ReadyFetch(FetchResult {
                request: request(&name),
                result: Ok(ObjectMolecule::new("source")),
            }))
            .unwrap();
        let snapshot = finish(&mut kernel, id);
        let TaskOutcomeStatus::Failure { error } = snapshot.outcome.unwrap().status else {
            panic!("expected failure");
        };
        assert_eq!(error.code, "result_too_large");
        assert!(kernel.session.registry.get(&name).is_none());
    }

    #[test]
    fn io_errors_preserve_machine_readable_categories() {
        assert_eq!(
            io_task_error(patinae_io::IoError::parse_msg("bad cif")).code,
            "parse_error"
        );
        assert_eq!(
            io_task_error(patinae_io::IoError::fetch("offline")).code,
            "network_error"
        );
        assert_eq!(
            io_task_error(patinae_io::IoError::invalid_pdb_id("!")).code,
            "invalid_pdb_id"
        );
    }
}
