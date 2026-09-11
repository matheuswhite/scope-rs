//! Unit tests for [`super::Screen`] and its helpers.
//!
//! They live in their own file rather than at the bottom of `screen.rs` so the
//! source file holds only the screen itself. Being a child module of `screen`,
//! this still reaches its private fields and associated functions, which is
//! what most of these tests assert on.

use super::{Screen, ScreenMode, ScreenPosition, SearchHit, SearchMatcher};
use crate::graphics::buffer::{Buffer, BufferLine, BufferPosition};
use crate::graphics::selection::Selection;
use chrono::Local;
use ratatui::layout::Rect;
use ratatui::style::Color;

/// A buffer holding `n` lines at a capacity of `n` — so it is full, and the
/// next line evicts the oldest. Each line is labelled with its ordinal
/// (`L000`, `L001`, …) so a test can tell which lines it is looking at.
fn full_buffer(n: usize) -> Buffer {
    let mut buffer = Buffer::new(n.max(1));
    for i in 0..n {
        buffer += BufferLine::new_rx(Local::now(), format!("L{i:03}").into_bytes());
    }
    buffer
}

fn push(buffer: &mut Buffer, text: &str) {
    *buffer += BufferLine::new_rx(Local::now(), text.as_bytes().to_vec());
}

fn ids(buffer: &Buffer) -> Vec<u64> {
    buffer.iter().map(|line| line.id).collect()
}

/// A screen `height` rows tall (so `height - 2` content rows) sized for
/// `buffer`, with the viewport still at the top of the history — the state
/// right after a resize, before any line has landed.
fn sized_screen(buffer: &Buffer, height: u16) -> Screen {
    let mut screen = Screen::default();
    screen.set_size(
        Rect {
            x: 0,
            y: 0,
            width: 80,
            height,
        },
        buffer.len(),
    );
    screen
}

/// Like [`sized_screen`], but already reconciled with `buffer` and therefore
/// sitting at the bottom, following the newest line.
fn screen_on(buffer: &Buffer, height: u16) -> Screen {
    let mut screen = sized_screen(buffer, height);
    screen.update_after_new_lines(buffer);
    screen
}

/// The messages the viewport currently shows, top to bottom.
fn visible(screen: &Screen, buffer: &Buffer) -> Vec<String> {
    let start = screen.position.line;
    let rows = screen.size.height.saturating_sub(2) as usize;

    buffer
        .get_range(start, start + rows)
        .map(|line| String::from_utf8_lossy(&line.message).to_string())
        .collect()
}

fn hit(line: usize, line_id: u64) -> SearchHit {
    SearchHit {
        position: BufferPosition { line, column: 0 },
        line_id,
    }
}

/// A search over `hits`, sitting on the one given by `current`.
fn searching_on(current: usize, hits: &[SearchHit]) -> Screen {
    let mut screen = Screen::default();
    screen.change_mode_to_search("x".to_string(), true, false);
    for hit in hits {
        screen.mode_mut().add_entry(*hit);
    }
    if let ScreenMode::Search { current: c, .. } = screen.mode_mut() {
        *c = current;
    }
    screen
}

// ---------------------------------------------------------------------------
// Regex search on each line (issue #209).
// ---------------------------------------------------------------------------

#[test]
fn plain_case_sensitive_finds_all_occurrences() {
    let matcher = SearchMatcher::build("ab", true, false);
    assert_eq!(matcher.matches("ab_ab_AB"), vec![(0, 2), (3, 2)]);
}

#[test]
fn plain_case_insensitive_matches_regardless_of_case() {
    let matcher = SearchMatcher::build("ab", false, false);
    assert_eq!(matcher.matches("ab_ab_AB"), vec![(0, 2), (3, 2), (6, 2)]);
}

#[test]
fn columns_are_character_offsets_not_bytes() {
    // "á" is two bytes but one column; the two "X" matches must land on
    // char columns 1 and 3, not byte offsets 2 and 5.
    let matcher = SearchMatcher::build("X", true, false);
    assert_eq!(matcher.matches("áXbX"), vec![(1, 1), (3, 1)]);
}

