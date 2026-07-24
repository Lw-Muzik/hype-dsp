//! Cross-process live-parameter IPC for the Windows APO.
//!
//! The custom Audio Processing Object runs inside `audiodg.exe` — a different
//! process from the desktop app — so live EQ/DSP parameter changes must cross
//! a process boundary. This module defines that channel:
//!
//! - [`EngineParamsPod`]: a `#[repr(C)]`, `Copy`, heap-free snapshot of every
//!   numeric field the DSP chain (`hm-dsp::ProcessChain::set_params`) actually
//!   reads for the stages the APO implements — EQ, bass boost, spatializer,
//!   3D surround, room reverb, output gain/limiter — plus `power`,
//!   `master_volume`, and an `active` gate. Stages whose live parameters carry
//!   unbounded heap data (headphone-correction parametric bands, the
//!   convolver's impulse response, the LiveProg script source) are out of
//!   scope for this POD; they are not `repr(C)`-representable and are not
//!   part of the APO's v1 feature set.
//! - [`SeqlockCell`]: the odd/even-version protocol that lets a single writer
//!   (the app) and any number of readers (the APO's real-time audio callback)
//!   share one `EngineParamsPod` without a lock. Readers never block and
//!   never allocate — a torn read is just retried a bounded number of times.
//! - [`SharedMapping`] (Windows only): the named `CreateFileMappingW` /
//!   `MapViewOfFile` region the two processes both attach to, sized to hold
//!   exactly one `SeqlockCell`. The mapping name is
//!   [`crate::apo_ids::MAPPING_NAME`].
//!
//! The POD and seqlock are pure and host-testable (macOS/Linux/Windows); only
//! [`SharedMapping`] is `#[cfg(windows)]` since it calls into the `windows`
//! crate.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::types::{BassBoostState, EngineState, RoomState, SpatialMode, SurroundSpeakers, BAND_COUNT};

/// `#[repr(C)]` snapshot of every numeric DSP parameter the Windows APO
/// consumes, carried across the process boundary through a [`SeqlockCell`].
///
/// Every field is a plain `u32`/`f32` (booleans as `u32` and the
/// [`SpatialMode`] enum as a `u32` discriminant) so the layout is stable and
/// FFI-safe without relying on Rust's `bool`/enum representations. `active`
/// gates whether the APO should apply these parameters at all (distinct from
/// `power`, which is the DSP chain's own master-bypass switch mirrored from
/// [`EngineState::power`]).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EngineParamsPod {
    /// Non-zero once the writer has published at least one real snapshot.
    pub active: u32,
    /// Mirrors [`EngineState::power`] (master bypass).
    pub power: u32,
    /// Mirrors [`EngineState::master_volume`] (linear gain).
    pub master_volume: f32,

    // ── Graphic EQ ───────────────────────────────────────────────────────
    pub eq_enabled: u32,
    pub eq_pre_gain: f32,
    pub eq_bands: [f32; BAND_COUNT],

    // ── Bass boost ───────────────────────────────────────────────────────
    pub bass_enabled: u32,
    pub bass_amount: f32,
    pub bass_harmonics: u32,
    pub bass_adaptive: u32,

    // ── Spatializer (crossfeed/HRTF widener) ────────────────────────────
    pub spatializer_enabled: u32,
    pub spatializer_amount: f32,
    /// [`SpatialMode`] discriminant: `0` = Crossfeed, `1` = Hrtf.
    pub spatializer_mode: u32,

    // ── 3D Surround ──────────────────────────────────────────────────────
    pub surround_enabled: u32,
    pub surround_intensity: f32,
    pub surround_subwoofer: f32,
    pub surround_front_l: u32,
    pub surround_front_r: u32,
    pub surround_side_l: u32,
    pub surround_side_r: u32,
    pub surround_rear_l: u32,
    pub surround_rear_r: u32,

    // ── Room reverb ──────────────────────────────────────────────────────
    pub room_enabled: u32,
    pub room_size: f32,
    pub room_decay: f32,
    pub room_damping: f32,
    pub room_pre_delay: f32,
    pub room_diffusion: f32,
    pub room_wet_dry: f32,

    // ── Output gain + limiter ────────────────────────────────────────────
    pub output_gain_db: f32,
    pub limiter_enabled: u32,
    pub limiter_ceiling_db: f32,
}

#[inline]
fn b2u(b: bool) -> u32 {
    b as u32
}

