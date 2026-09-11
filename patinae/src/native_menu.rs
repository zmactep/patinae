use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;

use patinae_cmd::tasks::{TaskId, TaskOutcomeStatus};
use patinae_framework::message::AppMessage;

use crate::native_file_actions::quote_command_arg;
use crate::{AppWindow, LayoutState, MenuState, StartState};

const PDB_ID_DISPLAY_CHARS: usize = 4;

pub fn setup_callbacks(app: Rc<RefCell<crate::app::App>>, window: &AppWindow) {
    let menu = window.global::<MenuState>();

    {
        let app = app.clone();
        let weak = window.as_weak();
        menu.on_action(move |action| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            dispatch_menu_action(app.clone(), &window, action.as_str());
        });
    }

    {
        let app = app.clone();
        let weak = window.as_weak();
        menu.on_open_recent(move |path| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            app.borrow_mut().open_recent_file(&window, path.as_str());
        });
    }

    {
        let weak = window.as_weak();
        menu.on_fetch_pdb_edited(move |pdb_id| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            normalize_fetch_pdb_edit(&window, pdb_id.as_str());
        });
    }

    {
        let app = app.clone();
        let weak = window.as_weak();
        menu.on_fetch_preview(move |pdb_id| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            request_fetch_preview(app.clone(), &window, pdb_id.as_str());
        });
    }

    {
        let app = app.clone();
        let weak = window.as_weak();
        menu.on_fetch_submit(move |pdb_id, object_name, format| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            submit_fetch(
                app.clone(),
                &window,
                pdb_id.as_str(),
                object_name.as_str(),
                format.as_str(),
            );
        });
    }

    {
        let app = app.clone();
        let weak = window.as_weak();
        menu.on_fetch_open_doi(move |doi| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            open_doi_link(app.clone(), &window, doi.as_str());
        });
    }

    let start = window.global::<StartState>();

    {
        let weak = window.as_weak();
        start.on_fetch_pdb_edited(move |pdb_id| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            normalize_start_fetch_edit(&window, pdb_id.as_str());
        });
    }

    {
        let app = app.clone();
        let weak = window.as_weak();
        start.on_fetch_submit(move |pdb_id| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            submit_start_fetch(app.clone(), &window, pdb_id.as_str());
        });
    }

    {
        let app = app.clone();
        let weak = window.as_weak();
        start.on_open_example(move |pdb_id| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            submit_start_fetch(app.clone(), &window, pdb_id.as_str());
        });
    }
}

pub(crate) fn sync_fetch_preview(
    kernel: &patinae_framework::kernel::AppKernel,
    window: &AppWindow,
) {
    let menu = window.global::<MenuState>();
    if !menu.get_fetch_loading() {
        return;
    }
    let Ok(id) = menu.get_fetch_task_id().as_str().parse::<TaskId>() else {
        return;
    };
    let outcome = match kernel.tasks.get(id) {
        Ok(snapshot) => match snapshot.outcome {
            Some(outcome) => outcome,
            None => return,
        },
        Err(error) => {
            menu.set_fetch_loading(false);
            menu.set_fetch_status(format!("Metadata lookup unavailable: {error}").into());
            return;
        }
    };

    menu.set_fetch_loading(false);
    let result = match outcome.status {
        TaskOutcomeStatus::Success { data: Some(data) } => {
            serde_json::from_value::<crate::fetch::PdbMetadataMessage>(data.payload)
                .map_err(|e| e.to_string())
        }
        TaskOutcomeStatus::Failure { error } => Err(error.message),
        TaskOutcomeStatus::Cancelled { reason } => Err(reason),
        _ => Err("missing metadata result".into()),
    };
    if let Ok(message) = result {
        menu.set_fetch_status(String::new().into());
        menu.set_fetch_title(message.title.into());
        menu.set_fetch_details(message.details.into());
        menu.set_fetch_method(message.method.into());
        menu.set_fetch_deposit_date(message.deposit_date.into());
        menu.set_fetch_release_date(message.release_date.into());
        menu.set_fetch_doi(message.doi.into());
    } else if let Err(error) = result {
        clear_fetch_metadata(&menu);
        menu.set_fetch_status(format!("Metadata lookup failed: {error}").into());
    }
}

