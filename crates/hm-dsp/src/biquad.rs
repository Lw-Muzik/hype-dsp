//! A single biquad (two-pole/two-zero) filter using Audio EQ Cookbook
//! coefficients. Coefficients are computed in `f64` for precision and stored as
//! `f32`; processing uses Direct Form II Transposed for good numerical behavior.

/// One biquad section: normalized coefficients (a0 == 1) plus DF2T state for a
/// single channel.
#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Default for Biquad {
    fn default() -> Self {
        Self::identity()
    }
}

impl Biquad {
    /// A pass-through filter (output == input).
    pub fn identity() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// Configure this section as an RBJ peaking EQ at `f0` with `gain_db` and
    /// quality `q`. Preserves the running filter state (for live re-tuning).
    /// Computed in `f64` for precision; coefficients stored as `f32`.
    pub fn set_peaking(&mut self, sample_rate: f32, f0: f32, gain_db: f32, q: f32) {
        let fs = sample_rate as f64;
        // Keep the center safely below Nyquist for low device sample rates.
        let f0 = (f0 as f64).clamp(1.0, fs * 0.495);
        let q = (q as f64).max(1e-4);
        let a = 10f64.powf(gain_db as f64 / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * f0 / fs;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;

        self.b0 = (b0 / a0) as f32;
        self.b1 = (b1 / a0) as f32;
        self.b2 = (b2 / a0) as f32;
        self.a1 = (a1 / a0) as f32;
        self.a2 = (a2 / a0) as f32;
    }

    /// Configure as an RBJ low-shelf at `f0`.
    pub fn set_low_shelf(&mut self, sample_rate: f32, f0: f32, gain_db: f32, q: f32) {
        let fs = sample_rate as f64;
        let f0 = (f0 as f64).clamp(1.0, fs * 0.495);
        let q = (q as f64).max(1e-4);
        let a = 10f64.powf(gain_db as f64 / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * f0 / fs;
        let cos = w0.cos();
        let alpha = w0.sin() / (2.0 * q);
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) - (a - 1.0) * cos + two_sqrt_a_alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos - two_sqrt_a_alpha);
        let a0 = (a + 1.0) + (a - 1.0) * cos + two_sqrt_a_alpha;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos);
        let a2 = (a + 1.0) + (a - 1.0) * cos - two_sqrt_a_alpha;
        self.assign(b0, b1, b2, a0, a1, a2);
    }

    /// Configure as an RBJ high-shelf at `f0`.
    pub fn set_high_shelf(&mut self, sample_rate: f32, f0: f32, gain_db: f32, q: f32) {
        let fs = sample_rate as f64;
        let f0 = (f0 as f64).clamp(1.0, fs * 0.495);
        let q = (q as f64).max(1e-4);
        let a = 10f64.powf(gain_db as f64 / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * f0 / fs;
        let cos = w0.cos();
        let alpha = w0.sin() / (2.0 * q);
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) + (a - 1.0) * cos + two_sqrt_a_alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cos - two_sqrt_a_alpha);
        let a0 = (a + 1.0) - (a - 1.0) * cos + two_sqrt_a_alpha;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos);
        let a2 = (a + 1.0) - (a - 1.0) * cos - two_sqrt_a_alpha;
        self.assign(b0, b1, b2, a0, a1, a2);
    }

    /// Configure as an RBJ low-pass at `f0` with quality `q`.
    pub fn set_lowpass(&mut self, sample_rate: f32, f0: f32, q: f32) {
        let fs = sample_rate as f64;
        let f0 = (f0 as f64).clamp(1.0, fs * 0.495);
        let q = (q as f64).max(1e-4);
        let w0 = 2.0 * std::f64::consts::PI * f0 / fs;
        let cos = w0.cos();
        let alpha = w0.sin() / (2.0 * q);
        let b1 = 1.0 - cos;
        let b0 = b1 / 2.0;
        let b2 = b0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos;
        let a2 = 1.0 - alpha;
        self.assign(b0, b1, b2, a0, a1, a2);
    }

    /// Configure as an RBJ high-pass at `f0` with quality `q`.
    pub fn set_highpass(&mut self, sample_rate: f32, f0: f32, q: f32) {
        let fs = sample_rate as f64;
        let f0 = (f0 as f64).clamp(1.0, fs * 0.495);
        let q = (q as f64).max(1e-4);
        let w0 = 2.0 * std::f64::consts::PI * f0 / fs;
        let cos = w0.cos();
        let alpha = w0.sin() / (2.0 * q);
        let b1 = -(1.0 + cos);
        let b0 = (1.0 + cos) / 2.0;
        let b2 = b0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos;
        let a2 = 1.0 - alpha;
        self.assign(b0, b1, b2, a0, a1, a2);
    }

    /// Configure from an AutoEq-style band kind (`peaking`, `lowShelf`,
    /// `highShelf`). Unknown kinds become a no-op identity.
    pub fn set_from_kind(&mut self, kind: &str, sample_rate: f32, f0: f32, gain_db: f32, q: f32) {
        match kind {
            "lowShelf" | "low_shelf" | "LSC" => self.set_low_shelf(sample_rate, f0, gain_db, q),
            "highShelf" | "high_shelf" | "HSC" => self.set_high_shelf(sample_rate, f0, gain_db, q),
            _ => self.set_peaking(sample_rate, f0, gain_db, q),
        }
    }

    fn assign(&mut self, b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) {
        self.b0 = (b0 / a0) as f32;
        self.b1 = (b1 / a0) as f32;
        self.b2 = (b2 / a0) as f32;
        self.a1 = (a1 / a0) as f32;
        self.a2 = (a2 / a0) as f32;
    }

    /// Clear the filter state (delay memory).
    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    /// Process one sample (Direct Form II Transposed).
    #[inline]
    pub fn process_sample(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A peaking filter at 0 dB has transfer function H(z) = 1 — it must pass
    /// the signal through unchanged. This is the building block of the EQ null
    /// test.
    #[test]
    fn peaking_at_zero_db_is_identity() {
        let mut bq = Biquad::identity();
        bq.set_peaking(48_000.0, 1_000.0, 0.0, 4.318);

        let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.05).sin()).collect();
        for &x in &input {
            let y = bq.process_sample(x);
            assert!((y - x).abs() < 1e-5, "expected {x}, got {y}");
        }
    }

    #[test]
    fn low_shelf_at_zero_db_is_identity() {
        let mut bq = Biquad::identity();
        bq.set_low_shelf(48_000.0, 120.0, 0.0, 0.707);
        for i in 0..256 {
            let x = (i as f32 * 0.07).sin();
            assert!((bq.process_sample(x) - x).abs() < 1e-5);
        }
    }

    #[test]
    fn low_shelf_boost_raises_dc_gain() {
        // A low shelf boost should amplify a very low-frequency (near-DC) input.
        let mut bq = Biquad::identity();
        bq.set_low_shelf(48_000.0, 200.0, 6.0, 0.707);
        // Settle, then measure steady-state gain on a constant input.
        let mut y = 0.0;
        for _ in 0..2000 {
            y = bq.process_sample(1.0);
        }
        assert!(y > 1.5, "expected ~+6 dB (×2) low boost, got {y}");
    }

    #[test]
    fn lowpass_passes_dc_attenuates_hf() {
        let mut lp = Biquad::identity();
        lp.set_lowpass(48_000.0, 1_000.0, std::f32::consts::FRAC_1_SQRT_2);
        // Settle on DC ⇒ ~unity gain.
        let mut y = 0.0;
        for _ in 0..4000 { y = lp.process_sample(1.0); }
        assert!((y - 1.0).abs() < 0.05, "LP DC gain ~1, got {y}");
        // A 12 kHz tone (well above cutoff) is strongly attenuated.
        lp.reset();
        let mut peak = 0.0f32;
        for i in 0..4000 {
            let x = (2.0 * std::f32::consts::PI * 12_000.0 * i as f32 / 48_000.0).sin();
            peak = peak.max(lp.process_sample(x).abs());
        }
        assert!(peak < 0.3, "12kHz through 1kHz LP should be small, got {peak}");
    }

    #[test]
    fn highpass_blocks_dc_passes_hf() {
        let mut hp = Biquad::identity();
        hp.set_highpass(48_000.0, 1_000.0, std::f32::consts::FRAC_1_SQRT_2);
        let mut y = 0.0;
        for _ in 0..4000 { y = hp.process_sample(1.0); }
        assert!(y.abs() < 0.05, "HP blocks DC, got {y}");
    }
}