#[inline]
fn u2b(u: u32) -> bool {
    u != 0
}

#[inline]
fn mode_to_u32(m: SpatialMode) -> u32 {
    match m {
        SpatialMode::Crossfeed => 0,
        SpatialMode::Hrtf => 1,
    }
}

#[inline]
fn u32_to_mode(u: u32) -> SpatialMode {
    match u {
        1 => SpatialMode::Hrtf,
        _ => SpatialMode::Crossfeed,
    }
}

impl EngineParamsPod {
    /// Build a POD snapshot from the live [`EngineState`]. `active` marks
    /// whether the APO should apply these parameters (distinct from
    /// `state.power`, the DSP chain's own bypass toggle).
    pub fn from_state(state: &EngineState, active: bool) -> Self {
        let eq = &state.eq;
        let bass = &state.bass;
        let sp = &state.spatializer;
        let sr = &state.surround3d;
        let room = &state.room;
        let out = &state.output;
        Self {
            active: b2u(active),
            power: b2u(state.power),
            master_volume: state.master_volume,

            eq_enabled: b2u(eq.enabled),
            eq_pre_gain: eq.pre_gain,
            eq_bands: eq.bands,

            bass_enabled: b2u(bass.enabled),
            bass_amount: bass.amount,
            bass_harmonics: b2u(bass.harmonics),
            bass_adaptive: b2u(bass.adaptive),

            spatializer_enabled: b2u(sp.enabled),
            spatializer_amount: sp.amount,
            spatializer_mode: mode_to_u32(sp.mode),

            surround_enabled: b2u(sr.enabled),
            surround_intensity: sr.intensity,
            surround_subwoofer: sr.subwoofer,
            surround_front_l: b2u(sr.speakers.front_l),
            surround_front_r: b2u(sr.speakers.front_r),
            surround_side_l: b2u(sr.speakers.side_l),
            surround_side_r: b2u(sr.speakers.side_r),
            surround_rear_l: b2u(sr.speakers.surround_l),
            surround_rear_r: b2u(sr.speakers.surround_r),

            room_enabled: b2u(room.enabled),
            room_size: room.room_size,
            room_decay: room.decay,
            room_damping: room.damping,
            room_pre_delay: room.pre_delay,
            room_diffusion: room.diffusion,
            room_wet_dry: room.wet_dry,

            output_gain_db: out.gain_db,
            limiter_enabled: b2u(out.limiter_enabled),
            limiter_ceiling_db: out.ceiling_db,
        }
    }
}

/// Write every field the POD carries back into a reusable [`EngineState`], so
/// the APO can feed the result straight into the existing
/// `ProcessChain::set_params`. Fields the POD does not carry (headphone
/// correction bands, convolver IR/script source, playback/system-EQ-scope
/// settings, preset ids, …) are left untouched — callers pass an `out` that
/// already holds sane defaults for those (e.g. a chain-local `EngineState`
/// that only this function ever mutates).
pub fn apply_pod(pod: &EngineParamsPod, out: &mut EngineState) {
    out.power = u2b(pod.power);
    out.master_volume = pod.master_volume;

    out.eq.enabled = u2b(pod.eq_enabled);
    out.eq.pre_gain = pod.eq_pre_gain;
    out.eq.bands = pod.eq_bands;

    out.bass = BassBoostState {
        enabled: u2b(pod.bass_enabled),
        amount: pod.bass_amount,
        harmonics: u2b(pod.bass_harmonics),
        adaptive: u2b(pod.bass_adaptive),
    };

    out.spatializer.enabled = u2b(pod.spatializer_enabled);
    out.spatializer.amount = pod.spatializer_amount;
    out.spatializer.mode = u32_to_mode(pod.spatializer_mode);

    out.surround3d.enabled = u2b(pod.surround_enabled);
    out.surround3d.intensity = pod.surround_intensity;
    out.surround3d.subwoofer = pod.surround_subwoofer;
    out.surround3d.speakers = SurroundSpeakers {
        front_l: u2b(pod.surround_front_l),
        front_r: u2b(pod.surround_front_r),
        side_l: u2b(pod.surround_side_l),
        side_r: u2b(pod.surround_side_r),
        surround_l: u2b(pod.surround_rear_l),
        surround_r: u2b(pod.surround_rear_r),
    };

    out.room = RoomState {
        enabled: u2b(pod.room_enabled),
        room_size: pod.room_size,
        decay: pod.room_decay,
        damping: pod.room_damping,
        pre_delay: pod.room_pre_delay,
        diffusion: pod.room_diffusion,
        wet_dry: pod.room_wet_dry,
        // Not carried by the POD (UI-only metadata) — preserve whatever the
        // reusable `out` already had.
        active_preset_id: out.room.active_preset_id.clone(),
    };

    out.output.gain_db = pod.output_gain_db;
    out.output.limiter_enabled = u2b(pod.limiter_enabled);
    out.output.ceiling_db = pod.limiter_ceiling_db;
}

