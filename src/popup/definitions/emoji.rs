//! Emoji picker popup definition (`Esc-e`)
//!
//! Search field + tab strip (categories) + grid of matching emoji — no
//! footer, no help text (R8 dropped both; the grid highlight already marks
//! the selection and movement is arrows-only). Modelled on
//! `world_selector.rs` — search field plus a live-narrowed list — with the
//! list swapped for a `Grid` and a `Tabs` field added for category
//! narrowing. See `EMOJI-PICKER-ROADMAP.md`'s Navigation rework.

use crate::emoji::{Category, EMOJI};
use crate::popup::{
    Field, FieldId, FieldKind, ListItem, ListItemStyle, PopupDefinition, PopupId, PopupLayout,
    PopupState,
};

// Field IDs
pub const EMOJI_FIELD_TABS: FieldId = FieldId(1);
pub const EMOJI_FIELD_SEARCH: FieldId = FieldId(2);
pub const EMOJI_FIELD_GRID: FieldId = FieldId(3);

/// The three focus zones the arrows-only key map moves between (see
/// `EMOJI-PICKER-ROADMAP.md`'s Navigation rework, "Key map — arrows only").
/// Search is row 0, Tabs is rows 1-2, Grid is everything below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmojiZone {
    Search,
    Tabs,
    Grid,
}

/// Map a tab index (as stored in the `Tabs` field's `selected_index`) to the
/// category it represents. `Category::all()[i]` for `i` in range; `None`
/// means the index is out of range — there is no `All` tab any more, so
/// global search is expressed by passing `None` to `filter_emoji` directly,
/// not by selecting a tab.
pub fn tab_index_to_category(tab_index: usize) -> Option<Category> {
    Category::all().get(tab_index).copied()
}

/// Tab strip labels: one glyph per category (`Category::tab_glyph()`), in
/// `Category::all()` order. No `All` entry — global search (an empty active
/// tab, `filter_emoji(None, query)`) replaces it.
fn tab_labels() -> Vec<String> {
    Category::all().iter().map(|c| c.tab_glyph().to_string()).collect()
}

/// Build a `ListItem` for one emoji entry: `id` is the character to insert,
/// `columns` are `[glyph, name, keywords]`.
fn emoji_list_item(entry: &crate::emoji::EmojiEntry) -> ListItem {
    let keywords = entry.aliases.join(" ");
    ListItem {
        id: entry.ch.to_string(),
        columns: vec![entry.ch.to_string(), entry.name.to_string(), keywords],
        style: ListItemStyle::default(),
    }
}

/// Filter `EMOJI` by category and a case-insensitive substring query over
/// `name` + every alias — the same `RecallMatchStyle::Simple` rule the rest
/// of Clay's search uses, never whole-word. `category == None` means global
/// search (no active tab — a live query suppresses the tab strip). An empty
/// query matches everything in the category.
pub fn filter_emoji(category: Option<Category>, query: &str) -> Vec<ListItem> {
    let query_lower = query.to_lowercase();
    EMOJI
        .iter()
        .filter(|entry| category.is_none_or(|c| entry.category == c))
        .filter(|entry| {
            if query_lower.is_empty() {
                return true;
            }
            if entry.name.to_lowercase().contains(&query_lower) {
                return true;
            }
            entry
                .aliases
                .iter()
                .any(|alias| alias.to_lowercase().contains(&query_lower))
        })
        .map(emoji_list_item)
        .collect()
}

/// [`filter_emoji`] for the **console** grid: the same matching rule, minus the
/// glyphs a terminal and `unicode_width` disagree about (see
/// [`crate::emoji::console_safe`]).
///
/// Deliberately a separate function rather than a filter inside `filter_emoji`.
/// `filter_emoji` is the shared matching rule that `getFilteredEmoji` in app.js
/// mirrors one-for-one, and the two are cross-checked against each other; the
/// exclusion here is a console *rendering* limit, not a change to what "matches".
/// The web and GUI pickers, and `:shortcode:` lookup, still see the whole table.
pub fn filter_emoji_console(category: Option<Category>, query: &str) -> Vec<ListItem> {
    filter_emoji(category, query)
        .into_iter()
        .filter(|item| crate::emoji::console_safe(&item.id))
        .collect()
}

