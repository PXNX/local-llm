//! Progress of the running flow on the window's taskbar button (ITaskbarList3), and the sounds played
//! when a queued run ends or the whole queue is through.

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Progress {
    None,
    /// Running, progress unknown.
    Busy,
    Value(f32),
    /// Queue on hold.
    Paused,
}

#[cfg(windows)]
pub use win::Taskbar;

#[cfg(not(windows))]
pub struct Taskbar;

#[cfg(not(windows))]
impl Taskbar {
    pub fn new(_: &eframe::CreationContext<'_>) -> Self {
        Self
    }
    pub fn set(&mut self, _: Progress) {}
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
    use windows_sys::core::{GUID, HRESULT};

    use super::Progress;

    const CLSID_TASKBAR_LIST: GUID = GUID::from_u128(0x56fdf344_fd6d_11d0_958a_006097c9a090);
    const IID_ITASKBAR_LIST3: GUID = GUID::from_u128(0xea1afb91_9e28_4b86_90e9_9e9f8a5eefaf);
    const TBPF_NOPROGRESS: u32 = 0;
    const TBPF_INDETERMINATE: u32 = 1;
    const TBPF_NORMAL: u32 = 2;
    const TBPF_PAUSED: u32 = 8;

    /// The ITaskbarList3 vtable up to SetProgressState (IUnknown, ITaskbarList, ITaskbarList2 first).
    #[repr(C)]
    struct Vtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        hr_init: unsafe extern "system" fn(*mut c_void) -> HRESULT,
        add_tab: usize,
        delete_tab: usize,
        activate_tab: usize,
        set_active_alt: usize,
        mark_fullscreen_window: usize,
        set_progress_value: unsafe extern "system" fn(*mut c_void, isize, u64, u64) -> HRESULT,
        set_progress_state: unsafe extern "system" fn(*mut c_void, isize, u32) -> HRESULT,
    }

    pub struct Taskbar {
        hwnd: isize,
        list: *mut c_void,
        shown: Progress,
    }

    impl Taskbar {
        pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
            let hwnd = match cc.window_handle().map(|h| h.as_raw()) {
                Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
                _ => 0,
            };
            let mut list = std::ptr::null_mut();
            if hwnd != 0 {
                unsafe {
                    // winit has initialized OLE on this thread already; this is then a no-op
                    CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
                    let hr = CoCreateInstance(&CLSID_TASKBAR_LIST, std::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_ITASKBAR_LIST3, &mut list);
                    if hr < 0 || list.is_null() || ((*vtbl(list)).hr_init)(list) < 0 {
                        list = std::ptr::null_mut();
                    }
                }
            }
            Self { hwnd, list, shown: Progress::None }
        }

        /// Updates the taskbar button; cheap when nothing changed (call every frame).
        pub fn set(&mut self, p: Progress) {
            // whole percents: the button is a few pixels wide, no need to call COM every frame
            let p = match p {
                Progress::Value(f) => Progress::Value((f.clamp(0.0, 1.0) * 100.0).round()),
                p => p,
            };
            if self.list.is_null() || p == self.shown {
                return;
            }
            self.shown = p;
            let (state, value) = match p {
                Progress::None => (TBPF_NOPROGRESS, None),
                Progress::Busy => (TBPF_INDETERMINATE, None),
                Progress::Value(pct) => (TBPF_NORMAL, Some(pct as u64)),
                Progress::Paused => (TBPF_PAUSED, Some(100)),
            };
            unsafe {
                let v = vtbl(self.list);
                ((*v).set_progress_state)(self.list, self.hwnd, state);
                if let Some(value) = value {
                    ((*v).set_progress_value)(self.list, self.hwnd, value, 100);
                }
            }
        }
    }

    impl Drop for Taskbar {
        fn drop(&mut self) {
            if !self.list.is_null() {
                unsafe { ((*vtbl(self.list)).release)(self.list) };
            }
        }
    }

    unsafe fn vtbl(obj: *mut c_void) -> *const Vtbl {
        unsafe { *(obj as *const *const Vtbl) }
    }
}