fn dispatch_menu_action(app: Rc<RefCell<crate::app::App>>, window: &AppWindow, action: &str) {
    if let Some(panel_id) = action.strip_prefix("panel:") {
        app.borrow_mut()
            .kernel
            .bus
            .send(AppMessage::TogglePanel(panel_id.to_string()));
        window.window().request_redraw();
        return;
    }

    match action {
        "run-script" => menu_run_script(app, window),
        "open" => menu_open_file(app, window),
        "fetch" => open_fetch_dialog(window),
        "save" => menu_save_file(app, "Save", "session.prs", "save"),
        "export-png" => menu_save_file(app, "Export PNG", "screenshot.png", "png"),
        "export-movie" => menu_save_file(app, "Export Movie", "movie.mp4", "mproduce"),
        "focus-repl" => focus_repl(app, window),
        "select-all" => enqueue_command(app, "select sele, all"),
        "deselect-all" => enqueue_command(app, "deselect"),
        "reset-view" => enqueue_command(app, "reset"),
        "zoom-all" => enqueue_command(app, "zoom"),
        "orient" => enqueue_command(app, "orient"),
        "center" => enqueue_command(app, "center"),
        "toggle-opaque-background" => toggle_opaque_background(app),
        "bg-white" => enqueue_command(app, "bg_color white"),
        "bg-black" => enqueue_command(app, "bg_color black"),
        "fullscreen" => enqueue_command(app, "full_screen"),
        "help" => enqueue_command(app, "help"),
        "quit" => enqueue_command(app, "quit"),
        other => {
            app.borrow_mut()
                .kernel
                .bus
                .print_warning(format!("Unknown menu action: {other}"));
        }
    }
}

fn enqueue_command(app: Rc<RefCell<crate::app::App>>, command: impl Into<String>) {
    app.borrow_mut().kernel.bus.execute_command(command.into());
}

fn focus_repl(app: Rc<RefCell<crate::app::App>>, window: &AppWindow) {
    if window.global::<LayoutState>().get_empty_mode() {
        return;
    }

    app.borrow_mut()
        .kernel
        .bus
        .send(AppMessage::ShowPanel("repl".to_string()));
    window.window().request_redraw();
}

fn toggle_opaque_background(app: Rc<RefCell<crate::app::App>>) {
    let current = app.borrow().kernel.session.settings.ui.opaque_background;
    let next = if current { "off" } else { "on" };
    enqueue_command(app, format!("set opaque_background, {next}"));
}

fn open_fetch_dialog(window: &AppWindow) {
    let menu = window.global::<MenuState>();
    menu.set_fetch_task_id(String::new().into());
    menu.set_fetch_pdb_id(String::new().into());
    menu.set_fetch_object_name(String::new().into());
    menu.set_fetch_format("cif".into());
    menu.set_fetch_loading(false);
    menu.set_fetch_status(String::new().into());
    clear_fetch_metadata(&menu);
    menu.set_fetch_visible(true);
}

fn request_fetch_preview(app: Rc<RefCell<crate::app::App>>, window: &AppWindow, pdb_id: &str) {
    let normalized = normalize_pdb_id(pdb_id);
    let menu = window.global::<MenuState>();
    menu.set_fetch_task_id(String::new().into());
    if let Err(err) = patinae_io::fetch::validate_pdb_id(&normalized) {
        menu.set_fetch_loading(false);
        clear_fetch_metadata(&menu);
        menu.set_fetch_status(err.to_string().into());
        return;
    }

    menu.set_fetch_pdb_id(normalized.to_ascii_uppercase().into());
    menu.set_fetch_loading(true);
    clear_fetch_metadata(&menu);
    menu.set_fetch_status(String::new().into());
    let admission = app
        .borrow()
        .kernel
        .spawn_task(crate::fetch::PdbMetadataTask::new(normalized));
    match admission {
        Ok(id) => menu.set_fetch_task_id(id.to_string().into()),
        Err(error) => {
            menu.set_fetch_loading(false);
            menu.set_fetch_status(format!("Metadata lookup unavailable: {error}").into());
        }
    }
    window.window().request_redraw();
}

