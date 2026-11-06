//! Audio output.
//!
//! On Linux this talks to ALSA (which PipeWire and PulseAudio also serve) by
//! loading `libasound.so.2` at runtime, so building needs no system headers.
//! If no device can be opened the emulator simply runs silently.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

pub const OUTPUT_RATE: u32 = 48000;

struct Shared {
    buffer: Mutex<VecDeque<[i16; 2]>>,
    running: AtomicBool,
    /// Volume in 1/256 units.
    volume: AtomicU32,
}

pub struct AudioOut {
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub rate: u32,
}

impl AudioOut {
    pub fn new() -> Option<AudioOut> {
        let shared = Arc::new(Shared {
            buffer: Mutex::new(VecDeque::with_capacity(16384)),
            running: AtomicBool::new(true),
            volume: AtomicU32::new(256),
        });
        let device = backend::Device::open(OUTPUT_RATE)?;
        let s = shared.clone();
        let thread = std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || {
                let mut device = device;
                let mut chunk = vec![[0i16; 2]; 512];
                while s.running.load(Ordering::Relaxed) {
                    {
                        let mut buf = s.buffer.lock().unwrap();
                        let vol = s.volume.load(Ordering::Relaxed) as i32;
                        for slot in chunk.iter_mut() {
                            let frame = buf.pop_front().unwrap_or([0, 0]);
                            *slot = [
                                ((frame[0] as i32 * vol) >> 8).clamp(-32768, 32767) as i16,
                                ((frame[1] as i32 * vol) >> 8).clamp(-32768, 32767) as i16,
                            ];
                        }
                    }
                    if !device.write(&chunk) {
                        break;
                    }
                }
            })
            .ok()?;
        Some(AudioOut { shared, thread: Some(thread), rate: OUTPUT_RATE })
    }

    pub fn push(&self, frames: &[[i16; 2]]) {
        let mut buf = self.shared.buffer.lock().unwrap();
        buf.extend(frames.iter().copied());
        // Never let latency run away (e.g. after a stall).
        let max = (self.rate / 4) as usize;
        if buf.len() > max {
            let excess = buf.len() - max;
            buf.drain(..excess);
        }
    }

    pub fn buffered(&self) -> usize {
        self.shared.buffer.lock().unwrap().len()
    }

    pub fn clear(&self) {
        self.shared.buffer.lock().unwrap().clear();
    }

    pub fn set_volume(&self, volume: f32) {
        self.shared.volume.store((volume.clamp(0.0, 2.0) * 256.0) as u32, Ordering::Relaxed);
    }
}

impl Drop for AudioOut {
    fn drop(&mut self) {
        self.shared.running.store(false, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Linear-interpolating resampler with a small dynamic rate adjustment that
/// keeps the output buffer near its target fill level.
pub struct Resampler {
    pos: f64,
    prev: [f32; 2],
    cur: [f32; 2],
    out: Vec<[i16; 2]>,
}

impl Resampler {
    pub fn new() -> Resampler {
        Resampler { pos: 0.0, prev: [0.0; 2], cur: [0.0; 2], out: Vec::new() }
    }

    /// Convert `input` at `in_rate` to `out_rate`, nudging the ratio by `adjust` (e.g. 1.002).
    pub fn process(&mut self, input: &[[i16; 2]], in_rate: f64, out_rate: f64, adjust: f64) -> &[[i16; 2]] {
        self.out.clear();
        let step = in_rate / out_rate * adjust;
        for s in input {
            self.prev = self.cur;
            self.cur = [s[0] as f32, s[1] as f32];
            while self.pos < 1.0 {
                let t = self.pos as f32;
                self.out.push([
                    (self.prev[0] + (self.cur[0] - self.prev[0]) * t) as i16,
                    (self.prev[1] + (self.cur[1] - self.prev[1]) * t) as i16,
                ]);
                self.pos += step;
            }
            self.pos -= 1.0;
        }
        &self.out
    }
}

#[cfg(target_os = "linux")]
mod backend {
    use libloading::{Library, Symbol};
    use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void, CString};

    type SndPcm = c_void;
    const SND_PCM_STREAM_PLAYBACK: c_int = 0;
    const SND_PCM_FORMAT_S16_LE: c_int = 2;
    const SND_PCM_ACCESS_RW_INTERLEAVED: c_int = 3;

    type OpenFn = unsafe extern "C" fn(*mut *mut SndPcm, *const c_char, c_int, c_int) -> c_int;
    type SetParamsFn = unsafe extern "C" fn(*mut SndPcm, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
    type WriteiFn = unsafe extern "C" fn(*mut SndPcm, *const c_void, c_ulong) -> c_long;
    type RecoverFn = unsafe extern "C" fn(*mut SndPcm, c_int, c_int) -> c_int;
    type CloseFn = unsafe extern "C" fn(*mut SndPcm) -> c_int;

    pub struct Device {
        _lib: Library,
        pcm: *mut SndPcm,
        writei: WriteiFn,
        recover: RecoverFn,
        close: CloseFn,
    }

    // The PCM handle is only used from the audio thread after construction.
    unsafe impl Send for Device {}

    impl Device {
        pub fn open(rate: u32) -> Option<Device> {
            unsafe {
                let lib = Library::new("libasound.so.2").ok()?;
                let open: Symbol<OpenFn> = lib.get(b"snd_pcm_open\0").ok()?;
                let set_params: Symbol<SetParamsFn> = lib.get(b"snd_pcm_set_params\0").ok()?;
                let writei: WriteiFn = *lib.get::<WriteiFn>(b"snd_pcm_writei\0").ok()?;
                let recover: RecoverFn = *lib.get::<RecoverFn>(b"snd_pcm_recover\0").ok()?;
                let close: CloseFn = *lib.get::<CloseFn>(b"snd_pcm_close\0").ok()?;
                let name = CString::new("default").ok()?;
                let mut pcm: *mut SndPcm = std::ptr::null_mut();
                if open(&mut pcm, name.as_ptr(), SND_PCM_STREAM_PLAYBACK, 0) < 0 || pcm.is_null() {
                    return None;
                }
                if set_params(pcm, SND_PCM_FORMAT_S16_LE, SND_PCM_ACCESS_RW_INTERLEAVED, 2, rate, 1, 40_000) < 0 {
                    close(pcm);
                    return None;
                }
                Some(Device { _lib: lib, pcm, writei, recover, close })
            }
        }

        /// Blocking write; returns false on an unrecoverable error.
        pub fn write(&mut self, frames: &[[i16; 2]]) -> bool {
            let mut offset = 0;
            while offset < frames.len() {
                let rest = &frames[offset..];
                let n = unsafe { (self.writei)(self.pcm, rest.as_ptr() as *const c_void, rest.len() as c_ulong) };
                if n < 0 {
                    if unsafe { (self.recover)(self.pcm, n as c_int, 1) } < 0 {
                        return false;
                    }
                } else {
                    offset += n as usize;
                }
            }
            true
        }
    }

    impl Drop for Device {
        fn drop(&mut self) {
            unsafe {
                (self.close)(self.pcm);
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod backend {
    pub struct Device;
    impl Device {
        pub fn open(_rate: u32) -> Option<Device> {
            None
        }
        pub fn write(&mut self, _frames: &[[i16; 2]]) -> bool {
            false
        }
    }
}
