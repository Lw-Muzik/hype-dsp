# src-tauri/src/commands/tv.rs

- cache_dir · function · L20-L22 — fn cache_dir(app: &AppHandle) -> Option<PathBuf>
- tv_search · function · L26-L28 — pub fn tv_search(app: AppHandle, query: String) -> Vec<TvChannel>
- tv_check_alive · function · L37-L42 — pub fn tv_check_alive(
- tv_by_country · function · L46-L48 — pub fn tv_by_country(app: AppHandle, code: String) -> Vec<TvChannel>
- tv_by_category · function · L52-L54 — pub fn tv_by_category(app: AppHandle, id: String) -> Vec<TvChannel>
- tv_categories · function · L58-L60 — pub fn tv_categories() -> Vec<TvCategory>
- tv_countries · function · L64-L66 — pub fn tv_countries() -> Vec<TvCountry>
- tv_favorites_list · function · L72-L74 — pub fn tv_favorites_list(store: State<'_, MediaStore>) -> Result<Vec<TvChannel>, IpcError>
- tv_favorite_add · function · L77-L79 — pub fn tv_favorite_add(store: State<'_, MediaStore>, channel: TvChannel) -> Result<(), IpcError>
- tv_favorite_remove · function · L82-L84 — pub fn tv_favorite_remove(store: State<'_, MediaStore>, id: String) -> Result<(), IpcError>
- tv_stream_url · function · L90-L96 — pub fn tv_stream_url(proxy: State<'_, TvProxy>, channel: TvChannel) -> Result<String, IpcError>