/// Bounded retry budget for [`read_seqlock`]. A writer critical section is a
/// handful of `u32`/`f32` copies — hundreds of spins is generous headroom
/// even under heavy contention, while still bounding worst-case latency for
/// the real-time audio callback that calls this.
const MAX_READ_RETRIES: u32 = 100;

/// A `#[repr(C)]` seqlock cell: an [`AtomicU32`] version counter guarding one
/// [`EngineParamsPod`] payload, laid out for a shared-memory mapping.
///
/// Protocol (single writer, any number of readers):
/// - `version == 0`: never written.
/// - `version` odd: a write is in progress (payload may be torn).
/// - `version` even and non-zero: payload is a complete, consistent snapshot
///   as of that version.
///
/// The payload lives in an [`UnsafeCell`] because both [`write_seqlock`] and
/// [`read_seqlock`] take `&SeqlockCell` — across a process boundary there is
/// no way to hand out `&mut` — so all mutation goes through interior
/// mutability guarded by the version protocol instead of Rust's borrow
/// checker.
#[repr(C)]
pub struct SeqlockCell {
    pub version: AtomicU32,
    _pad: u32,
    payload: UnsafeCell<EngineParamsPod>,
}

// SAFETY: all access to `payload` is mediated by the version protocol in
// `write_seqlock`/`read_seqlock`, which is the same discipline a real
// cross-process seqlock relies on (there is no borrow checker spanning two
// OS processes either way).
unsafe impl Sync for SeqlockCell {}

impl SeqlockCell {
    /// A cell in the "never written" state: `version == 0`, zeroed payload.
    /// This is exactly the bit pattern a freshly-mapped, pagefile-backed
    /// Windows shared memory region already has, so `SharedMapping` needs no
    /// separate initialization step on creation.
    pub fn zeroed() -> Self {
        Self {
            version: AtomicU32::new(0),
            _pad: 0,
            payload: UnsafeCell::new(EngineParamsPod::default()),
        }
    }
}

