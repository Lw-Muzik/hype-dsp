//! The HypeMuzik APO: a COM object the Windows audio engine loads into
//! `audiodg.exe` and calls, per block, to process every app's audio in place.
//!
//! It implements the four interfaces an sAPO needs (`IAudioProcessingObject`,
//! `IAudioProcessingObjectConfiguration`, `IAudioProcessingObjectRT`,
//! `IAudioSystemEffects`) and runs the app's shared [`ProcessChain`]. Live EQ
//! params arrive from the app across the process boundary via the
//! [`hm_core::apo_ipc`] seqlock; the real-time `APOProcess` reads them lock-free
//! and, on any fault, degrades to pass-through instead of crashing `audiodg`.

#![cfg(windows)]

use std::cell::UnsafeCell;
use std::sync::atomic::Ordering;

use hm_core::apo_ipc::{apply_pod, read_seqlock, SharedMapping};
use hm_core::EngineState;
use hm_dsp::ProcessChain;
use windows::Win32::Foundation::E_OUTOFMEMORY;
use windows::Win32::Media::Audio::Apo::{
    IAudioMediaType, IAudioProcessingObjectConfiguration_Impl, IAudioProcessingObjectRT_Impl,
    IAudioProcessingObject_Impl, IAudioSystemEffects, IAudioSystemEffects_Impl,
    APO_CONNECTION_DESCRIPTOR, APO_CONNECTION_PROPERTY, APO_FLAG_INPLACE, APO_REG_PROPERTIES,
    BUFFER_SILENT, BUFFER_VALID, UNCOMPRESSEDAUDIOFORMAT,
};
use windows::Win32::System::Com::CoTaskMemAlloc;
use windows_core::{implement, Interface, Ref};

use crate::guids::CLSID_HYPEMUZIK_APO;

const DEFAULT_RATE: f32 = 48_000.0;
const DEFAULT_CHANNELS: usize = 2;

/// Per-connection processing state, built in `LockForProcess` (where allocation
/// is allowed) and used by the real-time `APOProcess` (where it is not).
struct RtState {
    chain: ProcessChain,
    channels: usize,
    /// Reader over the app→APO param mapping. `None` if the app isn't running
    /// yet — the APO then passes audio through untouched.
    reader: Option<SharedMapping>,
    /// Scratch `EngineState` rebuilt from the param snapshot only when it changes.
    state: EngineState,
    /// Last seqlock version applied, so we rebuild params only on change.
    last_version: u32,
    /// Whether the app currently wants processing on (the `active` gate).
    active: bool,
}

/// The COM object. `UnsafeCell` gives interior mutability for the `&self` COM
/// methods; the APO contract serializes `LockForProcess`/`APOProcess`/
/// `UnlockForProcess`, so there is no concurrent access to `rt`.
#[implement(
    windows::Win32::Media::Audio::Apo::IAudioProcessingObject,
    windows::Win32::Media::Audio::Apo::IAudioProcessingObjectConfiguration,
    windows::Win32::Media::Audio::Apo::IAudioProcessingObjectRT,
    windows::Win32::Media::Audio::Apo::IAudioSystemEffects
)]
pub struct HypeMuzikApo {
    rt: UnsafeCell<Option<RtState>>,
}

impl HypeMuzikApo {
    pub fn new() -> Self {
        Self {
            rt: UnsafeCell::new(None),
        }
    }

    /// Interior-mutable access to the processing state. Safe under the APO's
    /// serialized-call contract.
    #[allow(clippy::mut_from_ref)]
    unsafe fn rt(&self) -> &mut Option<RtState> {
        &mut *self.rt.get()
    }
}

impl Default for HypeMuzikApo {
    fn default() -> Self {
        Self::new()
    }
}

impl IAudioProcessingObject_Impl for HypeMuzikApo_Impl {
    fn Reset(&self) -> windows_core::Result<()> {
        Ok(())
    }

