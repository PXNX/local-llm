use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
/// Children would inherit the GUI's lowered priority; flows, ComfyUI and Ollama run at normal priority.
#[cfg(windows)]
const NORMAL_PRIORITY_CLASS: u32 = 0x0000_0020;

pub fn hide_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW | NORMAL_PRIORITY_CLASS);
    cmd
}

pub fn new_console(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NEW_CONSOLE | NORMAL_PRIORITY_CLASS);
    cmd
}

/// The GUI yields CPU to the inference processes it starts.
pub fn lower_own_priority() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess, SetPriorityClass};
        SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS);
    }
}

/// Free bytes on the drive holding `path` (or its nearest existing parent).
pub fn free_space(path: &Path) -> Option<u64> {
    let mut p = path;
    while !p.exists() {
        p = p.parent()?;
    }
    free_space_of(p)
}

#[cfg(windows)]
fn free_space_of(p: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut avail = 0u64;
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(avail)
}

#[cfg(not(windows))]
fn free_space_of(_: &Path) -> Option<u64> {
    None
}

/// Persists a user environment variable (like `setx`), so tools started outside the GUI see it too.
pub fn setx(key: &str, value: &str) -> Result<(), String> {
    let mut cmd = Command::new("setx");
    cmd.args([key, value]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    match hide_window(&mut cmd).status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("setx failed ({s})")),
        Err(e) => Err(e.to_string()),
    }
}

/// Creates (or overwrites) `local-llm.lnk` on the user's Desktop, also when it is redirected (OneDrive).
pub fn create_desktop_shortcut(exe: &Path, work_dir: &Path) -> Result<PathBuf, String> {
    const SCRIPT: &str = "$ErrorActionPreference = 'Stop'; \
        $lnk = Join-Path ([Environment]::GetFolderPath('Desktop')) 'local-llm.lnk'; \
        $s = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk); \
        $s.TargetPath = $env:LLM_EXE; $s.WorkingDirectory = $env:LLM_DIR; $s.IconLocation = $env:LLM_EXE + ',0'; \
        $s.Description = 'local-llm'; $s.Save(); Write-Output $lnk";
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT])
        .env("LLM_EXE", exe)
        .env("LLM_DIR", work_dir)
        .stdin(std::process::Stdio::null());
    let out = hide_window(&mut cmd).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

/// Wall-clock "HH:MM:SS" for log lines, in the machine's local time zone.
#[cfg(windows)]
pub fn now_hms() -> String {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut t: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond)
}

#[cfg(not(windows))]
pub fn now_hms() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() % 86_400;
    format!("{:02}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
}

pub fn human(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else {
        format!("{:.0} KB", b / 1024.0)
    }
}

pub fn open(path: &Path) {
    let _ = open::that_detached(path);
}

pub fn open_url(url: &str) {
    let _ = open::that_detached(url);
}
