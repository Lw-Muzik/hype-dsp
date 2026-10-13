//! The progressively filled result of separating one track.
//!
//! The mix is known up front; vocals and drums arrive chunk by chunk (any
//! order, from a worker thread) and are read lock-free by the audio thread.
//! Instruments are never stored: they are `mix − vocals − drums`, which is what
//! makes unity gains reproduce the original exactly and halves the memory.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use crate::chunk::ChunkOut;
use crate::config::{chunk_containing, chunk_count, owned_end, owned_start};

pub struct StemStore {
    /// Interleaved stereo at 44.1 kHz, the whole track.
    mix: Vec<f32>,
    chunks: Vec<OnceLock<ChunkOut>>,
    ready: AtomicUsize,
    /// Model-domain frame the listener is at; the worker separates from here.
    wanted: AtomicUsize,
}

/// A borrowed run of separated samples starting at `from` (model frames).
pub struct Run<'a> {
    pub from: usize,
    pub vocals: &'a [f32],
    pub drums: &'a [f32],
}

impl StemStore {
    pub fn new(mix: Vec<f32>) -> Arc<Self> {
        let n = chunk_count(mix.len() / 2);
        Arc::new(Self {
            chunks: (0..n).map(|_| OnceLock::new()).collect(),
            mix,
            ready: AtomicUsize::new(0),
            wanted: AtomicUsize::new(0),
        })
    }

    pub fn mix(&self) -> &[f32] {
        &self.mix
    }

    pub fn frames(&self) -> usize {
        self.mix.len() / 2
    }

    pub fn chunk_total(&self) -> usize {
        self.chunks.len()
    }

    pub fn ready_count(&self) -> usize {
        self.ready.load(Ordering::Acquire)
    }

    pub fn is_complete(&self) -> bool {
        self.ready_count() == self.chunks.len()
    }

    pub fn is_ready(&self, c: usize) -> bool {
        self.chunks.get(c).is_some_and(|s| s.get().is_some())
    }

    /// Store chunk `c`'s result. Returns false if it was already there or the
    /// result has the wrong length (a bug upstream; never published).
    pub fn publish(&self, c: usize, out: ChunkOut) -> bool {
        let Some(slot) = self.chunks.get(c) else { return false };
        let want = (owned_end(c).min(self.frames()) - owned_start(c)) * 2;
        if out.vocals.len() != want || out.drums.len() != want {
            return false;
        }
        if slot.set(out).is_ok() {
            self.ready.fetch_add(1, Ordering::Release);
            true
        } else {
            false
        }
    }

    pub fn set_wanted(&self, frame: usize) {
        self.wanted.store(frame.min(self.frames()), Ordering::Relaxed);
    }

    pub fn wanted(&self) -> usize {
        self.wanted.load(Ordering::Relaxed)
    }

    /// The next chunk to work on: the first one not ready at or after the
    /// listener's chunk, wrapping to the start of the track.
    pub fn next_missing(&self) -> Option<usize> {
        let n = self.chunks.len();
        if n == 0 {
            return None;
        }
        let from = chunk_containing(self.wanted().min(self.frames().saturating_sub(1)));
        (0..n).map(|i| (from + i) % n).find(|&c| !self.is_ready(c))
    }