#[test]
fn regex_matches_pattern_with_char_columns_and_lengths() {
    let matcher = SearchMatcher::build(r"\d+", true, true);
    assert_eq!(matcher.matches("ab12cde345"), vec![(2, 2), (7, 3)]);
}

#[test]
fn regex_case_insensitive_flag_is_honored() {
    let sensitive = SearchMatcher::build("ERR", true, true);
    assert!(sensitive.matches("an err happened").is_empty());

    let insensitive = SearchMatcher::build("ERR", false, true);
    assert_eq!(insensitive.matches("an err happened"), vec![(3, 3)]);
}

#[test]
fn regex_anchor_matches_only_at_line_start() {
    let matcher = SearchMatcher::build("^ab", true, true);
    assert_eq!(matcher.matches("abcab"), vec![(0, 2)]);
    assert!(matcher.matches("xabcab").is_empty());
}

#[test]
fn regex_zero_width_matches_are_skipped() {
    // A trailing `.*` and empty `a*` runs would otherwise inflate the match
    // count with nothing to highlight.
    let matcher = SearchMatcher::build("a*", true, true);
    assert_eq!(matcher.matches("baa"), vec![(1, 2)]);
}

#[test]
fn invalid_regex_matches_nothing() {
    let matcher = SearchMatcher::build("(unclosed", true, true);
    assert!(matcher.is_empty());
    assert!(matcher.matches("(unclosed group here").is_empty());
}

#[test]
fn empty_query_matches_nothing() {
    assert!(SearchMatcher::build("", false, false).is_empty());
    assert!(SearchMatcher::build("", false, true).is_empty());
}

// ---------------------------------------------------------------------------
// A viewport frozen by scrolling up must stay on its lines while the buffer
// rotates underneath it (issue #218).
// ---------------------------------------------------------------------------

#[test]
fn a_frozen_viewport_keeps_showing_the_same_lines() {
    // 20 lines over 7 content rows: the bottom offset is 13.
    let mut buffer = full_buffer(20);
    let mut screen = screen_on(&buffer, 9);
    assert!(screen.auto_scroll);

    // Scroll up into the middle of the history.
    screen.scroll_vertical(-8, 13);
    assert!(!screen.auto_scroll);
    let frozen = visible(&screen, &buffer);
    assert_eq!(frozen.first().unwrap(), "L005");

    // Three new lines evict L000..L002 and re-index the rest, so the
    // offset has to come down by three to stay on the same content.
    for text in ["N1", "N2", "N3"] {
        push(&mut buffer, text);
    }
    screen.update_after_new_lines(&buffer);

    assert_eq!(visible(&screen, &buffer), frozen);
    assert_eq!(screen.position.line, 2);
}

#[test]
fn auto_scroll_still_follows_the_bottom_through_evictions() {
    let mut buffer = full_buffer(20);
    let mut screen = screen_on(&buffer, 9);

    push(&mut buffer, "N1");
    screen.update_after_new_lines(&buffer);

    assert!(screen.auto_scroll);
    assert_eq!(visible(&screen, &buffer).last().unwrap(), "N1");
}

#[test]
fn the_top_of_the_history_is_the_floor() {
    // At offset 0 the lines on screen are the ones being evicted: there
    // is nothing above to slide to, so here the content does move.
    let mut buffer = full_buffer(20);
    let mut screen = screen_on(&buffer, 9);
    screen.disable_auto_scroll();
    screen.jump_to_start();

    push(&mut buffer, "N1");
    screen.update_after_new_lines(&buffer);

    assert_eq!(screen.position.line, 0);
    assert_eq!(visible(&screen, &buffer).first().unwrap(), "L001");
}

#[test]
fn a_selection_slides_with_the_content() {
    let mut buffer = full_buffer(20);
    let mut screen = screen_on(&buffer, 9);
    screen.selection = Some(Selection::new(
        BufferPosition {
            line: 10,
            column: 1,
        },
        BufferPosition {
            line: 12,
            column: 4,
        },
    ));

    push(&mut buffer, "N1");
    screen.update_after_new_lines(&buffer);

    let selection = screen.selection.expect("the selection is still valid");
    assert_eq!(selection.start, BufferPosition { line: 9, column: 1 });
    assert_eq!(
        selection.end,
        BufferPosition {
            line: 11,
            column: 4
        }
    );
}

