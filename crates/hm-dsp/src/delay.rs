//! A fixed integer-sample delay line for a single channel. Shared by the
//! spatializer (inter-aural delay) and the 3D-surround virtual speakers.

/// A ring-buffer delay line. Sized once for a maximum delay; reads an integer
/// number of samples in the past.
pub(crate) struct DelayLine {
    buf: Vec<f32>,
    idx: usize,
}

impl DelayLine {
    /// Allocate a line that can delay by up to `max_delay` samples.
    pub(crate) fn new(max_delay: usize) -> Self {
        Self {
            buf: vec![0.0; max_delay.max(1) + 1],
            idx: 0,
        }
    }

    /// Push `x` and return the sample written `delay` samples ago.
    #[inline]
    pub(crate) fn process(&mut self, x: f32, delay: usize) -> f32 {
        let n = self.buf.len();
        let delay = delay.min(n - 1);
        let read = (self.idx + n - delay) % n;
        let y = self.buf[read];
        self.buf[self.idx] = x;
        self.idx = if self.idx + 1 == n { 0 } else { self.idx + 1 };
        y
    }
}
