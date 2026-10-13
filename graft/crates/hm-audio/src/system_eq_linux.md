# crates/hm-audio/src/system_eq_linux.rs

- Backend · enum · L35-L42 — enum Backend
- classify_stack · function · L48-L56 — fn classify_stack(pipewire_running: bool, pulse_usable: bool) -> Backend
- detect_backend · function · L58-L63 — fn detect_backend() -> Backend
- available · function · L66-L68 — pub fn available() -> bool
- LinuxSystemEq · enum · L72-L75 — pub enum LinuxSystemEq
- start · function · L80-L88 — pub fn start(state: Arc<ArcSwap<EngineState>>) -> Result<Self, AudioError>
- tests · module · L92-L112 — mod tests
- prefers_pipewire_even_when_pulse_looks_usable · function · L96-L101 — fn prefers_pipewire_even_when_pulse_looks_usable()
- falls_back_to_pulse_only_without_pipewire · function · L104-L106 — fn falls_back_to_pulse_only_without_pipewire()
- unavailable_when_neither_is_present · function · L109-L111 — fn unavailable_when_neither_is_present()