#[test]
fn a_selection_whose_lines_are_all_evicted_is_dropped() {
    let mut buffer = full_buffer(20);
    let mut screen = screen_on(&buffer, 9);
    screen.selection = Some(Selection::new(
        BufferPosition { line: 0, column: 0 },
        BufferPosition { line: 1, column: 2 },
    ));

    for text in ["N1", "N2", "N3"] {
        push(&mut buffer, text);
    }
    screen.update_after_new_lines(&buffer);

    assert!(screen.selection.is_none());
}

#[test]
fn clearing_resets_the_eviction_bookkeeping() {
    let mut buffer = full_buffer(20);
    let mut screen = screen_on(&buffer, 9);
    push(&mut buffer, "N1");
    screen.update_after_new_lines(&buffer);
    assert_eq!(screen.evicted_seen, 1);

    buffer.clear();
    screen.clear();

    assert_eq!(screen.evicted_seen, 0);
    assert_eq!(buffer.evicted(), 0);
}

// ---------------------------------------------------------------------------
// The hit list is re-scanned whenever new lines land, and must come back
// pointing at the hit the user navigated to even though the buffer re-indexed
// its lines underneath it (issue #218).
// ---------------------------------------------------------------------------

#[test]
fn a_rescan_stays_on_the_same_hit_after_the_lines_slid_up() {
    let mut screen = searching_on(1, &[hit(4, 104), hit(9, 109), hit(15, 115)]);

    let previous = screen.mode_mut().take_hits_for_rescan();
    assert_eq!(screen.search_indexes(), Some((1, 0)));

    // Three lines were evicted, so every hit is three indices lower.
    for h in [hit(1, 104), hit(6, 109), hit(12, 115)] {
        screen.mode_mut().add_entry(h);
    }
    screen.mode_mut().restore_current(previous);

    assert_eq!(screen.search_indexes(), Some((1, 3)));
}

#[test]
fn new_hits_appended_below_do_not_move_the_position() {
    let mut screen = searching_on(1, &[hit(4, 104), hit(9, 109)]);

    let previous = screen.mode_mut().take_hits_for_rescan();
    for h in [hit(4, 104), hit(9, 109), hit(20, 130)] {
        screen.mode_mut().add_entry(h);
    }
    screen.mode_mut().restore_current(previous);

    assert_eq!(screen.search_indexes(), Some((1, 3)));
}

#[test]
fn a_hit_scrolled_out_of_the_history_falls_to_the_next_one_in_time() {
    let mut screen = searching_on(0, &[hit(0, 104), hit(5, 109)]);

    let previous = screen.mode_mut().take_hits_for_rescan();
    // L104 is gone; ids grow monotonically, so 109 is the next hit in
    // time and the closest thing to where the user was.
    for h in [hit(2, 109), hit(8, 115)] {
        screen.mode_mut().add_entry(h);
    }
    screen.mode_mut().restore_current(previous);

    assert_eq!(screen.search_indexes(), Some((0, 2)));
}

#[test]
fn a_hit_past_the_end_of_the_new_list_falls_back_to_the_first() {
    let mut screen = searching_on(1, &[hit(0, 104), hit(5, 109)]);

    let previous = screen.mode_mut().take_hits_for_rescan();
    // The filter changed and only an older line still matches.
    screen.mode_mut().add_entry(hit(0, 100));
    screen.mode_mut().restore_current(previous);

    assert_eq!(screen.search_indexes(), Some((0, 1)));
}

#[test]
fn a_rescan_that_finds_nothing_leaves_a_valid_position() {
    let mut screen = searching_on(1, &[hit(0, 104), hit(5, 109)]);

    let previous = screen.mode_mut().take_hits_for_rescan();
    screen.mode_mut().restore_current(previous);

    assert_eq!(screen.search_indexes(), Some((0, 0)));
}