/// Create the emoji picker popup definition. `columns` and `visible_rows`
/// size the grid; they are derived from the terminal size by the caller
/// (Step 4's `App::open_emoji_popup`).
pub fn create_emoji_popup(visible_rows: usize, columns: usize) -> PopupDefinition {
    let cells = filter_emoji_console(None, "");

    PopupDefinition::new(PopupId("emoji"), "Emoji")
        .with_field(
            Field::new(
                EMOJI_FIELD_SEARCH,
                "Search",
                FieldKind::text_with_placeholder("", "Type to search..."),
            )
            .with_tab_index(0)
            .search(),
        )
        .with_field(Field::new(
            EMOJI_FIELD_TABS,
            "",
            FieldKind::tabs(tab_labels(), 0),
        ))
        .with_field(Field::new(
            EMOJI_FIELD_GRID,
            "",
            FieldKind::grid(cells, columns, visible_rows),
        ))
        .with_layout(PopupLayout {
            label_width: 8,
            min_width: 40,
            max_width_percent: 90,
            center_horizontal: true,
            center_vertical: true,
            modal: true,
            buttons_right_align: true,
            blank_line_before_list: false,
            tab_buttons_only: false,
            anchor_bottom_left: false,
            anchor_x: 0,
        })
}

/// Rewrite the Grid field's cells in place, clamping `selected_index` and
/// `scroll_offset` afterwards. Mirrors `update_world_list`
/// (`world_selector.rs`).
pub fn update_emoji_grid(state: &mut PopupState, cells: &[ListItem]) {
    if let Some(field) = state.field_mut(EMOJI_FIELD_GRID) {
        if let FieldKind::Grid {
            cells: field_cells,
            selected_index,
            scroll_offset,
            columns,
            ..
        } = &mut field.kind
        {
            *field_cells = cells.to_vec();
            if *selected_index >= field_cells.len() {
                *selected_index = field_cells.len().saturating_sub(1);
            }
            let columns = (*columns).max(1);
            let row = *selected_index / columns;
            if *scroll_offset > row {
                *scroll_offset = row;
            }
        }
    }
}

/// The search field's live text - character-for-character the expression
/// that used to be duplicated at `input_handler.rs:1032` and
/// `remote_client.rs:1984`. While the search row is focused and being
/// edited, `edit_buffer` is the source of truth (it hasn't been committed
/// back to the field yet); otherwise read the field's own `value`.
pub fn emoji_query(state: &PopupState) -> String {
    if state.editing && state.is_field_selected(EMOJI_FIELD_SEARCH) {
        state.edit_buffer.clone()
    } else {
        state.get_text(EMOJI_FIELD_SEARCH).unwrap_or("").to_string()
    }
}

/// The category a refilter should scope to, or `None` for global search.
/// Mode is derived, never stored: a live (non-empty) query means global
/// results and no active tab, full stop - regardless of what the Tabs
/// field's `selected_index` currently holds.
pub fn emoji_active_category(state: &PopupState) -> Option<Category> {
    if !emoji_query(state).is_empty() {
        return None;
    }
    let tab_index = state.field(EMOJI_FIELD_TABS).and_then(|f| {
        if let FieldKind::Tabs { selected_index, .. } = &f.kind {
            Some(*selected_index)
        } else {
            None
        }
    }).unwrap_or(0);
    tab_index_to_category(tab_index)
}

/// Rewrite the popup's title from the current mode: the active category's
/// name while browsing, or plain "Emoji" while a search query is live (the
/// search box already shows the query, and there is no current category to
/// name). No other popup mutates `definition.title` at runtime today -
/// `render_popup` (`console_renderer.rs:48`) re-reads it every frame, so
/// this works, but it's a first for this codebase.
pub fn update_emoji_title(state: &mut PopupState) {
    state.definition.title = match emoji_active_category(state) {
        Some(category) => format!("Emoji — {}", category.label()),
        None => "Emoji".to_string(),
    };
}

