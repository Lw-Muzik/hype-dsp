# crates/hm-audio/src/capture.rs

- LoopbackCaptureSource · struct · L25-L30 — pub struct LoopbackCaptureSource
- new · function · L34-L58 — pub fn new(device_rate: u32) -> Result<Self, AudioError>
- drop · function · L62-L64 — fn drop(&mut self)
- start · function · L68-L70 — fn start(&mut self, _format: StreamFormat) -> Result<(), AudioError>
- read · function · L72-L102 — fn read(&mut self, out: &mut [f32], channels: usize) -> usize
- stop · function · L104-L106 — fn stop(&mut self)
- position · function · L108-L110 — fn position(&self) -> usize
- is_live · function · L112-L114 — fn is_live(&self) -> bool
- capture_loop · function · L117-L154 — fn capture_loop(mut producer: rtrb::Producer<f32>, running: &AtomicBool)
- pick_f32_input_config · function · L156-L184 — fn pick_f32_input_config(device: &cpal::Device) -> Result<cpal::SupportedStreamConfig, AudioError>
- VirtualDeviceSource · struct · L188-L188 — pub struct VirtualDeviceSource;
- start · function · L191-L195 — fn start(&mut self, _format: StreamFormat) -> Result<(), AudioError>
- read · function · L196-L198 — fn read(&mut self, _out: &mut [f32], _channels: usize) -> usize
- stop · function · L199-L199 — fn stop(&mut self) {}
- is_live · function · L200-L202 — fn is_live(&self) -> bool
- virtual_device_available · function · L207-L209 — pub fn virtual_device_available() -> bool
