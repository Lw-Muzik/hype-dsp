# crates/hm-ytmusic/src/search.rs

- SearchFilter · enum · L50-L57 — pub enum SearchFilter
- parse · function · L65-L74 — pub fn parse(raw: &str) -> Self
- parse_search_page · function · L82-L84 — pub fn parse_search_page(json: &Value) -> Vec<ExploreShelf>
- parse_suggestions · function · L92-L97 — pub fn parse_suggestions(json: &Value) -> Vec<String>
- collect_suggestions · function · L99-L118 — fn collect_suggestions(v: &Value, out: &mut Vec<String>)
- tests · module · L121-L253 — mod tests
- names_map_onto_filters_and_anything_else_searches_everything · function · L127-L137 — fn names_map_onto_filters_and_anything_else_searches_everything()
- search_response · function · L141-L145 — fn search_response(shelf: Value) -> Value
- reads_an_artist_result_out_of_the_search_envelope · function · L148-L166 — fn reads_an_artist_result_out_of_the_search_envelope()
- one_unreadable_row_does_not_cost_the_others · function · L171-L193 — fn one_unreadable_row_does_not_cost_the_others()
- a_row_without_a_play_count_or_a_full_byline_is_still_a_result · function · L199-L217 — fn a_row_without_a_play_count_or_a_full_byline_is_still_a_result()
- a_response_it_understands_nothing_of_is_empty_not_an_error · function · L220-L229 — fn a_response_it_understands_nothing_of_is_empty_not_an_error()
- reads_suggestions_joining_the_halves_they_arrive_split_into · function · L232-L242 — fn reads_suggestions_joining_the_halves_they_arrive_split_into()
- suggestions_of_a_shape_it_cannot_read_are_none_not_an_error · function · L245-L252 — fn suggestions_of_a_shape_it_cannot_read_are_none_not_an_error()