/// Do the whole refilter cycle in one pass: read the query, filter, and
/// rewrite the grid, title and `Tabs.active` from the result. The single
/// call used by every entry point (open, and the `EmojiFilter` action from
/// both the console and the SSH remote console) so the open path and the
/// keypress path cannot diverge.
pub fn refilter_emoji_console(state: &mut PopupState) {
    let query = emoji_query(state);
    let category = emoji_active_category(state);
    let cells = filter_emoji_console(category, &query);
    update_emoji_grid(state, &cells);
    update_emoji_title(state);
    if let Some(field) = state.field_mut(EMOJI_FIELD_TABS) {
        if let FieldKind::Tabs { active, .. } = &mut field.kind {
            *active = query.is_empty();
        }
    }
}

/// The only thing that moves focus between the three zones: commits any
/// in-progress edit (so leaving Search never strands text in `edit_buffer`
/// — the exact bug `handle_popup_mouse_click` already relies on being
/// fixed, see `EMOJI-PICKER-ROADMAP.md`'s "The search text" section),
/// selects the target field, and starts editing iff the target is Search.
pub fn emoji_focus(state: &mut PopupState, zone: EmojiZone) {
    state.commit_edit();
    let field_id = match zone {
        EmojiZone::Search => EMOJI_FIELD_SEARCH,
        EmojiZone::Tabs => EMOJI_FIELD_TABS,
        EmojiZone::Grid => EMOJI_FIELD_GRID,
    };
    state.select_field(field_id);
    if zone == EmojiZone::Search {
        state.start_edit();
    }
}

/// Which of the three zones currently has focus, derived from `state.selected`.
pub fn emoji_zone(state: &PopupState) -> EmojiZone {
    if state.is_field_selected(EMOJI_FIELD_TABS) {
        EmojiZone::Tabs
    } else if state.is_field_selected(EMOJI_FIELD_GRID) {
        EmojiZone::Grid
    } else {
        EmojiZone::Search
    }
}

/// Insert a character into the search query. While Search is focused and
/// being edited, this is an ordinary caret insert (`state.insert_char`,
/// which is a no-op unless `state.editing`); from Tabs or Grid there is no
/// visible caret, so it appends directly to the field's own value instead —
/// the only defensible position with no caret to show.
pub fn emoji_query_insert(state: &mut PopupState, c: char) {
    if state.editing && state.is_field_selected(EMOJI_FIELD_SEARCH) {
        state.insert_char(c);
    } else {
        let mut value = state.get_text(EMOJI_FIELD_SEARCH).unwrap_or("").to_string();
        value.push(c);
        state.set_text(EMOJI_FIELD_SEARCH, value);
    }
}

/// Backspace the search query. Mirrors `emoji_query_insert`: a caret-aware
/// delete while Search is focused-and-editing, otherwise pop the last
/// character off the field's value directly.
pub fn emoji_query_backspace(state: &mut PopupState) {
    if state.editing && state.is_field_selected(EMOJI_FIELD_SEARCH) {
        state.backspace();
    } else {
        let mut value = state.get_text(EMOJI_FIELD_SEARCH).unwrap_or("").to_string();
        value.pop();
        state.set_text(EMOJI_FIELD_SEARCH, value);
    }
}

/// Clear the search query outright — used when a deliberate `←`/`→`/`Home`/
/// `End` on the tab row changes category (see the key-map table). Clears
/// both the committed field value and, defensively, any live edit buffer
/// (Search shouldn't be mid-edit when this is called, since changing
/// category only happens from the Tabs zone, but leaving a stale buffer
/// around for a later `commit_edit()` to resurrect would be a trap).
pub fn emoji_query_clear(state: &mut PopupState) {
    state.set_text(EMOJI_FIELD_SEARCH, String::new());
    if state.is_field_selected(EMOJI_FIELD_SEARCH) {
        state.edit_buffer.clear();
        state.edit_cursor = 0;
    }
}

