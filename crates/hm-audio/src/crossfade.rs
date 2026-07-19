//! The law both queues fade by.
//!
//! [`queue`](crate::queue) (local) and [`stream_queue`](crate::stream_queue)
//! (cloud/phone/YouTube Music) run the same bounded-window read logic over
//! different loaders, so the ramp lives here rather than being written twice —
//! two copies of a curve is two curves the moment one is touched.

/// The outgoing and incoming gains at `t` through a crossfade, `t` in `0..=1`.
///
/// **Equal power, not equal amplitude.** The obvious ramp — `1-t` against `t` —
/// is a *linear* fade, and it audibly dips in the middle of every transition.
/// Two different tracks are uncorrelated signals, so their powers add rather
/// than their amplitudes: at the midpoint each is at 0.5 amplitude, giving
/// `0.5² + 0.5² = 0.5` — **−3 dB**, a hole in the middle of the fade. On a long
/// crossfade it reads as the music sagging and recovering, which is exactly the
/// "not smooth" complaint a linear ramp always earns.
///
/// `cos`/`sin` over a quarter turn instead holds `out² + in² = 1` the whole way
/// across, so the total power is constant and the transition is level. The ends
/// are still exactly 1 and 0, so nothing about the boundaries changes.
///
/// (Equal *amplitude* is the right law for the opposite case — the same signal
/// on both sides, where amplitudes really do add and a cos/sin pair would bulge
/// by +3 dB instead. Two tracks in a playlist are never that.)
#[inline]
pub(crate) fn gains(t: f32) -> (f32, f32) {
    let t = t.clamp(0.0, 1.0);
    let angle = t * std::f32::consts::FRAC_PI_2;
    (angle.cos(), angle.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the whole module exists for: no dip, no bulge, anywhere.
    #[test]
    fn power_is_constant_across_the_whole_ramp() {
        for step in 0..=100 {
            let t = step as f32 / 100.0;
            let (out, inc) = gains(t);
            let power = out * out + inc * inc;
            assert!(
                (power - 1.0).abs() < 1e-5,
                "power must stay at unity: {power} at t={t}"
            );
        }
    }

    /// The midpoint is the whole argument. A linear ramp sits at 0.5/0.5 here,
    /// which is −3 dB of total power; equal power sits at √½ on both sides.
    #[test]
    fn the_midpoint_does_not_dip() {
        let (out, inc) = gains(0.5);
        let half_root = std::f32::consts::FRAC_1_SQRT_2;
        assert!((out - half_root).abs() < 1e-5, "outgoing at midpoint: {out}");
        assert!((inc - half_root).abs() < 1e-5, "incoming at midpoint: {inc}");
        assert!(out > 0.5, "a linear ramp would be 0.5 here — that's the dip");
    }

    /// The ends must be exact: a fade that starts at 0.9999 of the outgoing
    /// track, or ends short of the incoming one, is a step at the boundary.
    #[test]
    fn the_ends_are_fully_one_track_and_then_fully_the_other() {
        let (out0, in0) = gains(0.0);
        assert!((out0 - 1.0).abs() < 1e-6 && in0.abs() < 1e-6, "{out0}, {in0}");
        let (out1, in1) = gains(1.0);
        assert!(out1.abs() < 1e-6 && (in1 - 1.0).abs() < 1e-6, "{out1}, {in1}");
    }

    /// Both gains move monotonically, so the transition never reverses direction
    /// mid-fade — audible as a wobble if it ever did.
    #[test]
    fn the_ramp_never_backtracks() {
        let (mut last_out, mut last_in) = gains(0.0);
        for step in 1..=100 {
            let (out, inc) = gains(step as f32 / 100.0);
            assert!(out <= last_out + 1e-6, "outgoing rose at step {step}");
            assert!(inc >= last_in - 1e-6, "incoming fell at step {step}");
            last_out = out;
            last_in = inc;
        }
    }

    /// A `t` outside the window is a bug upstream, but it must not produce a
    /// gain outside `0..=1` and hand the output stage something to clip.
    #[test]
    fn out_of_range_t_is_clamped_rather_than_wrapped() {
        assert_eq!(gains(-1.0), gains(0.0));
        assert_eq!(gains(2.0), gains(1.0));
    }
}
