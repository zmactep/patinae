//! IPC plugin.
//!
//! Provides a Unix domain socket IPC server for external control of
//! the application (e.g., from patinae-python).
//!
//! Activated by setting the `PATINAE_IPC_SOCKET` environment variable
//! to the desired socket path.

mod handler;
mod protocol;
mod server;
use patinae_plugin::patinae_plugin;

use handler::IpcMessageHandler;
use server::IpcServer;

patinae_plugin! {
    name: "ipc",
    description: "IPC server for external control via Unix domain socket",
    commands: [],
    register: |reg| {
        if let Ok(socket_path) = std::env::var("PATINAE_IPC_SOCKET") {
            let path = std::path::PathBuf::from(&socket_path);
            match IpcServer::bind(&path) {
                Ok(server) => {
                    log::info!("IPC plugin: server bound to {:?}", socket_path);
                    reg.set_message_handler(IpcMessageHandler::new(server));
                }
                Err(e) => {
                    log::warn!("IPC plugin: failed to bind server: {}", e);
                }
            }
        } else {
            log::info!("IPC plugin: PATINAE_IPC_SOCKET not set, server disabled");
        }
    },
}

#[cfg(test)]
mod tests {
    //! Shared deterministic IPC test fixtures.

    #[cfg(unix)]
    pub(crate) fn socket_path(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_PATH: AtomicU64 = AtomicU64::new(1);
        // A short /tmp path stays within Unix socket path limits on macOS.
        std::path::PathBuf::from(format!(
            "/tmp/patinae-ipc-{}-{}-{label}.sock",
            std::process::id(),
            NEXT_PATH.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
