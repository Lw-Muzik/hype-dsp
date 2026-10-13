# crates/hm-stems/src/onnx.rs

- SEGMENT · constant · L22-L22 — pub const SEGMENT: usize = 343_980;
- Contract · struct · L25-L32 — struct Contract
- default_input · function · L33-L35 — fn default_input() -> String
- default_output · function · L36-L38 — fn default_output() -> String
- default_segment · function · L39-L41 — fn default_segment() -> usize
- default · function · L43-L49 — fn default() -> Self
- coreml_available · function · L54-L58 — pub fn coreml_available() -> bool
- intra_threads · function · L64-L70 — fn intra_threads() -> usize
- Model · struct · L73-L81 — pub struct Model
- load · function · L88-L133 — pub fn load(path: &Path, target_row: usize) -> Result<Self, StemError>
- run_segment · function · L138-L159 — pub fn run_segment(&mut self, planar: &[f32]) -> Result<Vec<f32>, StemError>
- with_json_ext · function · L162-L166 — fn with_json_ext(path: &Path) -> std::path::PathBuf
- ort_err · function · L168-L170 — fn ort_err(e: impl std::fmt::Display) -> StemError
