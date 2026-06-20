//! Internet radio directory + favorites commands.

use hm_core::{IpcError, MediaStore, RadioStation};
use hm_media::radio;
use serde::Serialize;
use tauri::State;

/// Search the radio directory (falls back to the bundled seed when offline).
#[tauri::command(async)]
pub fn radio_search(query: String) -> Vec<RadioStation> {
    radio::search(&query)
}

/// A country in the Africa browser (ISO 3166-1 alpha-2 code + English name).
#[derive(Clone, Serialize)]
pub struct RadioCountry {
    pub code: &'static str,
    pub name: &'static str,
}

/// The 54 African countries (alphabetical) — the frontend renders a flag from
/// the code and fetches each one's stations on demand via `radio_by_country`.
const AFRICAN_COUNTRIES: &[RadioCountry] = &[
    RadioCountry { code: "DZ", name: "Algeria" },
    RadioCountry { code: "AO", name: "Angola" },
    RadioCountry { code: "BJ", name: "Benin" },
    RadioCountry { code: "BW", name: "Botswana" },
    RadioCountry { code: "BF", name: "Burkina Faso" },
    RadioCountry { code: "BI", name: "Burundi" },
    RadioCountry { code: "CV", name: "Cabo Verde" },
    RadioCountry { code: "CM", name: "Cameroon" },
    RadioCountry { code: "CF", name: "Central African Republic" },
    RadioCountry { code: "TD", name: "Chad" },
    RadioCountry { code: "KM", name: "Comoros" },
    RadioCountry { code: "CG", name: "Congo" },
    RadioCountry { code: "CI", name: "Côte d'Ivoire" },
    RadioCountry { code: "CD", name: "DR Congo" },
    RadioCountry { code: "DJ", name: "Djibouti" },
    RadioCountry { code: "EG", name: "Egypt" },
    RadioCountry { code: "GQ", name: "Equatorial Guinea" },
    RadioCountry { code: "ER", name: "Eritrea" },
    RadioCountry { code: "SZ", name: "Eswatini" },
    RadioCountry { code: "ET", name: "Ethiopia" },
    RadioCountry { code: "GA", name: "Gabon" },
    RadioCountry { code: "GM", name: "Gambia" },
    RadioCountry { code: "GH", name: "Ghana" },
    RadioCountry { code: "GN", name: "Guinea" },
    RadioCountry { code: "GW", name: "Guinea-Bissau" },
    RadioCountry { code: "KE", name: "Kenya" },
    RadioCountry { code: "LS", name: "Lesotho" },
    RadioCountry { code: "LR", name: "Liberia" },
    RadioCountry { code: "LY", name: "Libya" },
    RadioCountry { code: "MG", name: "Madagascar" },
    RadioCountry { code: "MW", name: "Malawi" },
    RadioCountry { code: "ML", name: "Mali" },
    RadioCountry { code: "MR", name: "Mauritania" },
    RadioCountry { code: "MU", name: "Mauritius" },
    RadioCountry { code: "MA", name: "Morocco" },
    RadioCountry { code: "MZ", name: "Mozambique" },
    RadioCountry { code: "NA", name: "Namibia" },
    RadioCountry { code: "NE", name: "Niger" },
    RadioCountry { code: "NG", name: "Nigeria" },
    RadioCountry { code: "RW", name: "Rwanda" },
    RadioCountry { code: "ST", name: "São Tomé and Príncipe" },
    RadioCountry { code: "SN", name: "Senegal" },
    RadioCountry { code: "SC", name: "Seychelles" },
    RadioCountry { code: "SL", name: "Sierra Leone" },
    RadioCountry { code: "SO", name: "Somalia" },
    RadioCountry { code: "ZA", name: "South Africa" },
    RadioCountry { code: "SS", name: "South Sudan" },
    RadioCountry { code: "SD", name: "Sudan" },
    RadioCountry { code: "TZ", name: "Tanzania" },
    RadioCountry { code: "TG", name: "Togo" },
    RadioCountry { code: "TN", name: "Tunisia" },
    RadioCountry { code: "UG", name: "Uganda" },
    RadioCountry { code: "ZM", name: "Zambia" },
    RadioCountry { code: "ZW", name: "Zimbabwe" },
];

/// The African countries available in the radio browser.
#[tauri::command]
pub fn radio_african_countries() -> Vec<RadioCountry> {
    AFRICAN_COUNTRIES.to_vec()
}

/// Every station for a country (ISO alpha-2 code), most-popular first.
#[tauri::command(async)]
pub fn radio_by_country(code: String) -> Vec<RadioStation> {
    radio::by_country(&code)
}

/// Favorited stations (persisted).
#[tauri::command]
pub fn radio_favorites_list(store: State<'_, MediaStore>) -> Result<Vec<RadioStation>, IpcError> {
    store.list_favorites().map_err(Into::into)
}

#[tauri::command]
pub fn radio_favorite_add(
    store: State<'_, MediaStore>,
    station: RadioStation,
) -> Result<(), IpcError> {
    store.add_favorite(&station).map_err(Into::into)
}

#[tauri::command]
pub fn radio_favorite_remove(store: State<'_, MediaStore>, id: String) -> Result<(), IpcError> {
    store.remove_favorite(&id).map_err(Into::into)
}