/// Short sounds, synthesized once and played asynchronously from memory.
pub mod sound {
    #[derive(Clone, Copy)]
    pub enum Cue {
        /// One queued run finished, more are waiting.
        RunDone,
        /// A run failed.
        RunFailed,
        /// Nothing left in the queue.
        QueueDone,
    }

    const SR: u32 = 44100;

    /// (midi note, start s, length s) per note: a soft bell-like tone each.
    fn notes(cue: Cue) -> &'static [(f32, f32, f32)] {
        match cue {
            // a quick rising ding-ding
            Cue::RunDone => &[(81.0, 0.0, 0.35), (88.0, 0.09, 0.5)],
            // two low falling notes
            Cue::RunFailed => &[(64.0, 0.0, 0.3), (58.0, 0.16, 0.55)],
            // a little fanfare: major arpeggio up to a held chord
            Cue::QueueDone => &[(72.0, 0.0, 0.3), (76.0, 0.12, 0.3), (79.0, 0.24, 0.3), (84.0, 0.36, 1.1), (88.0, 0.36, 1.1), (91.0, 0.36, 1.1)],
        }
    }

    /// 16-bit mono WAV file bytes.
    fn wav(cue: Cue) -> Vec<u8> {
        let notes = notes(cue);
        let len = notes.iter().map(|&(_, at, d)| at + d).fold(0.0, f32::max) + 0.05;
        let n = (len * SR as f32) as usize;
        let mut buf = vec![0f32; n];
        for &(midi, at, dur) in notes {
            let hz = 440.0 * 2f32.powf((midi - 69.0) / 12.0);
            let start = (at * SR as f32) as usize;
            for i in 0..(dur * SR as f32) as usize {
                let t = i as f32 / SR as f32;
                let env = (t / 0.004).min(1.0) * (-t * 5.0 / dur).exp() * ((dur - t) / 0.03).clamp(0.0, 1.0);
                let tau = std::f32::consts::TAU;
                let s = (tau * hz * t).sin() + 0.3 * (tau * 2.0 * hz * t).sin() * (-t * 9.0).exp() + 0.12 * (tau * 3.0 * hz * t).sin();
                if let Some(b) = buf.get_mut(start + i) {
                    *b += s * env;
                }
            }
        }
        let peak = buf.iter().fold(0f32, |m, s| m.max(s.abs())).max(1e-6);
        let data: Vec<u8> = buf.iter().flat_map(|s| ((s / peak * 0.45 * 32767.0) as i16).to_le_bytes()).collect();
        let mut out = Vec::with_capacity(44 + data.len());
        out.extend(b"RIFF");
        out.extend((36 + data.len() as u32).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend(1u16.to_le_bytes()); // PCM
        out.extend(1u16.to_le_bytes()); // mono
        out.extend(SR.to_le_bytes());
        out.extend((SR * 2).to_le_bytes());
        out.extend(2u16.to_le_bytes());
        out.extend(16u16.to_le_bytes());
        out.extend(b"data");
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }

    pub fn play(cue: Cue) {
        #[cfg(windows)]
        {
            use std::sync::OnceLock;
            use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
            // PlaySound reads the bytes while it plays asynchronously, so they live forever
            static SOUNDS: [OnceLock<Vec<u8>>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
            let bytes = SOUNDS[cue as usize].get_or_init(|| wav(cue));
            unsafe { PlaySoundW(bytes.as_ptr().cast(), std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT) };
        }
        #[cfg(not(windows))]
        let _ = wav(cue);
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn wav_header_matches_data() {
            for cue in [Cue::RunDone, Cue::RunFailed, Cue::QueueDone] {
                let w = wav(cue);
                assert_eq!(&w[..4], b"RIFF");
                assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize, w.len() - 8);
                assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize, w.len() - 44);
            }
        }
    }
}
