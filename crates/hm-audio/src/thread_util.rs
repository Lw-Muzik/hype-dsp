//! Per-thread tuning for audio and decode threads.
//!
//! Two independent, side-effect-only knobs live here:
//!
//! * **Denormal flushing** ([`enable_denormal_flush`]) — sets the calling
//!   thread's FPU to flush-to-zero / denormals-are-zero. IIR filter tails decay
//!   into denormal range during quiet passages, and on x86 each denormal
//!   multiply costs 50–100+ cycles (Apple Silicon handles them at full speed —
//!   an architecture-inconsistent perf cliff). The FPU control register is
//!   per-thread state, so every real-time thread must opt in itself.
//! * **Priority adjustment** ([`lower_current_thread_priority`],
//!   [`promote_current_thread_to_realtime`]) — keeps bursty background decode
//!   work from competing with the audio callback, and (Linux) keeps the
//!   system-EQ worker from stuttering under load.

/// Enable flush-to-zero / denormals-are-zero on the calling thread's FPU.
///
/// x86/x86_64: sets MXCSR FTZ (bit 15) + DAZ (bit 6). aarch64: sets FPCR FZ
/// (bit 24). Elsewhere: no-op. Denormals only ever occur in decaying filter
/// tails at magnitudes ~120 dB below full scale, so flushing them is inaudible.
pub fn enable_denormal_flush() {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // The MXCSR intrinsics are deprecated upstream ("not properly modeled"),
    // but remain the sanctioned way to set FTZ/DAZ; the write is intentional.
    #[allow(deprecated)]
    unsafe {
        #[cfg(target_arch = "x86")]
        use core::arch::x86::{_mm_getcsr, _mm_setcsr};
        #[cfg(target_arch = "x86_64")]
        use core::arch::x86_64::{_mm_getcsr, _mm_setcsr};
        // FTZ (bit 15) + DAZ (bit 6).
        _mm_setcsr(_mm_getcsr() | 0x8040);
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        // FPCR.FZ (bit 24): flush denormal inputs/results to zero.
        let mut fpcr: u64;
        core::arch::asm!("mrs {}, fpcr", out(reg) fpcr, options(nomem, nostack, preserves_flags));
        fpcr |= 1 << 24;
        core::arch::asm!("msr fpcr, {}", in(reg) fpcr, options(nomem, nostack, preserves_flags));
    }
}

/// [`enable_denormal_flush`], once per thread — cheap enough (a thread-local
/// flag check) for the top of a real-time callback. Needed there because cpal
/// callbacks can migrate threads on some backends, so a one-shot at stream
/// build time wouldn't stick; each callback re-checks its *current* thread.
pub fn enable_denormal_flush_once() {
    use std::cell::Cell;
    thread_local! {
        static ENABLED: Cell<bool> = const { Cell::new(false) };
    }
    ENABLED.with(|enabled| {
        if !enabled.get() {
            enable_denormal_flush();
            enabled.set(true);
        }
    });
}

/// Drop the calling thread below normal priority. Best-effort (ignores
/// failure): used by background decode workers whose bursty whole-track work
/// would otherwise compete with the audio callback on 2-core machines exactly
/// at track transitions.
pub fn lower_current_thread_priority() {
    #[cfg(unix)]
    unsafe {
        // nice(5): still plenty of CPU when idle, but always yields to the
        // (normal-priority-or-better) audio threads under contention.
        let _ = libc::nice(5);
    }
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

/// Best-effort real-time scheduling for the Linux system-EQ worker, which
/// carries *every* app's audio (Windows gets MMCSS "Pro Audio"; without this
/// Linux got nothing, so system audio stuttered under load). Tries `SCHED_RR`,
/// falls back to a niceness bump without RT privilege, and never fails
/// startup — worst case the worker just stays at default priority.
#[cfg(target_os = "linux")]
pub fn promote_current_thread_to_realtime() {
    unsafe {
        // `pid == 0` targets the calling thread on Linux (threads are tasks).
        let mut param: libc::sched_param = std::mem::zeroed();
        param.sched_priority = 20;
        if libc::sched_setscheduler(0, libc::SCHED_RR, &param) == 0 {
            return;
        }
        // EPERM (no CAP_SYS_NICE / rtkit): a niceness bump still beats default.
        // `as _` bridges glibc/musl signature differences for `which`.
        if libc::setpriority(libc::PRIO_PROCESS as _, 0, -10) != 0 {
            crate::diag::log(
                "system EQ: could not raise worker thread priority — continuing at default",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denormal_flush_is_idempotent_and_harmless() {
        enable_denormal_flush_once();
        enable_denormal_flush_once(); // second call: flag short-circuits
        enable_denormal_flush(); // direct call: setting the same bits again is fine

        // Ordinary (normal-range) float math is unaffected.
        let x = std::hint::black_box(1.5f32);
        assert_eq!(x * 2.0, 3.0);

        // On the arches we handle, an underflowing multiply now flushes to zero
        // (each #[test] runs on its own thread, so this doesn't leak elsewhere).
        #[cfg(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64"))]
        {
            let tiny = std::hint::black_box(f32::MIN_POSITIVE);
            let half = std::hint::black_box(0.5f32);
            assert_eq!(tiny * half, 0.0, "denormal result must flush to zero");
        }
    }

    #[test]
    fn lower_priority_never_panics() {
        lower_current_thread_priority();
    }
}
