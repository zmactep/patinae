//! Notifications for the native application.

use super::*;

impl App {
    pub(super) fn notify_short(&mut self, message: impl Into<String>) {
        self.transient_notification = Some((
            message.into(),
            Instant::now() + Duration::from_secs(TRANSIENT_NOTIFICATION_SECS),
        ));
    }

    pub(super) fn sync_notifications(&mut self, app: &AppWindow) {
        let tasks: Vec<_> = self
            .kernel
            .tasks
            .active_snapshots()
            .into_iter()
            .map(|snapshot| {
                let message = snapshot
                    .progress
                    .as_ref()
                    .map_or_else(String::new, |progress| progress.message.clone());
                crate::TaskNotification {
                    task_id: snapshot.id.to_string().into(),
                    text: if snapshot.cancel_requested {
                        format!("{} — cancelling...", message).into()
                    } else {
                        message.into()
                    },
                    cancellable: snapshot.cancellable && !snapshot.cancel_requested,
                }
            })
            .collect();
        let mut messages: Vec<String> = self
            .plugin_runtime
            .plugin_startup
            .status()
            .into_iter()
            .collect();
        if self.rendering.renderer_startup.is_some() {
            messages.push("Preparing graphics…".into());
        }
        if messages.is_empty() && tasks.is_empty() {
            if let Some((message, expires_at)) = &self.transient_notification {
                if Instant::now() <= *expires_at {
                    messages.push(message.clone());
                } else {
                    self.transient_notification = None;
                }
            }
        }

        let state = app.global::<NotificationState>();
        state.set_visible(!messages.is_empty() || !tasks.is_empty());
        state.set_text(messages.join("\n").into());
        // Avoid creating a new model (and redraw) on every idle timer tick.
        if !state.get_tasks().iter().eq(tasks.iter().cloned()) {
            state.set_tasks(Rc::new(slint::VecModel::from(tasks)).into());
        }
    }
}
