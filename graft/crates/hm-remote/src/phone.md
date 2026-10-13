# crates/hm-remote/src/phone.rs

- phone_dns_resolver · function · L33-L52 — pub(crate) fn phone_dns_resolver() -> Option<iroh::dns::DnsResolver>
- NAMESERVERS · constant · L38-L43 — const NAMESERVERS: [IpAddr; 4] = [
- PhoneNode · struct · L55-L58 — pub struct PhoneNode
- start · function · L64-L87 — pub fn start(secret_path: PathBuf, shelf_port: u16) -> Result<Self>
- endpoint_id · function · L90-L92 — pub fn endpoint_id(&self) -> String
- pair · function · L98-L107 — pub fn pair(&self, desktop_ep: &str, pin: &str, name: &str, token: &str) -> Result<String>