fn normalize_fetch_pdb_edit(window: &AppWindow, pdb_id: &str) {
    let menu = window.global::<MenuState>();
    menu.set_fetch_task_id(String::new().into());
    let normalized = normalize_pdb_id_for_display(pdb_id);
    if normalized != menu.get_fetch_pdb_id().as_str() {
        menu.set_fetch_pdb_id(normalized.into());
    }
    menu.set_fetch_loading(false);
    menu.set_fetch_status(String::new().into());
    clear_fetch_metadata(&menu);
    window.window().request_redraw();
}

fn normalize_start_fetch_edit(window: &AppWindow, pdb_id: &str) {
    let start = window.global::<StartState>();
    let normalized = normalize_pdb_id_for_display(pdb_id);
    if normalized != start.get_fetch_pdb_id().as_str() {
        start.set_fetch_pdb_id(normalized.clone().into());
    }

    let command_id = normalize_pdb_id(&normalized);
    start.set_fetch_pdb_valid(patinae_io::fetch::validate_pdb_id(&command_id).is_ok());
    start.set_fetch_status(String::new().into());
    window.window().request_redraw();
}

fn open_doi_link(app: Rc<RefCell<crate::app::App>>, window: &AppWindow, doi: &str) {
    let Some(url) = doi_url(doi) else {
        app.borrow_mut()
            .kernel
            .bus
            .print_warning("No DOI is available for this PDB entry.");
        return;
    };

    match webbrowser::open(&url) {
        Ok(_) => app.borrow_mut().notify_short("Opened DOI in browser"),
        Err(err) => app
            .borrow_mut()
            .kernel
            .bus
            .print_warning(format!("Could not open DOI link: {err}")),
    }
    window.window().request_redraw();
}

fn submit_start_fetch(app: Rc<RefCell<crate::app::App>>, window: &AppWindow, pdb_id: &str) {
    let start = window.global::<StartState>();
    match build_start_fetch_command(pdb_id) {
        Ok(command) => {
            start.set_fetch_status(String::new().into());
            app.borrow_mut().kernel.bus.execute_command(command);
            window.window().request_redraw();
        }
        Err(error) => {
            start.set_fetch_pdb_valid(false);
            start.set_fetch_status(error.into());
            window.window().request_redraw();
        }
    }
}

fn submit_fetch(
    app: Rc<RefCell<crate::app::App>>,
    window: &AppWindow,
    pdb_id: &str,
    object_name: &str,
    format: &str,
) {
    let menu = window.global::<MenuState>();
    match build_fetch_command(pdb_id, object_name, format) {
        Ok(command) => {
            menu.set_fetch_visible(false);
            app.borrow_mut().kernel.bus.execute_command(command);
            window.window().request_redraw();
        }
        Err(error) => {
            menu.set_fetch_loading(false);
            clear_fetch_metadata(&menu);
            menu.set_fetch_status(error.into());
        }
    }
}

fn clear_fetch_metadata(menu: &MenuState) {
    menu.set_fetch_title(String::new().into());
    menu.set_fetch_details(String::new().into());
    menu.set_fetch_method(String::new().into());
    menu.set_fetch_deposit_date(String::new().into());
    menu.set_fetch_release_date(String::new().into());
    menu.set_fetch_doi(String::new().into());
}

