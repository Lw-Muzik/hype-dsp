# crates/hm-audio/src/sources.rs

- FilePlaybackSource · struct · L9-L13 — pub struct FilePlaybackSource
- new · function · L17-L22 — pub fn new(stereo_samples: Vec<f32>) -> Self
- len_frames · function · L24-L26 — fn len_frames(&self) -> usize
- is_finished · function · L29-L31 — pub fn is_finished(&self) -> bool
- start · function · L35-L37 — fn start(&mut self, _format: StreamFormat) -> Result<(), AudioError>
- read · function · L39-L69 — fn read(&mut self, out: &mut [f32], channels: usize) -> usize
- stop · function · L71-L73 — fn stop(&mut self)
- seek · function · L75-L77 — fn seek(&mut self, frame: usize)
- position · function · L79-L81 — fn position(&self) -> usize
- total_frames · function · L83-L85 — fn total_frames(&self) -> usize
- tests · module · L89-L112 — mod tests
- reads_stereo_frames_then_silence · function · L93-L102 — fn reads_stereo_frames_then_silence()
- downmixes_to_mono · function · L105-L111 — fn downmixes_to_mono()
