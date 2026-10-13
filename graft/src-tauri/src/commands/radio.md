# src-tauri/src/commands/radio.rs

- radio_search · function · L10-L12 — pub fn radio_search(query: String) -> Vec<RadioStation>
- RadioCountry · struct · L16-L19 — pub struct RadioCountry
- AFRICAN_COUNTRIES · constant · L23-L78 — const AFRICAN_COUNTRIES: &[RadioCountry] = &[
- radio_african_countries · function · L82-L84 — pub fn radio_african_countries() -> Vec<RadioCountry>
- radio_by_country · function · L88-L90 — pub fn radio_by_country(code: String) -> Vec<RadioStation>
- radio_favorites_list · function · L97-L99 — pub fn radio_favorites_list(store: State<'_, MediaStore>) -> Result<Vec<RadioStation>, IpcError>
- radio_favorite_add · function · L102-L107 — pub fn radio_favorite_add(
- radio_favorite_remove · function · L110-L112 — pub fn radio_favorite_remove(store: State<'_, MediaStore>, id: String) -> Result<(), IpcError>