#[test]
fn an_empty_query_is_not_worth_rescanning() {
    let mut screen = Screen::default();
    assert!(!screen.is_searching());

    screen.change_mode_to_search(String::new(), true, false);
    assert!(!screen.is_searching());

    screen.change_mode_to_search("x".to_string(), true, false);
    assert!(screen.is_searching());
}

// ---------------------------------------------------------------------------
// Bookmarks (issue #208).
// ---------------------------------------------------------------------------

#[test]
fn right_click_toggles_the_line_under_the_cursor() {
    let buffer = full_buffer(5);
    let ids = ids(&buffer);
    let mut screen = sized_screen(&buffer, 10);

    // Content starts one row below the top border, so line 2 is at y=3.
    screen.toggle_bookmark(&buffer, ScreenPosition { x: 0, y: 3 });
    assert!(screen.bookmarks.contains(&ids[2]));

    // A second right-click on the same line removes it.
    screen.toggle_bookmark(&buffer, ScreenPosition { x: 0, y: 3 });
    assert!(!screen.bookmarks.contains(&ids[2]));
}

#[test]
fn clicks_off_the_content_are_ignored() {
    let buffer = full_buffer(5);
    let mut screen = sized_screen(&buffer, 10);

    // Top border row.
    screen.toggle_bookmark(&buffer, ScreenPosition { x: 0, y: 0 });
    // Well past the last line.
    screen.toggle_bookmark(&buffer, ScreenPosition { x: 0, y: 50 });

    assert!(screen.bookmarks.is_empty());
}

#[test]
fn removing_the_current_bookmark_forgets_it() {
    let buffer = full_buffer(5);
    let ids = ids(&buffer);
    let mut screen = sized_screen(&buffer, 10);

    screen.toggle_bookmark(&buffer, ScreenPosition { x: 0, y: 3 });
    screen.current_bookmark = Some(ids[2]);

    screen.toggle_bookmark(&buffer, ScreenPosition { x: 0, y: 3 });
    assert_eq!(screen.current_bookmark, None);
}

#[test]
fn clear_drops_all_bookmarks() {
    let buffer = full_buffer(5);
    let mut screen = sized_screen(&buffer, 10);
    screen.toggle_bookmark(&buffer, ScreenPosition { x: 0, y: 3 });
    screen.current_bookmark = Some(0);

    screen.clear();

    assert!(screen.bookmarks.is_empty());
    assert_eq!(screen.current_bookmark, None);
}

// A filter change re-derives the displayed buffer, which re-indexes every
// line. The viewport and the selection are positional and have to be
// re-anchored, but bookmarks pin to a line id and must survive — a bookmark
// hidden by a filter is supposed to come back with its line.
#[test]
fn a_rebuilt_buffer_keeps_bookmarks_but_re_anchors_the_viewport() {
    let buffer = full_buffer(5);
    let ids = ids(&buffer);
    let mut screen = sized_screen(&buffer, 10);
    screen.toggle_bookmark(&buffer, ScreenPosition { x: 0, y: 3 });
    screen.current_bookmark = Some(ids[2]);
    screen.disable_auto_scroll();
    screen.selection = Some(Selection::new(
        BufferPosition { line: 1, column: 0 },
        BufferPosition { line: 2, column: 1 },
    ));
    screen.evicted_seen = 7;

    screen.rebase_on_rebuilt_buffer();

    assert!(screen.bookmarks.contains(&ids[2]));
    assert_eq!(screen.current_bookmark, Some(ids[2]));
    // Positional state is reset: back to following the bottom, no
    // selection, and the eviction count restarts with the buffer's.
    assert!(screen.auto_scroll);
    assert!(screen.selection.is_none());
    assert_eq!(screen.evicted_seen, 0);
}

#[test]
fn next_index_without_current_anchors_to_the_viewport() {
    let positions = [(2, 100), (10, 101), (18, 102)];

    // Forward: first bookmark below the anchor line.
    assert_eq!(
        Screen::next_bookmark_index(&positions, None, 4, true),
        Some(1)
    );
    // Backward: last bookmark above the anchor line.
    assert_eq!(
        Screen::next_bookmark_index(&positions, None, 4, false),
        Some(0)
    );
    // Forward with nothing below wraps to the first.
    assert_eq!(
        Screen::next_bookmark_index(&positions, None, 100, true),
        Some(0)
    );
    // Backward with nothing above wraps to the last.
    assert_eq!(
        Screen::next_bookmark_index(&positions, None, 0, false),
        Some(2)
    );
}

