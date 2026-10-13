# src-tauri/src/control.rs

- NowPlaying · struct · L23-L27 — struct NowPlaying
- start · function · L32-L51 — pub fn start(app: AppHandle) -> Option<u16>
- handle · function · L53-L73 — fn handle(app: &AppHandle, request: Request)
- handle_cast · function · L75-L111 — fn handle_cast(app: &AppHandle, mut request: Request, token: &str)
- handle_transport · function · L113-L127 — fn handle_transport(app: &AppHandle, mut request: Request, token: &str)
- handle_now · function · L129-L141 — fn handle_now(app: &AppHandle, request: Request, token: &str)
- bearer · function · L145-L154 — fn bearer(request: &Request) -> String
- MAX_BODY_BYTES · constant · L158-L158 — const MAX_BODY_BYTES: u64 = 64 * 1024;
- read_body · function · L160-L166 — fn read_body(request: &mut Request) -> String
- respond_json · function · L168-L172 — fn respond_json(request: Request, body: &str)
- unauthorized · function · L174-L176 — fn unauthorized(request: Request)
