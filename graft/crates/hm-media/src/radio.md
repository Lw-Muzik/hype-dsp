# crates/hm-media/src/radio.rs

- SEED · constant · L12-L12 — const SEED: &str = include_str!("../data/radio_seed.json");
- HOSTS · constant · L13-L17 — const HOSTS: &[&str] = &[
- seed · function · L20-L22 — pub fn seed() -> Vec<RadioStation>
- RbStation · struct · L25-L33 — struct RbStation
- to_station · function · L35-L58 — fn to_station(s: RbStation) -> Option<RadioStation>
- search · function · L62-L69 — pub fn search(query: &str) -> Vec<RadioStation>
- seed_filtered · function · L71-L88 — fn seed_filtered(query: &str) -> Vec<RadioStation>
- by_country · function · L92-L94 — pub fn by_country(code: &str) -> Vec<RadioStation>
- by_country_remote · function · L96-L122 — fn by_country_remote(code: &str) -> Option<Vec<RadioStation>>
- search_remote · function · L124-L153 — fn search_remote(query: &str) -> Option<Vec<RadioStation>>
- urlencode · function · L155-L165 — fn urlencode(s: &str) -> String
- tests · module · L168-L185 — mod tests
- seed_parses · function · L172-L176 — fn seed_parses()
- seed_filter_matches_name · function · L179-L184 — fn seed_filter_matches_name()
