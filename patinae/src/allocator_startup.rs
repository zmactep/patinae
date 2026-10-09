//! Applies macOS heap retention policy before loading application data.

use patinae_render::RenderMemoryProfile;

#[cfg(target_os = "macos")]
static LARGE_CACHE_DISABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Records the allocator environment before application initialization.
pub(crate) fn initialize() {
    #[cfg(target_os = "macos")]
    LARGE_CACHE_DISABLED.get_or_init(|| {
        std::env::var_os("MallocLargeCache").as_deref() == Some(std::ffi::OsStr::new("0"))
    });
}

/// Re-executes the app when the selected startup profile needs a smaller heap cache.
///
/// Called before creating the window, plugins, or session. Setting the current
/// process environment would be too late: libmalloc reads it before Rust main.
/// An explicit allocator override is preserved, including in performance mode.
#[cfg(not(target_os = "windows"))]
pub(crate) fn apply(profile: RenderMemoryProfile) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::process::CommandExt;

        if conserves_heap(profile) && std::env::var_os("MallocLargeCache").is_none() {
            log::info!("Restarting before window creation to reduce heap retention for {profile}");
            let error = std::process::Command::new(std::env::current_exe()?)
                .args(std::env::args_os().skip(1))
                .env("MallocLargeCache", "0")
                .exec();
            return Err(error);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = profile;
    Ok(())
}

/// Explains an allocator/profile mismatch after a runtime profile change.
pub(crate) fn restart_notice(profile: RenderMemoryProfile) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        initialize();
        notice(profile, *LARGE_CACHE_DISABLED.get().unwrap_or(&false))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = profile;
        None
    }
}

#[cfg(any(target_os = "macos", test))]
fn conserves_heap(profile: RenderMemoryProfile) -> bool {
    matches!(
        profile,
        RenderMemoryProfile::Balanced | RenderMemoryProfile::Lite
    )
}

#[cfg(any(target_os = "macos", test))]
fn notice(profile: RenderMemoryProfile, disabled: bool) -> Option<String> {
    if conserves_heap(profile) && !disabled {
        Some(format!(
            "To also reduce CPU heap retention, restart with PATINAE_RENDER_MEMORY_PROFILE={profile} and MallocLargeCache=0."
        ))
    } else if profile == RenderMemoryProfile::Performance && disabled {
        Some("CPU heap caching remains disabled until restart. Start with PATINAE_RENDER_MEMORY_PROFILE=performance and without MallocLargeCache to restore it.".into())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_changes_report_only_heap_policy_mismatches() {
        for profile in [RenderMemoryProfile::Balanced, RenderMemoryProfile::Lite] {
            assert!(notice(profile, false)
                .unwrap()
                .contains(&format!("PATINAE_RENDER_MEMORY_PROFILE={profile}")));
            assert!(notice(profile, true).is_none());
        }
        assert!(notice(RenderMemoryProfile::Performance, false).is_none());
        assert!(notice(RenderMemoryProfile::Performance, true)
            .unwrap()
            .contains("without MallocLargeCache"));
        for disabled in [true, false] {
            assert!(notice(RenderMemoryProfile::Manual { bytes: 1024 }, disabled).is_none());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reexec_preserves_pid_arguments_and_explicit_overrides() {
        use std::process::{Command, Stdio};
        for (profile, supplied, expected) in [
            ("balanced", None, "0"),
            ("lite", None, "0"),
            ("performance", None, "unset"),
            ("manual:1024", None, "unset"),
            ("balanced", Some("1"), "1"),
            ("performance", Some("0"), "0"),
        ] {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "allocator_startup::tests::reexec_child",
                    "--nocapture",
                ])
                .env("PATINAE_ALLOCATOR_TEST_PROFILE", profile)
                .env("PATINAE_ALLOCATOR_TEST_CWD", std::env::temp_dir())
                .env_remove("MallocLargeCache")
                .current_dir(std::env::temp_dir())
                .stdout(Stdio::piped());
            if let Some(value) = supplied {
                command.env("MallocLargeCache", value);
            }
            let child = command.spawn().unwrap();
            let pid = child.id();
            let output = child.wait_with_output().unwrap();
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(output.status.success(), "{profile}: {stdout}");
            assert!(
                stdout.contains(&format!("allocator-result:{pid}:{expected}")),
                "{profile}: {stdout}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reexec_child() {
        let Ok(profile) = std::env::var("PATINAE_ALLOCATOR_TEST_PROFILE") else {
            return;
        };
        initialize();
        apply(profile.parse().unwrap()).unwrap();
        let expected_cwd =
            std::path::PathBuf::from(std::env::var_os("PATINAE_ALLOCATOR_TEST_CWD").unwrap());
        assert_eq!(
            std::env::current_dir().unwrap(),
            expected_cwd.canonicalize().unwrap()
        );
        println!(
            "allocator-result:{}:{}",
            std::process::id(),
            std::env::var("MallocLargeCache").unwrap_or_else(|_| "unset".into())
        );
    }
}
