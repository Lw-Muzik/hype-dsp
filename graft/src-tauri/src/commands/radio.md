# src-tauri/src/commands/radio.rs

- radio_search · function · L10-L14 — pub fn radio_search(query: String) -> Vec<RadioStation>
- RadioCountry · struct · L18-L21 — pub struct RadioCountry
- AFRICAN_COUNTRIES · constant · L25-L80 — const AFRICAN_COUNTRIES: &[RadioCountry] = &[
- radio_african_countries · function · L84-L86 — pub fn radio_african_countries() -> Vec<RadioCountry>
- radio_by_country · function · L90-L94 — pub fn radio_by_country(code: String) -> Vec<RadioStation>
- radio_favorites_list · function · L101-L105 — pub fn radio_favorites_list(store: State<'_, MediaStore>) -> Result<Vec<RadioStation>, IpcError>
- radio_favorite_add · function · L108-L115 — pub fn radio_favorite_add(
- radio_favorite_remove · function · L118-L122 — pub fn radio_favorite_remove(store: State<'_, MediaStore>, id: String) -> Result<(), IpcError>
