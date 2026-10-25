//! Launch the Windows bundle with its private Python runtime.
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
// Rust guideline compliant 2026-02-21

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, ExitCode};

fn bundle_command(
    directory: &Path,
    arguments: impl IntoIterator<Item = OsString>,
    inherited_path: &OsStr,
) -> Result<Command, std::env::JoinPathsError> {
    let python = directory.join("python");
    let paths = std::iter::once(python.clone()).chain(std::env::split_paths(inherited_path));
    let mut command = Command::new(directory.join("patinae.exe"));
    command
        .args(arguments)
        .env("PATH", std::env::join_paths(paths)?)
        .env("PYTHONHOME", &python)
        .env("PYTHONPATH", python.join("Lib/site-packages"))
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env_remove("VIRTUAL_ENV");
    // Keep the caller's working directory so relative file arguments still work.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW hides the console of the CLI executable without
        // changing its subsystem or affecting its normal command-line use.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    Ok(command)
}

fn launch() -> Result<(), Box<dyn std::error::Error>> {
    let executable = std::env::current_exe()?;
    let directory = executable
        .parent()
        .ok_or("Cannot determine the Patinae installation directory")?;
    let mut command = bundle_command(
        directory,
        std::env::args_os().skip(1),
        &std::env::var_os("PATH").unwrap_or_default(),
    )?;
    command.spawn()?;
    Ok(())
}

#[cfg(windows)]
fn report_error(message: &str) {
    // This small standalone binary uses the system API without a Cargo dependency.
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(
            window: *mut std::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            flags: u32,
        ) -> i32;
    }
    let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let caption: Vec<u16> = "Patinae".encode_utf16().chain(Some(0)).collect();
    // SAFETY: Both strings are terminated UTF-16 buffers alive for the entire
    // synchronous call; a null owner is allowed. MB_ICONERROR needs no resources.
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), 0x10);
    }
}

#[cfg(not(windows))]
fn report_error(message: &str) {
    eprintln!("{message}");
}

fn main() -> ExitCode {
    match launch() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            report_error(&format!("Cannot start Patinae: {error}"));
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_arguments_and_sets_only_child_environment() {
        let directory = Path::new("installation with spaces/Патина");
        let arguments = vec![
            OsString::from("данные/my molecule.pdb"),
            OsString::from("quote\" and trailing\\"),
        ];
        let original_home = std::env::var_os("PYTHONHOME");
        let inherited =
            std::env::join_paths([Path::new("system tools"), Path::new("other tools")]).unwrap();
        let command = bundle_command(directory, arguments.clone(), &inherited).unwrap();
        assert_eq!(command.get_program(), directory.join("patinae.exe"));
        assert_eq!(command.get_args().collect::<Vec<_>>(), arguments);
        assert_eq!(command.get_current_dir(), None);
        let environment: std::collections::BTreeMap<_, _> = command.get_envs().collect();
        assert_eq!(
            environment[OsStr::new("PYTHONHOME")],
            Some(directory.join("python").as_os_str())
        );
        assert_eq!(environment[OsStr::new("VIRTUAL_ENV")], None);
        assert_eq!(std::env::var_os("PYTHONHOME"), original_home);
        let paths: Vec<_> =
            std::env::split_paths(environment[OsStr::new("PATH")].unwrap()).collect();
        assert_eq!(
            paths,
            vec![
                directory.join("python"),
                Path::new("system tools").to_owned(),
                Path::new("other tools").to_owned()
            ]
        );
    }
}
