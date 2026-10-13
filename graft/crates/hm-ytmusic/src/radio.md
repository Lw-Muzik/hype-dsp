# crates/hm-ytmusic/src/radio.rs

- RadioBatch · struct · L32-L35 — pub struct RadioBatch
- parse_radio_page · function · L38-L45 — pub(crate) fn parse_radio_page(json: &Value) -> RadioBatch
- collect_rows · function · L48-L68 — fn collect_rows<'a>(v: &'a Value, out: &mut Vec<&'a Value>)
- parse_row · function · L71-L130 — fn parse_row(r: &Value) -> Option<YtTrack>
- next_radio_token · function · L135-L152 — fn next_radio_token(json: &Value) -> Option<String>
- walk · function · L136-L150 — fn walk(v: &Value) -> Option<String>
- tests · module · L155-L264 — mod tests
- page · function · L158-L160 — fn page() -> serde_json::Value
- continuation · function · L161-L163 — fn continuation() -> serde_json::Value
- parses_the_rows_and_the_token · function · L166-L171 — fn parses_the_rows_and_the_token()
- skips_the_selected_seed_row · function · L176-L179 — fn skips_the_selected_seed_row()
- reads_a_wrapper_once_not_twice · function · L184-L202 — fn reads_a_wrapper_once_not_twice()
- maps_the_byline_album_by_its_album_link_not_by_position · function · L205-L222 — fn maps_the_byline_album_by_its_album_link_not_by_position()
- skips_unplayable_and_malformed_rows_without_failing_the_page · function · L225-L233 — fn skips_unplayable_and_malformed_rows_without_failing_the_page()
- fills_the_track_fields_the_queue_needs · function · L236-L248 — fn fills_the_track_fields_the_queue_needs()
- parses_a_continuation_page_with_the_same_parser · function · L251-L256 — fn parses_a_continuation_page_with_the_same_parser()
- a_page_with_no_token_reports_none_not_a_panic · function · L259-L263 — fn a_page_with_no_token_reports_none_not_a_panic()
