//! Emoji picker popup definition (`Esc-e`)
//!
//! Tab strip (categories) + search field + grid of matching emoji + a footer
//! label showing the selected emoji and its search terms. Modelled on
//! `world_selector.rs` — search field plus a live-narrowed list — with the
//! list swapped for a `Grid` and a `Tabs` field added for category
//! narrowing. See `EMOJI-PICKER-ROADMAP.md` Step 3.

use crate::emoji::{Category, EMOJI};
use crate::popup::{
    Field, FieldId, FieldKind, ListItem, ListItemStyle, PopupDefinition, PopupId, PopupLayout,
    PopupState,
};

// Field IDs
pub const EMOJI_FIELD_TABS: FieldId = FieldId(1);
pub const EMOJI_FIELD_SEARCH: FieldId = FieldId(2);
pub const EMOJI_FIELD_GRID: FieldId = FieldId(3);
pub const EMOJI_FIELD_INFO: FieldId = FieldId(4); // footer Label

/// Map a tab index (as stored in the `Tabs` field's `selected_index`) to the
/// category it represents. Tab 0 is always "All" (`None`); tab `N + 1` is
/// `Category::all()[N]`. Step 4 needs this to turn a tab-strip change (or a
/// grid horizontal edge, via `tabs_step`) into a `filter_emoji` call.
pub fn tab_index_to_category(tab_index: usize) -> Option<Category> {
    if tab_index == 0 {
        None
    } else {
        Category::all().get(tab_index - 1).copied()
    }
}

/// Tab strip labels: "All" followed by every category's label, in
/// `Category::all()` order.
fn tab_labels() -> Vec<String> {
    let mut labels = vec!["All".to_string()];
    labels.extend(Category::all().iter().map(|c| c.label().to_string()));
    labels
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
/// of Clay's search uses, never whole-word. `category == None` means the
/// `All` tab. An empty query matches everything in the category.
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
        .with_field(Field::new(
            EMOJI_FIELD_TABS,
            "",
            FieldKind::tabs(tab_labels(), 0),
        ))
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
            EMOJI_FIELD_GRID,
            "",
            FieldKind::grid(cells, columns, visible_rows),
        ))
        .with_field(Field::new(
            EMOJI_FIELD_INFO,
            "",
            FieldKind::label(emoji_info_text(None)),
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
        .with_help(emoji_help_text())
}

/// Help text for the Emoji popup
fn emoji_help_text() -> Vec<String> {
    vec![
        "Emoji Picker",
        "",
        "Browse or search a curated set of emoji and insert",
        "one into the command input.",
        "",
        "Navigation:",
        "  Tab / Shift-Tab   Next / previous category",
        "  Left/Right/Up/Down  Move the grid cursor",
        "  Left on first cell / Right on last  Change category",
        "  PageUp / PageDown Scroll the grid a page",
        "  Home / End        First / last cell",
        "  Enter             Insert the selected emoji, close",
        "  Esc               Close without inserting",
        "",
        "Type to search within the active category. Search",
        "matches the emoji's name and its keywords.",
    ]
    .into_iter()
    .map(|s| s.to_string())
    .collect()
}

/// Format the footer info line for a selected emoji, or a "no match" hint
/// when nothing is selected.
fn emoji_info_text(item: Option<&ListItem>) -> String {
    match item {
        Some(item) => {
            let ch = item.columns.first().map(String::as_str).unwrap_or(&item.id);
            let name = item.columns.get(1).map(String::as_str).unwrap_or("");
            let keywords = item.columns.get(2).map(String::as_str).unwrap_or("");
            // No `:shortcode:` form here - the console footer shows the glyph
            // and its search terms, with the name simply leading the list.
            let terms: Vec<&str> = std::iter::once(name)
                .chain(keywords.split_whitespace())
                .collect();
            format!("{ch}   {}", terms.join(", "))
        }
        None => "No matching emoji".to_string(),
    }
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

/// Rewrite the footer Label field from the currently selected grid cell.
pub fn update_emoji_info(state: &mut PopupState) {
    let text = emoji_info_text(state.get_selected_grid_item());
    if let Some(field) = state.field_mut(EMOJI_FIELD_INFO) {
        if let FieldKind::Label { text: label_text } = &mut field.kind {
            *label_text = text;
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

    /// The footer shows the glyph and its search terms - never a `:shortcode:`.
    #[test]
    fn test_emoji_info_text_has_no_shortcode_form() {
        let cells = filter_emoji(None, "grin");
        let text = emoji_info_text(cells.first());
        assert!(!text.contains(':'), "footer still shows a shortcode: {text:?}");
        assert!(text.contains("grin"), "footer lost the name: {text:?}");
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

        let info = def.get_field(EMOJI_FIELD_INFO).expect("info field");
        assert!(matches!(info.kind, FieldKind::Label { .. }));

        if let FieldKind::Tabs { labels, .. } = &tabs.kind {
            assert_eq!(labels[0], "All");
            assert_eq!(labels.len(), Category::all().len() + 1);
        }
    }

    #[test]
    fn test_tab_index_to_category() {
        assert_eq!(tab_index_to_category(0), None);
        assert_eq!(tab_index_to_category(1), Some(Category::all()[0]));
        assert_eq!(
            tab_index_to_category(Category::all().len()),
            Some(*Category::all().last().unwrap())
        );
        assert_eq!(tab_index_to_category(Category::all().len() + 1), None);
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
}