    /// The separated samples at `frame` through the end of its chunk, or `None`
    /// if that chunk isn't ready (play the original there).
    pub fn run_at(&self, frame: usize) -> Option<Run<'_>> {
        if frame >= self.frames() {
            return None;
        }
        let c = chunk_containing(frame);
        let out = self.chunks.get(c)?.get()?;
        let offset = frame - owned_start(c);
        Some(Run {
            from: frame,
            vocals: &out.vocals[offset * 2..],
            drums: &out.drums[offset * 2..],
        })
    }

    /// Fill every chunk from whole-track vocals and drums (a cache hit).
    pub fn fill_from(&self, vocals: &[f32], drums: &[f32]) -> bool {
        if vocals.len() != self.mix.len() || drums.len() != self.mix.len() {
            return false;
        }
        for c in 0..self.chunks.len() {
            let lo = owned_start(c) * 2;
            let hi = owned_end(c).min(self.frames()) * 2;
            self.publish(
                c,
                ChunkOut {
                    vocals: vocals[lo..hi].to_vec(),
                    drums: drums[lo..hi].to_vec(),
                },
            );
        }
        true
    }

    /// Whole-track vocals and drums once complete.
    pub fn assemble(&self) -> Option<(Vec<f32>, Vec<f32>)> {
        if !self.is_complete() {
            return None;
        }
        let mut v = Vec::with_capacity(self.mix.len());
        let mut d = Vec::with_capacity(self.mix.len());
        for slot in &self.chunks {
            let c = slot.get()?;
            v.extend_from_slice(&c.vocals);
            d.extend_from_slice(&c.drums);
        }
        Some((v, d))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::STEP;

    fn out(c: usize, frames: usize, value: f32) -> ChunkOut {
        let n = (owned_end(c).min(frames) - owned_start(c)) * 2;
        ChunkOut { vocals: vec![value; n], drums: vec![-value; n] }
    }

    #[test]
    fn publishes_and_serves_runs() {
        let frames = STEP * 2;
        let store = StemStore::new(vec![0.0; frames * 2]);
        assert_eq!(store.chunk_total(), 2);
        assert!(store.run_at(10).is_none(), "nothing ready yet");
        assert!(store.publish(0, out(0, frames, 0.5)));
        assert!(!store.publish(0, out(0, frames, 0.5)), "no double publish");
        let run = store.run_at(100).unwrap();
        assert_eq!(run.vocals[0], 0.5);
        assert_eq!(run.drums[0], -0.5);
        assert_eq!(run.vocals.len(), (owned_end(0) - 100) * 2);
        assert!(store.run_at(owned_end(0)).is_none(), "next chunk not ready");
    }

    #[test]
    fn wrong_length_is_refused() {
        let store = StemStore::new(vec![0.0; 2000]);
        assert!(!store.publish(0, ChunkOut { vocals: vec![0.0; 10], drums: vec![0.0; 10] }));
        assert_eq!(store.ready_count(), 0);
    }

    #[test]
    fn worker_follows_the_playhead_then_wraps() {
        let frames = STEP * 5;
        let store = StemStore::new(vec![0.0; frames * 2]);
        let n = store.chunk_total();
        store.set_wanted(owned_start(3) + 10);
        assert_eq!(store.next_missing(), Some(3));
        store.publish(3, out(3, frames, 0.1));
        assert_eq!(store.next_missing(), Some(4));
        store.publish(4, out(4, frames, 0.1));
        assert_eq!(store.next_missing(), Some(5 % n), "then onward to the end of the track");
        // the listener seeks back: the worker jumps there.
        store.set_wanted(0);
        assert_eq!(store.next_missing(), Some(0));
        for c in 0..n {
            store.publish(c, out(c, frames, 0.1));
        }
        assert_eq!(store.next_missing(), None);
        assert!(store.is_complete());
    }

    #[test]
    fn fill_from_cache_round_trips_through_assemble() {
        let frames = STEP + 40_000;
        let mix: Vec<f32> = (0..frames * 2).map(|i| i as f32).collect();
        let store = StemStore::new(mix);
        let v: Vec<f32> = (0..frames * 2).map(|i| i as f32 * 0.5).collect();
        let d: Vec<f32> = (0..frames * 2).map(|i| -(i as f32)).collect();
        assert!(store.fill_from(&v, &d));
        let (v2, d2) = store.assemble().unwrap();
        assert_eq!(v, v2);
        assert_eq!(d, d2);
        assert!(!store.fill_from(&v[..10], &d[..10]), "wrong length refused");
    }
}