/// Jump the Tabs field's `selected_index` to the first or last category —
/// the Home/End "on the tab row" key-map rule. Caller is responsible for
/// clearing the query and re-homing the grid afterward, same as the
/// `←`/`→`-on-Tabs case (`tabs_step`).
pub fn emoji_tabs_select_edge(state: &mut PopupState, last: bool) {
    if let Some(field) = state.field_mut(EMOJI_FIELD_TABS) {
        if let FieldKind::Tabs { labels, selected_index, .. } = &mut field.kind {
            if !labels.is_empty() {
                *selected_index = if last { labels.len() - 1 } else { 0 };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every cell the console grid can show must be one ratatui measures as 2
    /// columns - that is the whole contract that keeps the popup's border and
    /// scrollbar from being overwritten.
    #[test]
    fn test_filter_emoji_console_is_all_width_two() {
        use unicode_width::UnicodeWidthStr;
        let cells = filter_emoji_console(None, "");
        assert!(!cells.is_empty());
        for cell in &cells {
            assert_eq!(
                UnicodeWidthStr::width(cell.id.as_str()),
                2,
                "console grid cell {:?} is not 2 columns wide",
                cell.id
            );
        }
    }

    /// The console view is a strict subset of the shared rule: it never adds a
    /// match, and it only ever drops glyphs the terminal can't place reliably.
    #[test]
    fn test_filter_emoji_console_is_a_subset_of_filter_emoji() {
        for query in ["", "heart", "flag", "a"] {
            let all = filter_emoji(None, query);
            let console = filter_emoji_console(None, query);
            assert!(console.len() <= all.len());
            let all_ids: Vec<&str> = all.iter().map(|i| i.id.as_str()).collect();
            for cell in &console {
                assert!(all_ids.contains(&cell.id.as_str()));
            }
        }
    }

    #[test]
    fn test_filter_emoji_finds_by_name() {
        let heart = EMOJI.iter().find(|e| e.name == "heart").expect("fixture needs a 'heart' entry");
        let results = filter_emoji(None, "heart");
        assert!(results.iter().any(|item| item.id == heart.ch));
    }

    #[test]
    fn test_filter_emoji_finds_by_alias() {
        let entry = EMOJI
            .iter()
            .find(|e| !e.aliases.is_empty())
            .expect("fixture needs an entry with an alias");
        let alias = entry.aliases[0];
        let results = filter_emoji(None, alias);
        assert!(results.iter().any(|item| item.id == entry.ch));
    }

    #[test]
    fn test_filter_emoji_case_insensitive() {
        let upper = filter_emoji(None, "HEART");
        let lower = filter_emoji(None, "heart");
        assert_eq!(upper.len(), lower.len());
        assert!(!upper.is_empty());
        let upper_ids: Vec<&str> = upper.iter().map(|i| i.id.as_str()).collect();
        let lower_ids: Vec<&str> = lower.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(upper_ids, lower_ids);
    }

    #[test]
    fn test_filter_emoji_substring_not_whole_word() {
        // "grin" (entry name "grin", alias "grinning") is a mid-string
        // substring of "grinning", not a whole word on its own within that
        // alias — a whole-word matcher would reject it there.
        let entry = EMOJI
            .iter()
            .find(|e| e.name == "grin")
            .expect("fixture needs a 'grin' entry with a 'grinning' alias");
        assert!(entry.aliases.contains(&"grinning"));

        // "rinn" is a pure mid-word substring of "grinning" that is not a
        // whole word and does not equal the entry's own name ("grin").
        let results = filter_emoji(None, "rinn");
        assert!(
            results.iter().any(|item| item.id == entry.ch),
            "expected a substring match against the 'grinning' alias"
        );
    }

    #[test]
    fn test_filter_emoji_respects_category() {
        // Find a query that hits more than one category, then confirm
        // narrowing to a single category returns fewer results.
        let mut query = None;
        for entry in EMOJI {
            let count = EMOJI
                .iter()
                .filter(|e| {
                    e.name.to_lowercase().contains(&entry.name.to_lowercase())
                        || e.aliases.iter().any(|a| a.to_lowercase().contains(&entry.name.to_lowercase()))
                })
                .map(|e| e.category)
                .collect::<std::collections::HashSet<_>>()
                .len();
            if count > 1 {
                query = Some(entry.name.to_string());
                break;
            }
        }

        // Fall back: use a very common short substring likely shared across
        // categories (e.g. a single letter) if no exact-name overlap found.
        let query = query.unwrap_or_else(|| "a".to_string());

        let all_results = filter_emoji(None, &query);
        let categorized_count: usize = Category::all()
            .iter()
            .map(|&c| filter_emoji(Some(c), &query).len())
            .sum();

        assert!(!all_results.is_empty());
        // Every category-scoped result set summed together must equal the
        // All-tab result (partition), and any single category must not
        // exceed the All-tab total.
        assert_eq!(categorized_count, all_results.len());
        for &c in Category::all() {
            assert!(filter_emoji(Some(c), &query).len() <= all_results.len());
        }
    }

    #[test]
    fn test_filter_emoji_all_empty_query_returns_everything() {
        let results = filter_emoji(None, "");
        assert_eq!(results.len(), EMOJI.len());
    }

    #[test]
    fn test_filter_emoji_nonsense_query_returns_empty() {
        let results = filter_emoji(None, "zzzznonexistentquery9999");
        assert!(results.is_empty());
    }

    #[test]
    fn test_create_emoji_popup_fields() {
        let def = create_emoji_popup(6, 10);

        let tabs = def.get_field(EMOJI_FIELD_TABS).expect("tabs field");
        assert!(matches!(tabs.kind, FieldKind::Tabs { .. }));

        let search = def.get_field(EMOJI_FIELD_SEARCH).expect("search field");
        assert!(matches!(search.kind, FieldKind::Text { .. }));
        assert!(search.search);

        let grid = def.get_field(EMOJI_FIELD_GRID).expect("grid field");
        assert!(matches!(grid.kind, FieldKind::Grid { .. }));

        if let FieldKind::Tabs { labels, .. } = &tabs.kind {
            assert_eq!(labels[0], "😀");
            assert_eq!(labels.len(), Category::all().len());
        }

        // Draw order follows definition order, and the search box must
        // render above the tab strip - see EMOJI-PICKER-ROADMAP.md's
        // Navigation rework field-order rule. No Info/footer field any
        // more (R8): the popup is exactly Search, Tabs, Grid.
        let ids: Vec<FieldId> = def.fields.iter().map(|f| f.id).collect();
        assert_eq!(
            ids,
            vec![EMOJI_FIELD_SEARCH, EMOJI_FIELD_TABS, EMOJI_FIELD_GRID],
            "field order must be Search, Tabs, Grid"
        );
    }

    #[test]
    fn test_tab_index_to_category() {
        assert_eq!(tab_index_to_category(0), Some(Category::all()[0]));
        assert_eq!(
            tab_index_to_category(Category::all().len() - 1),
            Some(*Category::all().last().unwrap())
        );
        assert_eq!(tab_index_to_category(Category::all().len()), None);
    }

    #[test]
    fn test_update_emoji_grid_clamps_selected_index() {
        let def = create_emoji_popup(6, 10);
        let mut state = PopupState::new(def);

        // Select an index near the end of the full (unfiltered) grid.
        state.grid_select(EMOJI.len() - 1);

        // Now refilter down to a tiny result set.
        let narrowed = filter_emoji(None, "zzzzznonexistent-should-not-match");
        // Force at least one item so we can check clamping to len-1, not 0.
        let narrowed = if narrowed.is_empty() {
            vec![emoji_list_item(&EMOJI[0])]
        } else {
            narrowed
        };
        update_emoji_grid(&mut state, &narrowed);

        if let Some(field) = state.field(EMOJI_FIELD_GRID) {
            if let FieldKind::Grid { selected_index, cells, .. } = &field.kind {
                assert!(*selected_index < cells.len());
                assert_eq!(*selected_index, cells.len() - 1);
            } else {
                panic!("expected Grid field");
            }
        } else {
            panic!("expected grid field to exist");
        }
    }

    /// Point the popup's Tabs field at a given category, as if the user had
    /// arrived there directly - a test helper only, `grid_select` etc. don't
    /// reach the Tabs field.
    fn select_tab(state: &mut PopupState, category: Category) {
        let index = Category::all().iter().position(|&c| c == category).unwrap();
        if let Some(field) = state.field_mut(EMOJI_FIELD_TABS) {
            if let FieldKind::Tabs { selected_index, .. } = &mut field.kind {
                *selected_index = index;
            }
        }
    }

    /// Browsing (empty query) on the Nature tab: results are Nature-only,
    /// the title names the category, and the tab strip reads as active.
    #[test]
    fn test_refilter_emoji_console_browsing_nature_tab() {
        let def = create_emoji_popup(6, 10);
        let mut state = PopupState::new(def);
        select_tab(&mut state, Category::Nature);
        state.set_text(EMOJI_FIELD_SEARCH, String::new());

        refilter_emoji_console(&mut state);

        let grid = state.field(EMOJI_FIELD_GRID).expect("grid field");
        if let FieldKind::Grid { cells, .. } = &grid.kind {
            assert!(!cells.is_empty());
            for cell in cells {
                let entry = EMOJI
                    .iter()
                    .find(|e| e.ch == cell.id)
                    .expect("grid cell must be a real EMOJI entry");
                assert_eq!(
                    entry.category,
                    Category::Nature,
                    "browsing the Nature tab must only show Nature entries, found {:?}",
                    entry.name
                );
            }
        } else {
            panic!("expected Grid field");
        }

        assert_eq!(state.definition.title, "Emoji — Nature");

        let tabs = state.field(EMOJI_FIELD_TABS).expect("tabs field");
        if let FieldKind::Tabs { active, .. } = &tabs.kind {
            assert!(*active, "tab strip must be active while browsing");
        } else {
            panic!("expected Tabs field");
        }
    }

    /// A live query on the Nature tab is global search: results span more
    /// than one category, the title has no category name, and the tab strip
    /// reads as inactive.
    #[test]
    fn test_refilter_emoji_console_searching_ignores_active_tab() {
        let def = create_emoji_popup(6, 10);
        let mut state = PopupState::new(def);
        select_tab(&mut state, Category::Nature);
        state.set_text(EMOJI_FIELD_SEARCH, "heart".to_string());

        refilter_emoji_console(&mut state);

        let grid = state.field(EMOJI_FIELD_GRID).expect("grid field");
        if let FieldKind::Grid { cells, .. } = &grid.kind {
            assert!(!cells.is_empty());
            let categories: std::collections::HashSet<Category> = cells
                .iter()
                .map(|cell| {
                    EMOJI
                        .iter()
                        .find(|e| e.ch == cell.id)
                        .expect("grid cell must be a real EMOJI entry")
                        .category
                })
                .collect();
            // The global-search rule: results are not confined to the
            // previously-active tab's category (every "heart" entry in the
            // table is actually Category::Smileys, none Nature - which is
            // exactly the point, a category-scoped search would have found
            // nothing at all here).
            assert!(
                categories.iter().any(|&c| c != Category::Nature),
                "expected at least one result outside the previously-active Nature tab, got {categories:?}"
            );
        } else {
            panic!("expected Grid field");
        }

        assert_eq!(state.definition.title, "Emoji");

        let tabs = state.field(EMOJI_FIELD_TABS).expect("tabs field");
        if let FieldKind::Tabs { active, .. } = &tabs.kind {
            assert!(!*active, "tab strip must go inactive while a query is live");
        } else {
            panic!("expected Tabs field");
        }
    }

    /// Guard against the `EmojiFilter` duplication this helper replaced
    /// coming back: both console front ends must route through
    /// `refilter_emoji_console` rather than re-inlining the refilter cycle.
    /// The repo already parses its own source in tests for exactly this
    /// reason (`keybindings.rs`'s `test_docs_key_table_matches_defaults`).
    #[test]
    fn test_refilter_emoji_console_used_by_both_front_ends() {
        let input_handler = include_str!("../../input_handler.rs");
        assert!(
            input_handler.contains("refilter_emoji_console"),
            "src/input_handler.rs must call refilter_emoji_console, not re-inline the refilter cycle"
        );

        let remote_client = include_str!("../../remote_client.rs");
        assert!(
            remote_client.contains("refilter_emoji_console"),
            "src/remote_client.rs must call refilter_emoji_console, not re-inline the refilter cycle"
        );
    }
}
