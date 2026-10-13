# src-tauri/src/commands/tv.rs

- cache_dir · function · L20-L22 — fn cache_dir(app: &AppHandle) -> Option<PathBuf>
- tv_search · function · L26-L30 — pub fn tv_search(app: AppHandle, query: String) -> Vec<TvChannel>
- tv_check_alive · function · L39-L46 — pub fn tv_check_alive(
- tv_by_country · function · L50-L54 — pub fn tv_by_country(app: AppHandle, code: String) -> Vec<TvChannel>
- tv_by_category · function · L58-L62 — pub fn tv_by_category(app: AppHandle, id: String) -> Vec<TvChannel>
- tv_categories · function · L66-L68 — pub fn tv_categories() -> Vec<TvCategory>
- tv_countries · function · L72-L74 — pub fn tv_countries() -> Vec<TvCountry>
- tv_favorites_list · function · L80-L84 — pub fn tv_favorites_list(store: State<'_, MediaStore>) -> Result<Vec<TvChannel>, IpcError>
- tv_favorite_add · function · L87-L91 — pub fn tv_favorite_add(store: State<'_, MediaStore>, channel: TvChannel) -> Result<(), IpcError>
- tv_favorite_remove · function · L94-L98 — pub fn tv_favorite_remove(store: State<'_, MediaStore>, id: String) -> Result<(), IpcError>
- tv_stream_url · function · L104-L110 — pub fn tv_stream_url(proxy: State<'_, TvProxy>, channel: TvChannel) -> Result<String, IpcError>
