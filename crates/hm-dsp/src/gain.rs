//! Makeup gain stage. Applies `output.gain_db` as a linear multiplier ahead of
//! the limiter, so boosted volume is caught by the brickwall before output.

use crate::{AudioProcessor, ProcessorParams};

/// A simple wideband gain.
pub struct Gain {
    gain_lin: f32,
}

impl Default for Gain {
    fn default() -> Self {
        Self::new()
    }
}

impl Gain {
    /// Unity-gain stage.
    pub fn new() -> Self {
        Self { gain_lin: 1.0 }
    }
}

impl AudioProcessor for Gain {
    fn prepare(&mut self, _sample_rate: f32, _channels: usize) {}

    fn process(&mut self, buffer: &mut [f32], _channels: usize) {
        if (self.gain_lin - 1.0).abs() < f32::EPSILON {
            return;
        }
        for sample in buffer.iter_mut() {
            *sample *= self.gain_lin;
        }
    }

    fn set_params(&mut self, params: &ProcessorParams) {
        self.gain_lin = 10f32.powf(params.output.gain_db / 20.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hm_core::{EngineState, OutputState};

    #[test]
    fn applies_plus_six_db_as_linear_doubling() {
        let state = EngineState {
            output: OutputState {
                gain_db: 6.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut gain = Gain::new();
        gain.set_params(&state);

        let mut buffer = vec![0.5f32; 64];
        gain.process(&mut buffer, 2);

        // +6.0206 dB == ×2; +6 dB ≈ ×1.995.
        for &y in &buffer {
            assert!((y - 0.5 * 1.995).abs() < 0.01, "got {y}");
        }
    }

    #[test]
    fn zero_db_is_identity() {
        let mut gain = Gain::new();
        gain.set_params(&EngineState::default());
        let input = vec![0.3f32, -0.7, 0.1, -0.2];
        let mut buffer = input.clone();
        gain.process(&mut buffer, 2);
        assert_eq!(buffer, input);
    }
}