#[test]
fn next_index_steps_and_wraps_from_the_current_bookmark() {
    let positions = [(2, 100), (10, 101), (18, 102)];

    assert_eq!(
        Screen::next_bookmark_index(&positions, Some(101), 999, true),
        Some(2)
    );
    // Forward off the end wraps to the first.
    assert_eq!(
        Screen::next_bookmark_index(&positions, Some(102), 999, true),
        Some(0)
    );
    // Backward off the front wraps to the last.
    assert_eq!(
        Screen::next_bookmark_index(&positions, Some(100), 999, false),
        Some(2)
    );
}

#[test]
fn next_index_falls_back_to_anchor_when_current_is_gone() {
    let positions = [(2, 100), (10, 101), (18, 102)];
    // Id 999 is not among the positions (its line was filtered out or
    // scrolled off), so navigation restarts from the viewport anchor.
    assert_eq!(
        Screen::next_bookmark_index(&positions, Some(999), 4, true),
        Some(1)
    );
}

#[test]
fn next_index_is_none_when_there_are_no_bookmarks() {
    assert_eq!(Screen::next_bookmark_index(&[], None, 4, true), None);
    assert_eq!(Screen::next_bookmark_index(&[], Some(1), 4, false), None);
}

#[test]
fn navigation_cycles_through_bookmarks_by_id() {
    let buffer = full_buffer(20);
    let ids = ids(&buffer);
    let mut screen = sized_screen(&buffer, 10);
    for line in [2usize, 10, 18] {
        screen.bookmarks.insert(ids[line]);
    }
    let max_main_axis = buffer.len().saturating_sub(8);

    // Fresh (no current): anchor is the screen centre (line 4), so the
    // first Tab lands on the first bookmark below it.
    screen.jump_to_next_bookmark(&buffer, max_main_axis);
    assert_eq!(screen.current_bookmark, Some(ids[10]));

    screen.jump_to_next_bookmark(&buffer, max_main_axis);
    assert_eq!(screen.current_bookmark, Some(ids[18]));

    // Wrap around to the top.
    screen.jump_to_next_bookmark(&buffer, max_main_axis);
    assert_eq!(screen.current_bookmark, Some(ids[2]));

    // Shift+Tab steps back, wrapping to the bottom.
    screen.jump_to_previous_bookmark(&buffer, max_main_axis);
    assert_eq!(screen.current_bookmark, Some(ids[18]));
}

#[test]
fn navigation_is_a_no_op_without_bookmarks() {
    let buffer = full_buffer(20);
    let mut screen = sized_screen(&buffer, 10);
    let max_main_axis = buffer.len().saturating_sub(8);

    screen.jump_to_next_bookmark(&buffer, max_main_axis);
    assert_eq!(screen.current_bookmark, None);
}

// The `Tab`-focused bookmark must stand out from the others: it keeps the full
// yellow-background highlight, while every other bookmark drops to a subtler
// yellow foreground on the normal background.
#[test]
fn current_bookmark_is_highlighted_apart_from_other_bookmarks() {
    let ts = Local::now();
    let style_of = |is_bookmarked, is_current| {
        ScreenMode::timestamp_line(ts, false, is_bookmarked, is_current)[0].style
    };

    // A plain (non-bookmarked) line: dim gray text, no background.
    let plain = style_of(false, false);
    assert_eq!(plain.bg, None);
    assert_eq!(plain.fg, Some(Color::DarkGray));

    // A bookmark that isn't the current one: yellow text, normal background.
    let other = style_of(true, false);
    assert_eq!(other.bg, None);
    assert_eq!(other.fg, Some(Color::Yellow));

    // The current bookmark: full yellow-background highlight.
    let current = style_of(true, true);
    assert_eq!(current.bg, Some(Color::Yellow));
    assert_eq!(current.fg, Some(Color::Black));
}
