//! Host-thread installation shared by startup and manifest reconciliation.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use patinae_cmd::{tasks::TaskRunner, CommandExecutor};

use crate::{
    library_identity, BackgroundPluginLoader, PluginDiscovery, PluginHost, PluginLoadEvent,
};

/// One nonblocking installation step.
#[derive(Debug, Default)]
pub struct PluginLoadUpdate {
    pub changed: bool,
    pub error: Option<String>,
}

/// Sequential preparation and attachment independent of any frontend.
pub struct PluginLoadSession {
    loader: Option<BackgroundPluginLoader>,
    processed: usize,
    total: Option<usize>,
}

impl std::fmt::Debug for PluginLoadSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PluginLoadSession")
            .field("finished", &self.finished())
            .field("processed", &self.processed)
            .field("total", &self.total)
            .finish()
    }
}

impl PluginLoadSession {
    /// Starts discovery and background preparation.
    ///
    /// # Errors
    /// Returns an error if the loader thread cannot start.
    pub fn discover(discovery: PluginDiscovery) -> std::io::Result<Self> {
        BackgroundPluginLoader::start(discovery).map(Self::new)
    }

    fn new(loader: BackgroundPluginLoader) -> Self {
        Self {
            loader: Some(loader),
            processed: 0,
            total: None,
        }
    }

    /// Whether preparation and attachment have stopped.
    pub fn finished(&self) -> bool {
        self.loader.is_none()
    }
    /// Number of libraries attempted, including individual failures.
    pub fn processed(&self) -> usize {
        self.processed
    }
    /// Number of selected libraries, once discovery has completed.
    pub fn total(&self) -> Option<usize> {
        self.total
    }

    /// Consumes at most one event and installs at most one plugin on the host thread.
    pub fn tick(
        &mut self,
        host: &mut PluginHost,
        executor: &mut CommandExecutor,
    ) -> PluginLoadUpdate {
        let Some(loader) = &self.loader else {
            return PluginLoadUpdate::default();
        };
        let mut update = PluginLoadUpdate::default();
        match loader.try_next() {
            Ok(Some(PluginLoadEvent::Discovered { directories, total })) => {
                for directory in directories {
                    host.add_plugin_dir(&directory);
                }
                self.total = Some(total);
            }
            Ok(Some(PluginLoadEvent::Plugin {
                path,
                result,
                preparation_time,
            })) => {
                let started = Instant::now();
                let attached =
                    result.and_then(|prepared| host.attach_prepared(*prepared, executor));
                loader.acknowledge();
                self.processed += 1;
                match attached {
                    Ok(name) => {
                        update.changed = true;
                        log::info!(
                            "Loaded plugin: {name} ({:.3} ms)",
                            (preparation_time + started.elapsed()).as_secs_f64() * 1000.0
                        );
                    }
                    Err(error) => {
                        update.error =
                            Some(format!("Failed to load plugin {}: {error}", path.display()))
                    }
                }
            }
            Ok(Some(PluginLoadEvent::DiscoveryError { error })) => {
                update.error = Some(format!("Plugin discovery failed: {error}"));
            }
            Ok(Some(PluginLoadEvent::Finished)) => self.loader = None,
            Err(error) => {
                update.error = Some(error);
                self.loader = None;
            }
            Ok(None) => {}
        }
        update
    }
}

#[derive(Debug, Default)]
enum ApplyStage {
    #[default]
    Idle,
    Pending(Vec<PathBuf>),
    Loading(PluginLoadSession),
    Finished,
}

/// Applies a saved library list between host callbacks, even without an open dialog.
#[derive(Debug, Default)]
pub struct PluginApplication {
    stage: ApplyStage,
    errors: Vec<String>,
}

impl PluginApplication {
    /// Queues the desired snapshot after successful persistence.
    pub fn queue(&mut self, paths: Vec<PathBuf>) {
        self.stage = ApplyStage::Pending(paths);
        self.errors.clear();
    }

    pub fn applying(&self) -> bool {
        matches!(self.stage, ApplyStage::Pending(_) | ApplyStage::Loading(_))
    }
    pub fn finished(&self) -> bool {
        matches!(self.stage, ApplyStage::Finished)
    }
    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    /// Reconciles removals and additions without restarting unchanged libraries.
    pub fn tick(
        &mut self,
        host: &mut PluginHost,
        executor: &mut CommandExecutor,
        tasks: &TaskRunner,
    ) -> bool {
        let mut changed = false;
        if matches!(self.stage, ApplyStage::Pending(_)) {
            let ApplyStage::Pending(paths) = std::mem::take(&mut self.stage) else {
                unreachable!()
            };
            let desired: HashSet<_> = paths.iter().map(|path| library_identity(path)).collect();
            let current: Vec<_> = host
                .loaded_libraries()
                .map(|(path, _)| path.to_path_buf())
                .collect();
            for path in &current {
                if !desired.contains(&library_identity(path)) {
                    changed |= host.unload_library(path, executor, tasks);
                }
            }
            let mut seen: HashSet<_> = current
                .into_iter()
                .map(|path| library_identity(&path))
                .collect();
            let additions: Vec<_> = paths
                .into_iter()
                .filter(|path| seen.insert(library_identity(path)))
                .collect();
            self.stage = if additions.is_empty() {
                ApplyStage::Finished
            } else {
                match BackgroundPluginLoader::start_paths(additions) {
                    Ok(loader) => ApplyStage::Loading(PluginLoadSession::new(loader)),
                    Err(error) => {
                        self.errors
                            .push(format!("Cannot start plugin loader: {error}"));
                        ApplyStage::Finished
                    }
                }
            };
        }
        if let ApplyStage::Loading(session) = &mut self.stage {
            let update = session.tick(host, executor);
            changed |= update.changed;
            if let Some(error) = update.error {
                self.errors.push(error);
            }
            if session.finished() {
                self.stage = ApplyStage::Finished;
            }
        }
        changed
    }
}