impl Default for SeqlockCell {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// Publish `pod` into `cell`. The sole writer: bumps the version to odd
/// (writing), copies the payload, then bumps it to even (done). Never
/// allocates or blocks.
pub fn write_seqlock(cell: &SeqlockCell, pod: &EngineParamsPod) {
    let v = cell.version.load(Ordering::Relaxed);
    let writing = v | 1;
    cell.version.store(writing, Ordering::Release);
    // SAFETY: `writing` (odd) tells every reader to retry instead of reading
    // `payload`, so this is the sole accessor for the duration of the copy.
    unsafe {
        *cell.payload.get() = *pod;
    }
    cell.version.store(writing.wrapping_add(1), Ordering::Release);
}

/// Read the last consistent snapshot from `cell`. Returns `None` if the cell
/// has never been written, or if a consistent snapshot couldn't be obtained
/// within the bounded retry budget (persistent torn reads under extreme
/// contention) — never blocks, never allocates, safe to call from a
/// real-time audio callback.
pub fn read_seqlock(cell: &SeqlockCell) -> Option<EngineParamsPod> {
    for _ in 0..MAX_READ_RETRIES {
        let before = cell.version.load(Ordering::Acquire);
        if before == 0 {
            return None; // never written
        }
        if before & 1 != 0 {
            std::hint::spin_loop();
            continue; // writer in progress — torn payload, retry
        }
        // SAFETY: `before` was even, i.e. no writer held the cell at the
        // moment of that load; we re-check the version below to detect a
        // write that started during this copy.
        let pod = unsafe { *cell.payload.get() };
        let after = cell.version.load(Ordering::Acquire);
        if after == before {
            return Some(pod);
        }
        std::hint::spin_loop();
    }
    None
}

/// The Windows named shared-memory mapping carrying one [`SeqlockCell`].
///
/// The app process calls [`SharedMapping::create_writer`] once at startup
/// (or whenever the APO feature is enabled); the APO DLL loaded into
/// `audiodg.exe` calls [`SharedMapping::open_reader`] to attach to the same
/// region. Both sides then read/write through [`SharedMapping::cell`] using
/// [`write_seqlock`]/[`read_seqlock`] — never anything else, since there is
/// no OS-level synchronization here beyond the mapping itself.
#[cfg(windows)]
pub struct SharedMapping {
    handle: windows::Win32::Foundation::HANDLE,
    view: *mut SeqlockCell,
}

#[cfg(windows)]
// SAFETY: the mapping is just an OS handle plus a stable pointer into shared
// memory; every access to the pointee goes through the seqlock protocol,
// which is itself sound to call from any thread.
unsafe impl Send for SharedMapping {}
#[cfg(windows)]
unsafe impl Sync for SharedMapping {}

#[cfg(windows)]
impl SharedMapping {
    /// Create the named mapping (pagefile-backed, sized for one
    /// `SeqlockCell`) as the writer side — the desktop app. If the mapping
    /// already exists (e.g. a previous instance's APO is still attached),
    /// this attaches to it rather than failing.
    pub fn create_writer(name: &str) -> std::io::Result<Self> {
        use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows::Win32::System::Memory::{CreateFileMappingW, PAGE_READWRITE};
        use windows::core::PCWSTR;

        let wide = to_wide(name);
        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                std::mem::size_of::<SeqlockCell>() as u32,
                PCWSTR(wide.as_ptr()),
            )
        }
        .map_err(win_err)?;
        Self::from_handle(handle)
    }

    /// Open an existing mapping as the reader side — the APO running inside
    /// `audiodg.exe`. Fails if the app hasn't created the mapping yet.
    pub fn open_reader(name: &str) -> std::io::Result<Self> {
        use windows::Win32::System::Memory::{OpenFileMappingW, FILE_MAP_ALL_ACCESS};
        use windows::core::PCWSTR;

        let wide = to_wide(name);
        let handle = unsafe { OpenFileMappingW(FILE_MAP_ALL_ACCESS.0, false, PCWSTR(wide.as_ptr())) }
            .map_err(win_err)?;
        Self::from_handle(handle)
    }

    fn from_handle(handle: windows::Win32::Foundation::HANDLE) -> std::io::Result<Self> {
        use windows::Win32::System::Memory::{MapViewOfFile, FILE_MAP_ALL_ACCESS};

        let view = unsafe {
            MapViewOfFile(
                handle,
                FILE_MAP_ALL_ACCESS,
                0,
                0,
                std::mem::size_of::<SeqlockCell>(),
            )
        };
        if view.Value.is_null() {
            let err = win_err(windows::core::Error::from_win32());
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(handle);
            }
            return Err(err);
        }
        Ok(Self {
            handle,
            view: view.Value as *mut SeqlockCell,
        })
    }

    /// The shared seqlock cell backing this mapping.
    pub fn cell(&self) -> &SeqlockCell {
        // SAFETY: `view` points at a live `MapViewOfFile` region sized for
        // exactly one `SeqlockCell`, valid until `Drop` unmaps it.
        unsafe { &*self.view }
    }
}

#[cfg(windows)]
impl Drop for SharedMapping {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Memory::{UnmapViewOfFile, MEMORY_MAPPED_VIEW_ADDRESS};

        unsafe {
            let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                Value: self.view as *mut _,
            });
            let _ = CloseHandle(self.handle);
        }
    }
}