    fn GetLatency(&self) -> windows_core::Result<i64> {
        // We add no algorithmic latency the engine must compensate for beyond the
        // block itself; report zero.
        Ok(0)
    }

    fn GetRegistrationProperties(&self) -> windows_core::Result<*mut APO_REG_PROPERTIES> {
        // The audio engine takes ownership and frees this with CoTaskMemFree.
        let size = std::mem::size_of::<APO_REG_PROPERTIES>();
        let p = unsafe { CoTaskMemAlloc(size) } as *mut APO_REG_PROPERTIES;
        if p.is_null() {
            return Err(windows_core::Error::from_hresult(E_OUTOFMEMORY));
        }
        let mut props = APO_REG_PROPERTIES {
            clsid: CLSID_HYPEMUZIK_APO,
            Flags: APO_FLAG_INPLACE,
            szFriendlyName: [0u16; 256],
            szCopyrightInfo: [0u16; 256],
            u32MajorVersion: 1,
            u32MinorVersion: 0,
            u32MinInputConnections: 1,
            u32MaxInputConnections: 1,
            u32MinOutputConnections: 1,
            u32MaxOutputConnections: 1,
            u32MaxInstances: u32::MAX,
            u32NumAPOInterfaces: 1,
            iidAPOInterfaceList: [IAudioSystemEffects::IID],
        };
        write_wide(&mut props.szFriendlyName, "HypeMuzik System Effect");
        write_wide(&mut props.szCopyrightInfo, "HypeMuzik");
        unsafe { std::ptr::write(p, props) };
        Ok(p)
    }

    fn Initialize(&self, _cbdatasize: u32, _pbydata: *const u8) -> windows_core::Result<()> {
        Ok(())
    }

    fn IsInputFormatSupported(
        &self,
        _poppositeformat: Ref<'_, IAudioMediaType>,
        prequestedinputformat: Ref<'_, IAudioMediaType>,
    ) -> windows_core::Result<IAudioMediaType> {
        // Accept whatever the engine offers (it negotiates 32-bit float); echo it
        // back as the supported format.
        Ok(prequestedinputformat.ok()?.clone())
    }

    fn IsOutputFormatSupported(
        &self,
        _poppositeformat: Ref<'_, IAudioMediaType>,
        prequestedoutputformat: Ref<'_, IAudioMediaType>,
    ) -> windows_core::Result<IAudioMediaType> {
        Ok(prequestedoutputformat.ok()?.clone())
    }

    fn GetInputChannelCount(&self) -> windows_core::Result<u32> {
        let ch = unsafe { self.rt() }
            .as_ref()
            .map(|r| r.channels as u32)
            .unwrap_or(DEFAULT_CHANNELS as u32);
        Ok(ch)
    }
}

impl IAudioProcessingObjectConfiguration_Impl for HypeMuzikApo_Impl {
    fn LockForProcess(
        &self,
        u32numinputconnections: u32,
        ppinputconnections: *const *const APO_CONNECTION_DESCRIPTOR,
        _u32numoutputconnections: u32,
        _ppoutputconnections: *const *const APO_CONNECTION_DESCRIPTOR,
    ) -> windows_core::Result<()> {
        let (mut rate, mut channels) = (DEFAULT_RATE, DEFAULT_CHANNELS);
        unsafe {
            if u32numinputconnections >= 1 && !ppinputconnections.is_null() {
                let desc = *ppinputconnections;
                if !desc.is_null() {
                    let d = &*desc;
                    if let Some(fmt) = (*d.pFormat).as_ref() {
                        let mut uf = UNCOMPRESSEDAUDIOFORMAT::default();
                        if fmt.GetUncompressedAudioFormat(&mut uf).is_ok() {
                            if uf.fFramesPerSecond > 0.0 {
                                rate = uf.fFramesPerSecond;
                            }
                            if uf.dwSamplesPerFrame > 0 {
                                channels = uf.dwSamplesPerFrame as usize;
                            }
                        }
                    }
                }
            }
            // Best-effort: if the app isn't running the reader is None and we pass
            // audio through until it starts.
            let reader = SharedMapping::open_reader(hm_core::apo_ids::MAPPING_NAME).ok();
            *self.rt() = Some(RtState {
                chain: ProcessChain::standard(rate, channels),
                channels,
                reader,
                state: EngineState::default(),
                last_version: 0,
                active: false,
            });
        }
        Ok(())
    }

