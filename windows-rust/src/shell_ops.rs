//! Thin wrappers over the Windows shell, matching `shell.openPath`,
//! `shell.showItemInFolder` and `shell.openExternal` from the Electron build.

#[cfg(windows)]
mod imp {
    use std::path::Path;
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn shell_execute(verb: &str, target: &str, params: Option<&str>) -> bool {
        let verb = HSTRING::from(verb);
        let target = HSTRING::from(target);
        let params = params.map(HSTRING::from);
        let result = unsafe {
            ShellExecuteW(
                None,
                PCWSTR(verb.as_ptr()),
                PCWSTR(target.as_ptr()),
                params
                    .as_ref()
                    .map(|p| PCWSTR(p.as_ptr()))
                    .unwrap_or(PCWSTR::null()),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        result.0 as isize > 32
    }

    pub fn open_path(path: &Path) -> bool {
        shell_execute("open", &path.to_string_lossy(), None)
    }

    pub fn open_external(url: &str) -> bool {
        shell_execute("open", url, None)
    }

    pub fn reveal_in_folder(path: &Path) -> bool {
        // explorer.exe /select,<path> highlights the file in its folder.
        std::process::Command::new("explorer.exe")
            .arg(format!("/select,{}", path.display()))
            .spawn()
            .is_ok()
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    pub fn open_path(_path: &Path) -> bool {
        false
    }
    pub fn open_external(_url: &str) -> bool {
        false
    }
    pub fn reveal_in_folder(path: &Path) -> bool {
        let _ = path;
        false
    }
}

pub use imp::{open_external, open_path, reveal_in_folder};
