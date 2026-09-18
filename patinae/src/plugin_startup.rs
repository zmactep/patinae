//! First-frame gate and host-thread attachment for native plugin startup.

use std::time::Instant;

use patinae_cmd::CommandExecutor;
use patinae_plugin_host::{BackgroundPluginLoader, PluginDiscovery, PluginHost, PluginLoadEvent};

enum Stage {
    WaitingForFrame,
    Scheduled,
    Loading(BackgroundPluginLoader),
    Ready,
    Cancelled,
}

pub(crate) struct PluginStartup {
    stage: Stage,
    started: Instant,
    processed: usize,
    total: Option<usize>,
}

impl PluginStartup {
    pub(crate) fn new(started: Instant) -> Self {
        Self {
            stage: Stage::WaitingForFrame,
            started,
            processed: 0,
            total: None,
        }
    }

    pub(crate) fn after_frame(&mut self) {
        if matches!(self.stage, Stage::WaitingForFrame) {
            log::info!(
                "Startup first frame: {:.3} ms",
                self.started.elapsed().as_secs_f64() * 1000.0
            );
            self.stage = Stage::Scheduled;
        }
    }

    pub(crate) fn ready(&self) -> bool {
        matches!(self.stage, Stage::Ready)
    }

    pub(crate) fn status(&self) -> Option<String> {
        match self.stage {
            Stage::Ready | Stage::Cancelled => None,
            _ => Some(match self.total {
                Some(total) => format!("Loading plugins: {} / {total}", self.processed),
                None => "Loading plugins…".into(),
            }),
        }
    }

    pub(crate) fn cancel(&mut self) {
        self.stage = Stage::Cancelled;
    }

    // Exactly one event per host tick, hence at most one plugin attachment.
    pub(crate) fn tick(
        &mut self,
        host: &mut PluginHost,
        executor: &mut CommandExecutor,
    ) -> Result<bool, String> {
        self.tick_with_discovery(host, executor, PluginDiscovery::from_process_env)
    }

    fn tick_with_discovery(
        &mut self,
        host: &mut PluginHost,
        executor: &mut CommandExecutor,
        discovery: impl FnOnce() -> PluginDiscovery,
    ) -> Result<bool, String> {
        if matches!(self.stage, Stage::Scheduled) {
            match BackgroundPluginLoader::start(discovery()) {
                Ok(loader) => self.stage = Stage::Loading(loader),
                Err(error) => {
                    self.finish();
                    return Err(format!("Cannot start plugin loader: {error}"));
                }
            }
            return Ok(false);
        }
        let Stage::Loading(loader) = &self.stage else {
            return Ok(false);
        };
        match loader.try_next() {
            Ok(None) => Ok(false),
            Ok(Some(PluginLoadEvent::Discovered { directories, total })) => {
                for directory in directories {
                    host.add_plugin_dir(&directory);
                }
                self.total = Some(total);
                Ok(false)
            }
            Ok(Some(PluginLoadEvent::Plugin {
                path,
                result,
                preparation_time,
            })) => {
                let start = Instant::now();
                let attached =
                    result.and_then(|prepared| host.attach_prepared(*prepared, executor));
                loader.acknowledge();
                self.processed += 1;
                let load_time = preparation_time + start.elapsed();
                match attached {
                    Ok(name) => {
                        log::info!(
                            "Loaded plugin: {name} ({:.3} ms)",
                            load_time.as_secs_f64() * 1000.0
                        );
                        Ok(true)
                    }
                    Err(error) => Err(format!("Failed to load plugin {}: {error}", path.display())),
                }
            }
            Ok(Some(PluginLoadEvent::DiscoveryError { error })) => {
                Err(format!("Plugin discovery failed: {error}"))
            }
            Ok(Some(PluginLoadEvent::Finished)) => {
                self.finish();
                Ok(false)
            }
            Err(error) => {
                self.finish();
                Err(error)
            }
        }
    }

    pub(super) fn finish(&mut self) {
        log::info!(
            "Startup plugins ready: {:.3} ms ({} attempted)",
            self.started.elapsed().as_secs_f64() * 1000.0,
            self.processed
        );
        self.stage = Stage::Ready;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_settings::paths::{PathResolver, PathResolverInput};
    use std::time::Duration;

    #[test]
    fn first_frame_schedules_once_and_empty_discovery_completes() {
        let mut startup = PluginStartup::new(Instant::now());
        let mut host = PluginHost::new();
        let mut executor = CommandExecutor::new();
        startup
            .tick_with_discovery(&mut host, &mut executor, || {
                panic!("discovery before frame")
            })
            .unwrap();
        assert!(!startup.ready());
        startup.after_frame();
        startup.after_frame();
        let missing =
            std::env::temp_dir().join(format!("patinae-no-plugins-{}", std::process::id()));
        assert!(!missing.exists());
        startup
            .tick_with_discovery(&mut host, &mut executor, || {
                PluginDiscovery::new(PathResolver::new(PathResolverInput {
                    plugin_dir: Some(missing),
                    ..Default::default()
                }))
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !startup.ready() {
            startup.after_frame();
            startup
                .tick_with_discovery(&mut host, &mut executor, || panic!("repeated discovery"))
                .unwrap();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(startup.total, Some(0));
        assert!(startup.status().is_none());
        startup.after_frame();
        startup
            .tick_with_discovery(&mut host, &mut executor, || panic!("restart after ready"))
            .unwrap();
    }

    #[test]
    fn cancelled_startup_cannot_be_restarted_by_a_frame_or_tick() {
        let mut startup = PluginStartup::new(Instant::now());
        startup.cancel();
        startup.after_frame();
        startup
            .tick_with_discovery(&mut PluginHost::new(), &mut CommandExecutor::new(), || {
                panic!("cancelled discovery")
            })
            .unwrap();
        assert!(!startup.ready());
        assert!(startup.status().is_none());
    }

    #[test]
    fn invalid_manifest_reports_error_and_completes_startup() {
        let root = std::env::temp_dir().join(format!(
            "patinae-startup-invalid-manifest-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let manifest = root.join("plugins.toml");
        std::fs::write(&manifest, "[[plugins]]\npath = 'wrong'").unwrap();
        let mut startup = PluginStartup::new(Instant::now());
        let mut host = PluginHost::new();
        let mut executor = CommandExecutor::new();
        startup.after_frame();
        startup
            .tick_with_discovery(&mut host, &mut executor, || {
                PluginDiscovery::new(PathResolver::new(PathResolverInput {
                    plugin_dir: Some(root.clone()),
                    ..Default::default()
                }))
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut errors = Vec::new();
        while !startup.ready() {
            if let Err(error) = startup
                .tick_with_discovery(&mut host, &mut executor, || panic!("repeated discovery"))
            {
                errors.push(error);
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains(&manifest.display().to_string()));
        assert_eq!(startup.total, Some(0));
        assert_eq!(host.plugin_count(), 0);
        assert!(startup.status().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