    fn UnlockForProcess(&self) -> windows_core::Result<()> {
        unsafe { *self.rt() = None };
        Ok(())
    }
}

impl IAudioProcessingObjectRT_Impl for HypeMuzikApo_Impl {
    fn APOProcess(
        &self,
        u32numinputconnections: u32,
        ppinputconnections: *const *const APO_CONNECTION_PROPERTY,
        u32numoutputconnections: u32,
        ppoutputconnections: *mut *mut APO_CONNECTION_PROPERTY,
    ) {
        // A fault in the DSP must never crash audiodg (which would kill ALL system
        // audio) — catch it and fall through to pass-through.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            if u32numinputconnections == 0
                || u32numoutputconnections == 0
                || ppinputconnections.is_null()
                || ppoutputconnections.is_null()
            {
                return;
            }
            let in_p = *ppinputconnections;
            let out_p = *ppoutputconnections;
            if in_p.is_null() || out_p.is_null() {
                return;
            }
            let inp = &*in_p;
            let outp = &mut *out_p;
            outp.u32ValidFrameCount = inp.u32ValidFrameCount;

            let Some(rt) = self.rt().as_mut() else {
                outp.u32BufferFlags = inp.u32BufferFlags;
                return;
            };
            let n = (inp.u32ValidFrameCount as usize).saturating_mul(rt.channels);
            let src = inp.pBuffer as *const f32;
            let dst = outp.pBuffer as *mut f32;
            if n == 0 || inp.u32BufferFlags == BUFFER_SILENT || src.is_null() || dst.is_null() {
                outp.u32BufferFlags = inp.u32BufferFlags;
                return;
            }
            // Copy input into the (possibly distinct) output buffer, then process
            // it in place there.
            if src as usize != dst as usize {
                std::ptr::copy_nonoverlapping(src, dst, n);
            }
            outp.u32BufferFlags = BUFFER_VALID;
            let buf = std::slice::from_raw_parts_mut(dst, n);

            // Refresh params only when the app's snapshot version changed (rare),
            // so the RT path is just a lock-free version read most blocks.
            let ver = rt
                .reader
                .as_ref()
                .map(|r| r.cell().version.load(Ordering::Acquire));
            if let Some(ver) = ver {
                if ver != rt.last_version {
                    if let Some(pod) = rt.reader.as_ref().and_then(|r| read_seqlock(r.cell())) {
                        apply_pod(&pod, &mut rt.state);
                        rt.chain.set_params(&rt.state);
                        rt.active = pod.active == 1;
                    }
                    rt.last_version = ver;
                }
                if rt.active && rt.state.power {
                    rt.chain.process(buf, rt.channels);
                } else if (rt.state.master_volume - 1.0).abs() > f32::EPSILON {
                    for s in buf.iter_mut() {
                        *s *= rt.state.master_volume;
                    }
                }
            }
        }));
    }

    fn CalcInputFrames(&self, u32outputframecount: u32) -> u32 {
        u32outputframecount
    }

    fn CalcOutputFrames(&self, u32inputframecount: u32) -> u32 {
        u32inputframecount
    }
}

impl IAudioSystemEffects_Impl for HypeMuzikApo_Impl {}

/// Fill a fixed-size UTF-16 buffer with `s` and a null terminator (truncating if
/// needed). Unused-import note: `GUID` stays imported for the trait signatures.
fn write_wide(dst: &mut [u16], s: &str) {
    let mut i = 0;
    for u in s.encode_utf16() {
        if i + 1 >= dst.len() {
            break;
        }
        dst[i] = u;
        i += 1;
    }
    dst[i] = 0;
}