fn menu_open_file(app: Rc<RefCell<crate::app::App>>, _window: &AppWindow) {
    #[cfg(target_os = "macos")]
    {
        if let Some(path) = crate::macos::open_file_path("Open") {
            app.borrow_mut().route_menu_open_file(path);
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        app.borrow_mut()
            .kernel
            .bus
            .print_warning("Open dialog is unavailable on this platform; use load in the REPL.");
    }
}

fn menu_run_script(app: Rc<RefCell<crate::app::App>>, _window: &AppWindow) {
    #[cfg(target_os = "macos")]
    {
        if let Some(path) = crate::macos::open_file_path("Run Script") {
            app.borrow_mut().route_menu_run_script(path);
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        app.borrow_mut().kernel.bus.print_warning(
            "Run Script dialog is unavailable on this platform; use run in the REPL.",
        );
    }
}

fn menu_save_file(
    app: Rc<RefCell<crate::app::App>>,
    title: &str,
    default_name: &str,
    command: &str,
) {
    #[cfg(not(target_os = "macos"))]
    let _ = default_name;

    #[cfg(target_os = "macos")]
    {
        if let Some(path) = crate::macos::save_file_path_with_default(title, default_name) {
            let path_text = path.to_string_lossy();
            let path = quote_command_arg(path_text.as_ref());
            app.borrow_mut()
                .kernel
                .bus
                .execute_command(format!("{command} {path}"));
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        app.borrow_mut().kernel.bus.print_warning(format!(
            "{title} dialog is unavailable on this platform; use {command} in the REPL."
        ));
    }
}

pub(crate) fn build_fetch_command(
    pdb_id: &str,
    object_name: &str,
    format: &str,
) -> Result<String, String> {
    let pdb_id = normalize_pdb_id(pdb_id);
    patinae_io::fetch::validate_pdb_id(&pdb_id).map_err(|err| err.to_string())?;

    let format = normalize_fetch_format(format)?;
    let object_name = object_name.trim();

    let mut command = format!("fetch {pdb_id}");
    if !object_name.is_empty() {
        command.push_str(", name=");
        command.push_str(&quote_command_arg(object_name));
    }
    command.push_str(", type=");
    command.push_str(format);
    Ok(command)
}

fn build_start_fetch_command(pdb_id: &str) -> Result<String, String> {
    build_fetch_command(pdb_id, "", "cif")
}

fn normalize_pdb_id(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_pdb_id_for_display(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .take(PDB_ID_DISPLAY_CHARS)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn doi_url(doi: &str) -> Option<String> {
    let doi = doi.trim();
    if doi.is_empty() {
        return None;
    }

    let doi = doi
        .strip_prefix("https://doi.org/")
        .or_else(|| doi.strip_prefix("http://doi.org/"))
        .unwrap_or(doi);
    if doi.is_empty() {
        None
    } else {
        Some(format!("https://doi.org/{doi}"))
    }
}

fn normalize_fetch_format(value: &str) -> Result<&'static str, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "pdb" => Ok("pdb"),
        "cif" | "mmcif" => Ok("cif"),
        "bcif" | "binarycif" => Ok("bcif"),
        other => Err(format!("Unsupported fetch format: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_command_normalizes_pdb_id_and_format() {
        assert_eq!(
            build_fetch_command(" 1UBQ ", "", "mmcif").unwrap(),
            "fetch 1ubq, type=cif"
        );
    }

    #[test]
    fn fetch_command_includes_optional_object_name() {
        assert_eq!(
            build_fetch_command("4HHB", "hemoglobin", "pdb").unwrap(),
            "fetch 4hhb, name=hemoglobin, type=pdb"
        );
    }

    #[test]
    fn fetch_command_quotes_object_name_when_needed() {
        assert_eq!(
            build_fetch_command("1CRN", "crambin model", "bcif").unwrap(),
            "fetch 1crn, name=\"crambin model\", type=bcif"
        );
    }

    #[test]
    fn fetch_command_rejects_invalid_pdb_id() {
        let error = build_fetch_command("abcde", "", "cif").unwrap_err();
        assert!(error.contains("must be exactly 4 characters"));
    }

    #[test]
    fn fetch_command_rejects_unknown_format() {
        let error = build_fetch_command("1ubq", "", "xml").unwrap_err();
        assert_eq!(error, "Unsupported fetch format: xml");
    }

    #[test]
    fn start_fetch_command_uses_default_cif_fetch() {
        assert_eq!(
            build_start_fetch_command(" 1IGT ").unwrap(),
            "fetch 1igt, type=cif"
        );
    }

    #[test]
    fn pdb_id_display_normalization_uppercases_and_truncates() {
        assert_eq!(normalize_pdb_id_for_display(" 1fsd5 "), "1FSD");
    }

    #[test]
    fn doi_url_targets_doi_org() {
        assert_eq!(
            doi_url("10.1126/science.278.5335.82").as_deref(),
            Some("https://doi.org/10.1126/science.278.5335.82")
        );
        assert_eq!(
            doi_url("https://doi.org/10.1126/science.278.5335.82").as_deref(),
            Some("https://doi.org/10.1126/science.278.5335.82")
        );
    }
}
