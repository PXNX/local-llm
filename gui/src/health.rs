//! CPU / RAM / GPU / VRAM usage, sampled on one background thread every 2 s.
//! GPU numbers come from NVML (nvml.dll ships with the NVIDIA driver): far cheaper than spawning nvidia-smi.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;

#[derive(Clone, PartialEq, Default)]
pub struct Sample {
    pub cpu: f32,
    pub ram_used: u64,
    pub ram_total: u64,
    pub gpu: Option<Gpu>,
}

#[derive(Clone, PartialEq)]
pub struct Gpu {
    pub name: String,
    /// e.g. "581.57"
    pub driver: String,
    /// highest CUDA version the driver supports, e.g. (13, 0)
    pub cuda: Option<(i32, i32)>,
    pub util: u32,
    pub vram_used: u64,
    pub vram_total: u64,
    pub temp: Option<u32>,
}

#[derive(Clone, Default)]
pub struct Health {
    latest: Arc<Mutex<Option<Sample>>>,
}

impl Health {
    pub fn spawn(ctx: egui::Context) -> Self {
        let h = Self::default();
        let latest = h.latest.clone();
        std::thread::Builder::new()
            .name("health".into())
            .spawn(move || {
                let nvml = nvml_wrapper::Nvml::init().ok();
                let device = nvml.as_ref().and_then(|n| n.device_by_index(0).ok());
                let name = device.as_ref().and_then(|d| d.name().ok()).unwrap_or_default();
                let driver = nvml.as_ref().and_then(|n| n.sys_driver_version().ok()).unwrap_or_default();
                let cuda = nvml.as_ref().and_then(|n| n.sys_cuda_driver_version().ok()).map(|v| (v / 1000, (v % 1000) / 10));
                let mut cpu = CpuTimes::now();
                let mut shown: Option<(u32, u64, u32, u64)> = None;
                loop {
                    std::thread::sleep(Duration::from_secs(2));
                    let (cpu_pct, next) = cpu.usage();
                    cpu = next;
                    let (ram_used, ram_total) = ram();
                    let gpu = device.as_ref().and_then(|d| {
                        let mem = d.memory_info().ok()?;
                        Some(Gpu {
                            name: name.clone(),
                            driver: driver.clone(),
                            cuda,
                            util: d.utilization_rates().map(|u| u.gpu).unwrap_or(0),
                            vram_used: mem.used,
                            vram_total: mem.total,
                            temp: d.temperature(nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu).ok(),
                        })
                    });
                    // repaint only when a displayed (rounded) value changed
                    let key = (
                        cpu_pct.round() as u32,
                        ram_used >> 26,
                        gpu.as_ref().map_or(0, |g| g.util),
                        gpu.as_ref().map_or(0, |g| g.vram_used >> 26),
                    );
                    *latest.lock().unwrap() = Some(Sample { cpu: cpu_pct, ram_used, ram_total, gpu });
                    if shown != Some(key) {
                        shown = Some(key);
                        ctx.request_repaint();
                    }
                }
            })
            .expect("spawn health sampler");
        h
    }

    pub fn latest(&self) -> Option<Sample> {
        self.latest.lock().unwrap().clone()
    }
}

/// ComfyUI portable (CUDA 12.6+) needs a driver of this major version or newer (see README).
pub const MIN_DRIVER: u32 = 580;

pub fn driver_ok(driver: &str) -> Option<bool> {
    driver.split('.').next()?.parse::<u32>().ok().map(|major| major >= MIN_DRIVER)
}

struct CpuTimes {
    idle: u64,
    total: u64,
}

impl CpuTimes {
    #[cfg(windows)]
    fn now() -> Self {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::GetSystemTimes;
        let mut t = [FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 }; 3];
        let [idle, kernel, user] = &mut t;
        unsafe { GetSystemTimes(idle, kernel, user) };
        let v = |f: &FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
        // kernel time includes idle time
        Self { idle: v(&t[0]), total: v(&t[1]) + v(&t[2]) }
    }

    #[cfg(not(windows))]
    fn now() -> Self {
        Self { idle: 0, total: 0 }
    }

    fn usage(&self) -> (f32, Self) {
        let now = Self::now();
        let total = now.total.saturating_sub(self.total);
        let idle = now.idle.saturating_sub(self.idle);
        let pct = if total == 0 { 0.0 } else { 100.0 * (1.0 - idle as f32 / total as f32) };
        (pct.clamp(0.0, 100.0), now)
    }
}

#[cfg(windows)]
fn ram() -> (u64, u64) {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 {
        return (0, 0);
    }
    (m.ullTotalPhys - m.ullAvailPhys, m.ullTotalPhys)
}

#[cfg(not(windows))]
fn ram() -> (u64, u64) {
    (0, 0)
}