#[cfg(windows)]
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn win_err(e: windows::core::Error) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EngineState;

    #[test]
    fn pod_roundtrips_through_engine_state() {
        let mut st = EngineState::default();
        st.power = true;
        st.master_volume = 0.75;
        st.eq.enabled = true;
        st.eq.pre_gain = -3.0;
        st.eq.bands[0] = 6.0;
        let pod = EngineParamsPod::from_state(&st, /*active=*/ true);
        let mut back = EngineState::default();
        apply_pod(&pod, &mut back);
        assert!(back.power);
        assert!((back.master_volume - 0.75).abs() < 1e-6);
        assert!(back.eq.enabled);
        assert!((back.eq.pre_gain + 3.0).abs() < 1e-6);
        assert!((back.eq.bands[0] - 6.0).abs() < 1e-6);
    }

    /// The brief's illustrative test asserted `got.version_even()` on a
    /// *returned pod* — nonsensical, since a pod has no version of its own.
    /// What actually matters is the seqlock protocol: after a completed
    /// write the cell's version is even, and a read performed after that
    /// write returns a pod with `active == 1`.
    #[test]
    fn seqlock_reads_last_consistent_write() {
        let cell = SeqlockCell::zeroed();
        let a = EngineParamsPod::from_state(&EngineState::default(), true);
        write_seqlock(&cell, &a);

        assert_eq!(
            cell.version.load(Ordering::Acquire) % 2,
            0,
            "version must be left even after a completed write"
        );

        let got = read_seqlock(&cell).expect("a write happened, so a read must succeed");
        assert_eq!(got.active, 1);
    }

    #[test]
    fn seqlock_none_before_first_write() {
        let cell = SeqlockCell::zeroed();
        assert!(read_seqlock(&cell).is_none());
    }

    /// A second write must fully replace the first — no stale fields survive
    /// (guards against a partial/`..Default` bug in `write_seqlock`).
    #[test]
    fn seqlock_second_write_replaces_first() {
        let cell = SeqlockCell::zeroed();
        let mut st = EngineState::default();
        st.eq.bands[3] = 2.0;
        write_seqlock(&cell, &EngineParamsPod::from_state(&st, true));

        st.eq.bands[3] = -5.0;
        st.master_volume = 0.4;
        write_seqlock(&cell, &EngineParamsPod::from_state(&st, true));

        let got = read_seqlock(&cell).expect("written twice");
        assert!((got.eq_bands[3] + 5.0).abs() < 1e-6);
        assert!((got.master_volume - 0.4).abs() < 1e-6);
    }

    /// `apply_pod` must round-trip every stage the POD claims to carry, not
    /// just the fields the brief's example happened to touch.
    #[test]
    fn apply_pod_covers_every_carried_stage() {
        let mut st = EngineState::default();
        st.bass.enabled = true;
        st.bass.amount = 4.5;
        st.bass.harmonics = true;
        st.bass.adaptive = true;
        st.spatializer.enabled = true;
        st.spatializer.amount = 0.6;
        st.spatializer.mode = SpatialMode::Hrtf;
        st.surround3d.enabled = true;
        st.surround3d.intensity = 0.7;
        st.surround3d.subwoofer = 0.2;
        st.surround3d.speakers.side_l = false;
        st.surround3d.speakers.surround_r = false;
        st.room.enabled = true;
        st.room.room_size = 0.8;
        st.room.decay = 0.6;
        st.room.damping = 0.3;
        st.room.pre_delay = 20.0;
        st.room.diffusion = 0.9;
        st.room.wet_dry = 0.5;
        st.output.gain_db = 3.0;
        st.output.limiter_enabled = false;
        st.output.ceiling_db = -1.0;

        let pod = EngineParamsPod::from_state(&st, true);
        let mut back = EngineState::default();
        apply_pod(&pod, &mut back);

        assert_eq!(back.bass, st.bass);
        assert_eq!(back.spatializer, st.spatializer);
        assert_eq!(back.surround3d, st.surround3d);
        assert_eq!(back.room.enabled, st.room.enabled);
        assert_eq!(back.room.room_size, st.room.room_size);
        assert_eq!(back.room.decay, st.room.decay);
        assert_eq!(back.room.damping, st.room.damping);
        assert_eq!(back.room.pre_delay, st.room.pre_delay);
        assert_eq!(back.room.diffusion, st.room.diffusion);
        assert_eq!(back.room.wet_dry, st.room.wet_dry);
        assert_eq!(back.output.gain_db, st.output.gain_db);
        assert_eq!(back.output.limiter_enabled, st.output.limiter_enabled);
        assert_eq!(back.output.ceiling_db, st.output.ceiling_db);
    }

    #[test]
    fn pod_is_plain_old_data() {
        // Compile-time assertions that the POD stays FFI-safe: `Copy`, no
        // padding surprises from a stray non-numeric field. `size_of` is a
        // cheap sanity check against accidental additions of pointer-sized
        // heap-owning fields (which would break `#[repr(C)]` sharing).
        fn assert_copy<T: Copy>() {}
        assert_copy::<EngineParamsPod>();
        assert!(std::mem::size_of::<EngineParamsPod>() < 512);
    }
}
