//! Curated emoji table for the emoji picker popup (`Esc-e`).
//!
//! This is a deliberately small, hand-curated set (~400 entries), not the full
//! Unicode emoji list. See `EMOJI-PICKER-ROADMAP.md` Step 1.
//!
//! Every entry is guaranteed (by the tests below) to render as exactly 2
//! terminal cells: narrow glyphs (bare arrows, `©`, `™`, `▪`, etc.) are
//! excluded, sequences that need the VS16 variation selector carry it, and
//! ZWJ sequences / regional-indicator flag pairs are used as-is.
//!
//! **`unicode_width` answering 2 does not mean every terminal draws 2.** The
//! crate applies emoji presentation and answers 2 for every entry here, and
//! ratatui lays its buffer out with those numbers — but a terminal that ignores
//! a VS16 selector draws `❤️` in ONE column, and one that cannot compose a ZWJ
//! sequence draws `❤️‍🔥` as two glyphs in FOUR. Either way the row slides
//! relative to the buffer and its last cell lands on the popup's own border and
//! scrollbar.
//!
//! The console grid used to hold those 38 sequences back from the picker
//! entirely — excluded from the grid AND unreachable by search, which is how
//! searching `heart` in the console came to return 17 results and not ❤️
//! itself (`EMOJI-PICKER-ROADMAP.md` R14). Instead, [`console_glyph`] derives
//! a *display* form whose rendered width `unicode_width` can state
//! truthfully — strip the VS16 selector, truncate at the first ZWJ — while
//! the value actually inserted on `Enter` stays the real, full glyph
//! (`ListItem::id` in `popup/definitions/emoji.rs` is never touched by this;
//! only `columns[0]`, the text painted into the grid, is). The web/GUI picker
//! (a browser does its own layout) and `:shortcode:` lookup were never
//! affected either way and always saw the whole table.

use std::collections::HashMap;
use std::sync::OnceLock;

/// The eight emoji-picker categories. `All` (the default tab) is not a
/// variant here — it is represented as `Option<Category>::None` by callers
/// (see `filter_emoji` in Step 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Smileys,
    People,
    Nature,
    Food,
    Activity,
    Objects,
    Symbols,
    Flags,
}

impl Category {
    /// All categories, in tab-strip order.
    pub fn all() -> &'static [Category] {
        &[
            Category::Smileys,
            Category::People,
            Category::Nature,
            Category::Food,
            Category::Activity,
            Category::Objects,
            Category::Symbols,
            Category::Flags,
        ]
    }

    /// The tab label shown in the picker's tab strip.
    pub fn label(&self) -> &'static str {
        match self {
            Category::Smileys => "Smileys",
            Category::People => "People",
            Category::Nature => "Nature",
            Category::Food => "Food",
            Category::Activity => "Activity",
            Category::Objects => "Objects",
            Category::Symbols => "Symbols",
            Category::Flags => "Flags",
        }
    }

    /// The glyph shown in the picker's tab strip (R5). Deliberately not
    /// required to be a member of this category's own `EMOJI` entries - e.g.
    /// Nature's tab glyph is 🐶 (U+1F436), which isn't in the table at all
    /// (the table has 🐕 for `dog`); the glyph is chosen for recognizability
    /// as a tab icon, not as a representative catalog entry.
    pub fn tab_glyph(&self) -> &'static str {
        match self {
            Category::Smileys => "😀",
            Category::People => "👋",
            Category::Nature => "🐶",
            Category::Food => "🍕",
            Category::Activity => "⚽",
            Category::Objects => "💡",
            Category::Symbols => "⭐",
            Category::Flags => "🏁",
        }
    }

    /// A stable numeric index, used on the wire (`emoji_json`) so JS doesn't
    /// need to know the Rust enum's variant names.
    pub fn index(&self) -> usize {
        match self {
            Category::Smileys => 0,
            Category::People => 1,
            Category::Nature => 2,
            Category::Food => 3,
            Category::Activity => 4,
            Category::Objects => 5,
            Category::Symbols => 6,
            Category::Flags => 7,
        }
    }
}

/// One entry in the curated emoji table.
pub struct EmojiEntry {
    /// The literal emoji character (may be a multi-codepoint sequence: VS16,
    /// ZWJ, or a regional-indicator flag pair).
    pub ch: &'static str,
    /// The `:shortcode:` body, unique across the whole table.
    pub name: &'static str,
    /// Extra search keywords. Also unique across the whole table (no alias
    /// collides with any other entry's name or aliases).
    pub aliases: &'static [&'static str],
    pub category: Category,
}

/// The curated emoji table (~400 entries), seeded from the former
/// `encoding::emoji_name_to_unicode` match arms and grown with more food,
/// fantasy/MUD-flavoured objects, flags, hand gestures, and nature.
pub const EMOJI: &[EmojiEntry] = &[
    // ---- Smileys ----
    EmojiEntry { ch: "😊", name: "smile", aliases: &["smiley", "happy", "blush"], category: Category::Smileys },
    EmojiEntry { ch: "😀", name: "grin", aliases: &["grinning"], category: Category::Smileys },
    EmojiEntry { ch: "😂", name: "joy", aliases: &["laughing", "lol"], category: Category::Smileys },
    EmojiEntry { ch: "🤣", name: "rofl", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "😉", name: "wink", aliases: &["winking"], category: Category::Smileys },
    EmojiEntry { ch: "😍", name: "heart_eyes", aliases: &["hearteyes"], category: Category::Smileys },
    EmojiEntry { ch: "😘", name: "kissing_heart", aliases: &["kissingheart"], category: Category::Smileys },
    EmojiEntry { ch: "💋", name: "kiss", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "😋", name: "yum", aliases: &["delicious"], category: Category::Smileys },
    EmojiEntry { ch: "😛", name: "stuck_out_tongue", aliases: &["tongue"], category: Category::Smileys },
    EmojiEntry { ch: "🤪", name: "crazy", aliases: &["zany"], category: Category::Smileys },
    EmojiEntry { ch: "🤔", name: "thinking", aliases: &["think", "hmm"], category: Category::Smileys },
    EmojiEntry { ch: "🤫", name: "shush", aliases: &["shushing"], category: Category::Smileys },
    EmojiEntry { ch: "😐", name: "neutral", aliases: &["meh"], category: Category::Smileys },
    EmojiEntry { ch: "😑", name: "expressionless", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "😒", name: "unamused", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "🙄", name: "roll_eyes", aliases: &["rolleyes", "eyeroll"], category: Category::Smileys },
    EmojiEntry { ch: "😬", name: "grimace", aliases: &["grimacing"], category: Category::Smileys },
    EmojiEntry { ch: "😌", name: "relieved", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "😔", name: "pensive", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "😪", name: "sleepy", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "😴", name: "sleeping", aliases: &["zzz"], category: Category::Smileys },
    EmojiEntry { ch: "🤢", name: "sick", aliases: &["ill"], category: Category::Smileys },
    EmojiEntry { ch: "🤮", name: "vomit", aliases: &["puke"], category: Category::Smileys },
    EmojiEntry { ch: "🤧", name: "sneeze", aliases: &["sneezing"], category: Category::Smileys },
    EmojiEntry { ch: "🥵", name: "hot", aliases: &["overheated"], category: Category::Smileys },
    EmojiEntry { ch: "🥶", name: "cold", aliases: &["freezing"], category: Category::Smileys },
    EmojiEntry { ch: "🥴", name: "woozy", aliases: &["dizzy"], category: Category::Smileys },
    EmojiEntry { ch: "🤯", name: "exploding_head", aliases: &["mindblown"], category: Category::Smileys },
    EmojiEntry { ch: "🤠", name: "cowboy", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "😎", name: "sunglasses", aliases: &["cool"], category: Category::Smileys },
    EmojiEntry { ch: "🤓", name: "nerd", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "🧐", name: "monocle", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "😕", name: "confused", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "😟", name: "worried", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "☹️", name: "frown", aliases: &["frowning", "sad"], category: Category::Smileys },
    EmojiEntry { ch: "😢", name: "cry", aliases: &["crying"], category: Category::Smileys },
    EmojiEntry { ch: "😭", name: "sob", aliases: &["sobbing"], category: Category::Smileys },
    EmojiEntry { ch: "😠", name: "angry", aliases: &["mad"], category: Category::Smileys },
    EmojiEntry { ch: "😡", name: "rage", aliases: &["furious"], category: Category::Smileys },
    EmojiEntry { ch: "💀", name: "skull", aliases: &["dead"], category: Category::Smileys },
    EmojiEntry { ch: "💩", name: "poop", aliases: &["poo", "shit"], category: Category::Smileys },
    EmojiEntry { ch: "🤡", name: "clown", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "👻", name: "ghost", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "👽", name: "alien", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "🤖", name: "robot", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "😺", name: "cat", aliases: &["smiley_cat"], category: Category::Smileys },
    EmojiEntry { ch: "😻", name: "heart_eyes_cat", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "🙀", name: "scream_cat", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "😿", name: "crying_cat", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "😾", name: "pouting_cat", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "😈", name: "devil", aliases: &["imp"], category: Category::Smileys },
    EmojiEntry { ch: "😇", name: "angel", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "❤️", name: "heart", aliases: &["love", "red_heart"], category: Category::Smileys },
    EmojiEntry { ch: "🧡", name: "orange_heart", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "💛", name: "yellow_heart", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "💚", name: "green_heart", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "💙", name: "blue_heart", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "💜", name: "purple_heart", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "🖤", name: "black_heart", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "🤍", name: "white_heart", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "💔", name: "broken_heart", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "💖", name: "sparkling_heart", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "💓", name: "heartbeat", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "💗", name: "heartpulse", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "💕", name: "two_hearts", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "💞", name: "revolving_hearts", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "💘", name: "cupid", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "💝", name: "gift_heart", aliases: &[], category: Category::Smileys },
    EmojiEntry { ch: "🥺", name: "pleading_face", aliases: &["begging", "puppy_eyes"], category: Category::Smileys },
    EmojiEntry { ch: "🤩", name: "star_struck", aliases: &["starstruck", "starry_eyed"], category: Category::Smileys },
    EmojiEntry { ch: "🥳", name: "partying_face", aliases: &["party_face", "celebrating"], category: Category::Smileys },
    EmojiEntry { ch: "🫠", name: "melting_face", aliases: &["melting"], category: Category::Smileys },
    EmojiEntry { ch: "🥹", name: "face_holding_back_tears", aliases: &["holding_back_tears"], category: Category::Smileys },
    EmojiEntry { ch: "😳", name: "flushed", aliases: &["embarrassed"], category: Category::Smileys },
    EmojiEntry { ch: "😱", name: "scream", aliases: &["terrified"], category: Category::Smileys },
    EmojiEntry { ch: "😨", name: "fearful", aliases: &["scared"], category: Category::Smileys },
    EmojiEntry { ch: "😞", name: "disappointed", aliases: &["letdown"], category: Category::Smileys },
    EmojiEntry { ch: "😤", name: "triumph", aliases: &["huff"], category: Category::Smileys },
    EmojiEntry { ch: "😏", name: "smirk", aliases: &["smug"], category: Category::Smileys },
    EmojiEntry { ch: "🥱", name: "yawn", aliases: &["yawning", "tired"], category: Category::Smileys },
    EmojiEntry { ch: "❤️‍🔥", name: "heart_on_fire", aliases: &["passion"], category: Category::Smileys },
    // ---- People ----
    EmojiEntry { ch: "👋", name: "wave", aliases: &["waving"], category: Category::People },
    EmojiEntry { ch: "✋", name: "raised_hand", aliases: &["hand"], category: Category::People },
    EmojiEntry { ch: "👌", name: "ok_hand", aliases: &["ok"], category: Category::People },
    EmojiEntry { ch: "👍", name: "thumbs_up", aliases: &["thumbsup", "+1", "like"], category: Category::People },
    EmojiEntry { ch: "👎", name: "thumbs_down", aliases: &["thumbsdown", "-1", "dislike"], category: Category::People },
    EmojiEntry { ch: "👏", name: "clap", aliases: &["clapping"], category: Category::People },
    EmojiEntry { ch: "🤝", name: "handshake", aliases: &[], category: Category::People },
    EmojiEntry { ch: "🙏", name: "pray", aliases: &["praying", "please", "thanks"], category: Category::People },
    EmojiEntry { ch: "💪", name: "muscle", aliases: &["flex", "strong"], category: Category::People },
    EmojiEntry { ch: "🖕", name: "middle_finger", aliases: &["fu"], category: Category::People },
    EmojiEntry { ch: "☝️", name: "point_up", aliases: &[], category: Category::People },
    EmojiEntry { ch: "👇", name: "point_down", aliases: &[], category: Category::People },
    EmojiEntry { ch: "👈", name: "point_left", aliases: &[], category: Category::People },
    EmojiEntry { ch: "👉", name: "point_right", aliases: &[], category: Category::People },
    EmojiEntry { ch: "👊", name: "fist", aliases: &["punch"], category: Category::People },
    EmojiEntry { ch: "✊", name: "raised_fist", aliases: &[], category: Category::People },
    EmojiEntry { ch: "✌️", name: "v", aliases: &["peace", "victory"], category: Category::People },
    EmojiEntry { ch: "🤞", name: "fingers_crossed", aliases: &["crossed_fingers"], category: Category::People },
    EmojiEntry { ch: "🤟", name: "love_you", aliases: &["ily"], category: Category::People },
    EmojiEntry { ch: "🤘", name: "metal", aliases: &["rock", "horns"], category: Category::People },
    EmojiEntry { ch: "🤙", name: "call_me", aliases: &["shaka"], category: Category::People },
    EmojiEntry { ch: "👀", name: "eyes", aliases: &[], category: Category::People },
    EmojiEntry { ch: "👁️", name: "eye", aliases: &[], category: Category::People },
    EmojiEntry { ch: "🧠", name: "brain", aliases: &[], category: Category::People },
    EmojiEntry { ch: "🙌", name: "raised_hands", aliases: &["praise", "hooray"], category: Category::People },
    EmojiEntry { ch: "👐", name: "open_hands", aliases: &["opening_hands"], category: Category::People },
    EmojiEntry { ch: "✍️", name: "writing_hand", aliases: &["write", "signing"], category: Category::People },
    EmojiEntry { ch: "🤳", name: "selfie", aliases: &["selfie_camera"], category: Category::People },
    EmojiEntry { ch: "💅", name: "nail_care", aliases: &["nails", "manicure"], category: Category::People },
    EmojiEntry { ch: "🤭", name: "hand_over_mouth", aliases: &["oops", "gasp"], category: Category::People },
    EmojiEntry { ch: "🤏", name: "pinching_hand", aliases: &["small_amount", "tiny"], category: Category::People },
    EmojiEntry { ch: "🖖", name: "vulcan", aliases: &["spock", "live_long"], category: Category::People },
    EmojiEntry { ch: "🤲", name: "palm_up", aliases: &["upside_down_palm"], category: Category::People },
    EmojiEntry { ch: "🤚", name: "raised_back_of_hand", aliases: &["backhand"], category: Category::People },
    EmojiEntry { ch: "🦶", name: "foot", aliases: &["kick"], category: Category::People },
    EmojiEntry { ch: "👂", name: "ear", aliases: &["listen"], category: Category::People },
    EmojiEntry { ch: "👃", name: "nose", aliases: &["sniff"], category: Category::People },
    EmojiEntry { ch: "👶", name: "baby", aliases: &["infant"], category: Category::People },
    EmojiEntry { ch: "👦", name: "boy", aliases: &["lad"], category: Category::People },
    EmojiEntry { ch: "👧", name: "girl", aliases: &["lass"], category: Category::People },
    EmojiEntry { ch: "👨", name: "man", aliases: &["gentleman"], category: Category::People },
    EmojiEntry { ch: "👩", name: "woman", aliases: &["lady"], category: Category::People },
    EmojiEntry { ch: "🥷", name: "ninja", aliases: &["stealth", "assassin"], category: Category::People },
    EmojiEntry { ch: "🧙", name: "mage", aliases: &["wizard", "witch", "sorcerer"], category: Category::People },
    EmojiEntry { ch: "🧛", name: "vampire", aliases: &["dracula"], category: Category::People },
    EmojiEntry { ch: "🧟", name: "zombie", aliases: &["undead"], category: Category::People },
    EmojiEntry { ch: "🧜", name: "mermaid", aliases: &["merman", "siren"], category: Category::People },
    EmojiEntry { ch: "🧞", name: "genie", aliases: &["wish", "lamp_genie"], category: Category::People },
    EmojiEntry { ch: "🫂", name: "people_hugging", aliases: &["hug"], category: Category::People },
    EmojiEntry { ch: "💃", name: "dancer", aliases: &["dancing"], category: Category::People },
    // ---- Nature ----
    EmojiEntry { ch: "🐕", name: "dog", aliases: &["puppy"], category: Category::Nature },
    EmojiEntry { ch: "🐈", name: "cat2", aliases: &["kitty"], category: Category::Nature },
    EmojiEntry { ch: "🐁", name: "mouse", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐹", name: "hamster", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐰", name: "rabbit", aliases: &["bunny"], category: Category::Nature },
    EmojiEntry { ch: "🦊", name: "fox", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐻", name: "bear", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐼", name: "panda", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐨", name: "koala", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐯", name: "tiger", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🦁", name: "lion", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐄", name: "cow", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐷", name: "pig", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐸", name: "frog", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐒", name: "monkey", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐔", name: "chicken", aliases: &["hen"], category: Category::Nature },
    EmojiEntry { ch: "🐧", name: "penguin", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐦", name: "bird", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🦅", name: "eagle", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🦆", name: "duck", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🦉", name: "owl", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🦇", name: "bat", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐺", name: "wolf", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐴", name: "horse", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🦄", name: "unicorn", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐝", name: "bee", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐛", name: "bug", aliases: &["beetle"], category: Category::Nature },
    EmojiEntry { ch: "🦋", name: "butterfly", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐌", name: "snail", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐚", name: "shell", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🦀", name: "crab", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🦐", name: "shrimp", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🦑", name: "squid", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐙", name: "octopus", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐟", name: "fish", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐬", name: "dolphin", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐳", name: "whale", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🦈", name: "shark", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐊", name: "crocodile", aliases: &["alligator"], category: Category::Nature },
    EmojiEntry { ch: "🐍", name: "snake", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐢", name: "turtle", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐉", name: "dragon", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🐲", name: "dragon_face", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🦖", name: "t_rex", aliases: &["trex", "dinosaur"], category: Category::Nature },
    EmojiEntry { ch: "🌹", name: "rose", aliases: &["flower"], category: Category::Nature },
    EmojiEntry { ch: "🌷", name: "tulip", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🌻", name: "sunflower", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🌺", name: "hibiscus", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🌸", name: "cherry_blossom", aliases: &["sakura"], category: Category::Nature },
    EmojiEntry { ch: "🍀", name: "four_leaf_clover", aliases: &["lucky", "clover"], category: Category::Nature },
    EmojiEntry { ch: "🌵", name: "cactus", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🌴", name: "palm_tree", aliases: &["palm"], category: Category::Nature },
    EmojiEntry { ch: "🍁", name: "maple_leaf", aliases: &["canada_leaf"], category: Category::Nature },
    EmojiEntry { ch: "🍄", name: "mushroom", aliases: &["fungus", "toadstool"], category: Category::Nature },
    EmojiEntry { ch: "🌱", name: "seedling", aliases: &["sprout"], category: Category::Nature },
    EmojiEntry { ch: "🕷️", name: "spider", aliases: &["arachnid"], category: Category::Nature },
    EmojiEntry { ch: "🦎", name: "lizard", aliases: &["gecko"], category: Category::Nature },
    EmojiEntry { ch: "🦚", name: "peacock", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🦩", name: "flamingo", aliases: &[], category: Category::Nature },
    EmojiEntry { ch: "🦙", name: "llama", aliases: &["alpaca"], category: Category::Nature },
    // ---- Food ----
    EmojiEntry { ch: "🍎", name: "apple", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍌", name: "banana", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍊", name: "orange", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍋", name: "lemon", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍉", name: "watermelon", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍇", name: "grapes", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍓", name: "strawberry", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍑", name: "peach", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍒", name: "cherry", aliases: &["cherries"], category: Category::Food },
    EmojiEntry { ch: "🍕", name: "pizza", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍔", name: "hamburger", aliases: &["burger"], category: Category::Food },
    EmojiEntry { ch: "🍟", name: "fries", aliases: &["french_fries"], category: Category::Food },
    EmojiEntry { ch: "🌭", name: "hotdog", aliases: &["hot_dog"], category: Category::Food },
    EmojiEntry { ch: "🌮", name: "taco", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🌯", name: "burrito", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍿", name: "popcorn", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍦", name: "icecream", aliases: &["ice_cream"], category: Category::Food },
    EmojiEntry { ch: "🍩", name: "donut", aliases: &["doughnut"], category: Category::Food },
    EmojiEntry { ch: "🍪", name: "cookie", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🎂", name: "cake", aliases: &["birthday"], category: Category::Food },
    EmojiEntry { ch: "🥧", name: "pie", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍫", name: "chocolate", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍬", name: "candy", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "☕", name: "coffee", aliases: &["cafe"], category: Category::Food },
    EmojiEntry { ch: "🍵", name: "tea", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍺", name: "beer", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍻", name: "beers", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍷", name: "wine", aliases: &["wine_glass"], category: Category::Food },
    EmojiEntry { ch: "🍸", name: "cocktail", aliases: &["martini"], category: Category::Food },
    EmojiEntry { ch: "🍾", name: "champagne", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍍", name: "pineapple", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🥭", name: "mango", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🥑", name: "avocado", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🥕", name: "carrot", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🌽", name: "corn", aliases: &["maize"], category: Category::Food },
    EmojiEntry { ch: "🍞", name: "bread", aliases: &["loaf", "toast"], category: Category::Food },
    EmojiEntry { ch: "🧀", name: "cheese", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🥚", name: "egg", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🥓", name: "bacon", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍣", name: "sushi", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍜", name: "ramen", aliases: &["noodles"], category: Category::Food },
    EmojiEntry { ch: "🥟", name: "dumpling", aliases: &["gyoza", "potsticker"], category: Category::Food },
    EmojiEntry { ch: "🥨", name: "pretzel", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🥞", name: "pancakes", aliases: &[], category: Category::Food },
    EmojiEntry { ch: "🍯", name: "honey", aliases: &["honeypot"], category: Category::Food },
    EmojiEntry { ch: "🧁", name: "cupcake", aliases: &["muffin"], category: Category::Food },
    EmojiEntry { ch: "🍭", name: "lollipop", aliases: &["sucker"], category: Category::Food },
    EmojiEntry { ch: "🥛", name: "milk", aliases: &["milk_glass"], category: Category::Food },
    EmojiEntry { ch: "🍹", name: "tropical_drink", aliases: &["cocktail2", "piña_colada"], category: Category::Food },
    EmojiEntry { ch: "🧂", name: "salt", aliases: &["saltshaker"], category: Category::Food },
    // ---- Activity ----
    EmojiEntry { ch: "⚽", name: "soccer", aliases: &["football"], category: Category::Activity },
    EmojiEntry { ch: "🏀", name: "basketball", aliases: &[], category: Category::Activity },
    EmojiEntry { ch: "⚾", name: "baseball", aliases: &[], category: Category::Activity },
    EmojiEntry { ch: "🎾", name: "tennis", aliases: &[], category: Category::Activity },
    EmojiEntry { ch: "🏐", name: "volleyball", aliases: &[], category: Category::Activity },
    EmojiEntry { ch: "⛳", name: "golf", aliases: &[], category: Category::Activity },
    EmojiEntry { ch: "🎳", name: "bowling", aliases: &[], category: Category::Activity },
    EmojiEntry { ch: "🏆", name: "trophy", aliases: &["winner"], category: Category::Activity },
    EmojiEntry { ch: "🥇", name: "medal", aliases: &["gold_medal"], category: Category::Activity },
    EmojiEntry { ch: "🥈", name: "silver_medal", aliases: &[], category: Category::Activity },
    EmojiEntry { ch: "🥉", name: "bronze_medal", aliases: &[], category: Category::Activity },
    EmojiEntry { ch: "🎮", name: "video_game", aliases: &["gaming", "controller"], category: Category::Activity },
    EmojiEntry { ch: "🎲", name: "dice", aliases: &["game_die"], category: Category::Activity },
    EmojiEntry { ch: "🎯", name: "dart", aliases: &["bullseye"], category: Category::Activity },
    EmojiEntry { ch: "🏈", name: "football_american", aliases: &["american_football"], category: Category::Activity },
    EmojiEntry { ch: "🏉", name: "rugby", aliases: &[], category: Category::Activity },
    EmojiEntry { ch: "🏓", name: "ping_pong", aliases: &["table_tennis"], category: Category::Activity },
    EmojiEntry { ch: "🏸", name: "badminton", aliases: &[], category: Category::Activity },
    EmojiEntry { ch: "🏒", name: "hockey", aliases: &["ice_hockey"], category: Category::Activity },
    EmojiEntry { ch: "🎿", name: "ski", aliases: &["skiing"], category: Category::Activity },
    EmojiEntry { ch: "🏂", name: "snowboard", aliases: &["snowboarding"], category: Category::Activity },
    EmojiEntry { ch: "🏄", name: "surfing", aliases: &["surfer"], category: Category::Activity },
    EmojiEntry { ch: "🎣", name: "fishing", aliases: &["fishing_pole"], category: Category::Activity },
    EmojiEntry { ch: "🎸", name: "guitar", aliases: &["music_instrument"], category: Category::Activity },
    // ---- Objects ----
    EmojiEntry { ch: "📱", name: "phone", aliases: &["iphone", "mobile"], category: Category::Objects },
    EmojiEntry { ch: "💻", name: "computer", aliases: &["laptop", "pc"], category: Category::Objects },
    EmojiEntry { ch: "⌨️", name: "keyboard", aliases: &[], category: Category::Objects },
    EmojiEntry { ch: "🖱️", name: "mouse2", aliases: &["computer_mouse"], category: Category::Objects },
    EmojiEntry { ch: "🖨️", name: "printer", aliases: &[], category: Category::Objects },
    EmojiEntry { ch: "📷", name: "camera", aliases: &[], category: Category::Objects },
    EmojiEntry { ch: "📺", name: "tv", aliases: &["television"], category: Category::Objects },
    EmojiEntry { ch: "📻", name: "radio", aliases: &[], category: Category::Objects },
    EmojiEntry { ch: "💡", name: "bulb", aliases: &["lightbulb", "idea"], category: Category::Objects },
    EmojiEntry { ch: "🔦", name: "flashlight", aliases: &["torch"], category: Category::Objects },
    EmojiEntry { ch: "📖", name: "book", aliases: &[], category: Category::Objects },
    EmojiEntry { ch: "📚", name: "books", aliases: &[], category: Category::Objects },
    EmojiEntry { ch: "💵", name: "money", aliases: &["cash", "dollar"], category: Category::Objects },
    EmojiEntry { ch: "💳", name: "credit_card", aliases: &[], category: Category::Objects },
    EmojiEntry { ch: "💎", name: "gem", aliases: &["diamond"], category: Category::Objects },
    EmojiEntry { ch: "🔨", name: "hammer", aliases: &[], category: Category::Objects },
    EmojiEntry { ch: "🔧", name: "wrench", aliases: &[], category: Category::Objects },
    EmojiEntry { ch: "⚙️", name: "gear", aliases: &["cog", "settings"], category: Category::Objects },
    EmojiEntry { ch: "🔒", name: "lock", aliases: &["locked"], category: Category::Objects },
    EmojiEntry { ch: "🔓", name: "unlock", aliases: &["unlocked"], category: Category::Objects },
    EmojiEntry { ch: "🔑", name: "key", aliases: &[], category: Category::Objects },
    EmojiEntry { ch: "🔔", name: "bell", aliases: &[], category: Category::Objects },
    EmojiEntry { ch: "🎁", name: "gift", aliases: &["present"], category: Category::Objects },
    EmojiEntry { ch: "🎈", name: "balloon", aliases: &["balloons"], category: Category::Objects },
    EmojiEntry { ch: "🎉", name: "tada", aliases: &["party", "celebration"], category: Category::Objects },
    EmojiEntry { ch: "🎊", name: "confetti", aliases: &[], category: Category::Objects },
    EmojiEntry { ch: "🏰", name: "castle", aliases: &["fortress"], category: Category::Objects },
    EmojiEntry { ch: "🗡️", name: "dagger", aliases: &["sword", "blade"], category: Category::Objects },
    EmojiEntry { ch: "🛡️", name: "shield", aliases: &["armor", "defense"], category: Category::Objects },
    EmojiEntry { ch: "📜", name: "scroll", aliases: &["parchment"], category: Category::Objects },
    EmojiEntry { ch: "🔮", name: "crystal_ball", aliases: &["orb", "fortune"], category: Category::Objects },
    EmojiEntry { ch: "🧪", name: "potion", aliases: &["test_tube", "elixir"], category: Category::Objects },
    EmojiEntry { ch: "🪦", name: "tombstone", aliases: &["grave", "rip"], category: Category::Objects },
    EmojiEntry { ch: "🪙", name: "coin", aliases: &["gold", "money2"], category: Category::Objects },
    EmojiEntry { ch: "🗺️", name: "map", aliases: &["world_map", "treasure_map"], category: Category::Objects },
    EmojiEntry { ch: "🧭", name: "compass", aliases: &["navigate"], category: Category::Objects },
    EmojiEntry { ch: "🕯️", name: "candle", aliases: &["candlelight"], category: Category::Objects },
    EmojiEntry { ch: "🏮", name: "lantern", aliases: &["lamp"], category: Category::Objects },
    EmojiEntry { ch: "🪓", name: "axe", aliases: &["battleaxe"], category: Category::Objects },
    EmojiEntry { ch: "🏹", name: "bow_arrow", aliases: &["bow", "archery"], category: Category::Objects },
    EmojiEntry { ch: "⚓", name: "anchor", aliases: &["ship_anchor"], category: Category::Objects },
    EmojiEntry { ch: "🚢", name: "ship", aliases: &["boat"], category: Category::Objects },
    EmojiEntry { ch: "🪄", name: "magic_wand", aliases: &["wand", "spell"], category: Category::Objects },
    EmojiEntry { ch: "💊", name: "pill", aliases: &["medicine"], category: Category::Objects },
    EmojiEntry { ch: "💉", name: "syringe", aliases: &["injection"], category: Category::Objects },
    EmojiEntry { ch: "☂️", name: "umbrella", aliases: &["rain_umbrella"], category: Category::Objects },
    EmojiEntry { ch: "👓", name: "glasses", aliases: &["spectacles"], category: Category::Objects },
    EmojiEntry { ch: "✉️", name: "envelope", aliases: &["mail", "letter"], category: Category::Objects },
    EmojiEntry { ch: "📦", name: "package", aliases: &["parcel", "box"], category: Category::Objects },
    EmojiEntry { ch: "✂️", name: "scissors", aliases: &["cut"], category: Category::Objects },
    // ---- Symbols ----
    EmojiEntry { ch: "✅", name: "check", aliases: &["checkmark", "yes"], category: Category::Symbols },
    EmojiEntry { ch: "❌", name: "x", aliases: &["cross", "no"], category: Category::Symbols },
    EmojiEntry { ch: "⚠️", name: "warning", aliases: &["warn"], category: Category::Symbols },
    EmojiEntry { ch: "❓", name: "question", aliases: &["?"], category: Category::Symbols },
    EmojiEntry { ch: "❗", name: "exclamation", aliases: &["!"], category: Category::Symbols },
    EmojiEntry { ch: "💯", name: "100", aliases: &["hundred"], category: Category::Symbols },
    EmojiEntry { ch: "🔥", name: "fire", aliases: &["lit", "hot2"], category: Category::Symbols },
    EmojiEntry { ch: "⭐", name: "star", aliases: &["stars"], category: Category::Symbols },
    EmojiEntry { ch: "✨", name: "sparkles", aliases: &["sparkle"], category: Category::Symbols },
    EmojiEntry { ch: "💥", name: "boom", aliases: &["explosion"], category: Category::Symbols },
    EmojiEntry { ch: "⚡", name: "zap", aliases: &["lightning", "thunder"], category: Category::Symbols },
    EmojiEntry { ch: "🌈", name: "rainbow", aliases: &[], category: Category::Symbols },
    EmojiEntry { ch: "☀️", name: "sun", aliases: &["sunny"], category: Category::Symbols },
    EmojiEntry { ch: "🌙", name: "moon", aliases: &["crescent_moon"], category: Category::Symbols },
    EmojiEntry { ch: "☁️", name: "cloud", aliases: &["cloudy"], category: Category::Symbols },
    EmojiEntry { ch: "🌧️", name: "rain", aliases: &["rainy"], category: Category::Symbols },
    EmojiEntry { ch: "❄️", name: "snow", aliases: &["snowy", "snowflake"], category: Category::Symbols },
    EmojiEntry { ch: "🌍", name: "earth", aliases: &["globe", "world"], category: Category::Symbols },
    EmojiEntry { ch: "🚀", name: "rocket", aliases: &[], category: Category::Symbols },
    EmojiEntry { ch: "✈️", name: "airplane", aliases: &["plane"], category: Category::Symbols },
    EmojiEntry { ch: "🚗", name: "car", aliases: &["automobile"], category: Category::Symbols },
    EmojiEntry { ch: "🚌", name: "bus", aliases: &[], category: Category::Symbols },
    EmojiEntry { ch: "🚃", name: "train", aliases: &[], category: Category::Symbols },
    EmojiEntry { ch: "🚲", name: "bike", aliases: &["bicycle"], category: Category::Symbols },
    EmojiEntry { ch: "👑", name: "crown", aliases: &["king", "queen"], category: Category::Symbols },
    EmojiEntry { ch: "💍", name: "ring", aliases: &[], category: Category::Symbols },
    EmojiEntry { ch: "🕐", name: "clock", aliases: &["time"], category: Category::Symbols },
    EmojiEntry { ch: "⏳", name: "hourglass", aliases: &[], category: Category::Symbols },
    EmojiEntry { ch: "⏰", name: "alarm", aliases: &["alarm_clock"], category: Category::Symbols },
    EmojiEntry { ch: "🎵", name: "music", aliases: &["musical_note"], category: Category::Symbols },
    EmojiEntry { ch: "🎶", name: "notes", aliases: &["musical_notes"], category: Category::Symbols },
    EmojiEntry { ch: "🎤", name: "microphone", aliases: &["mic"], category: Category::Symbols },
    EmojiEntry { ch: "🎧", name: "headphones", aliases: &["headphone"], category: Category::Symbols },
    EmojiEntry { ch: "🎨", name: "art", aliases: &["palette"], category: Category::Symbols },
    EmojiEntry { ch: "🎬", name: "movie", aliases: &["film"], category: Category::Symbols },
    EmojiEntry { ch: "🎭", name: "mask", aliases: &["theater"], category: Category::Symbols },
    EmojiEntry { ch: "🚩", name: "flag", aliases: &[], category: Category::Symbols },
    EmojiEntry { ch: "🏳️", name: "white_flag", aliases: &[], category: Category::Symbols },
    EmojiEntry { ch: "☠️", name: "skull_crossbones", aliases: &["danger"], category: Category::Symbols },
    EmojiEntry { ch: "⬆️", name: "arrow_up", aliases: &["up"], category: Category::Symbols },
    EmojiEntry { ch: "⬇️", name: "arrow_down", aliases: &["down"], category: Category::Symbols },
    EmojiEntry { ch: "⬅️", name: "arrow_left", aliases: &["left"], category: Category::Symbols },
    EmojiEntry { ch: "➡️", name: "arrow_right", aliases: &["right"], category: Category::Symbols },
    EmojiEntry { ch: "🔄", name: "arrows_counterclockwise", aliases: &["refresh", "reload"], category: Category::Symbols },
    EmojiEntry { ch: "➕", name: "plus", aliases: &["add"], category: Category::Symbols },
    EmojiEntry { ch: "➖", name: "minus", aliases: &["subtract"], category: Category::Symbols },
    EmojiEntry { ch: "♾️", name: "infinity", aliases: &["forever"], category: Category::Symbols },
    EmojiEntry { ch: "♻️", name: "recycle", aliases: &["recycling"], category: Category::Symbols },
    EmojiEntry { ch: "☮️", name: "peace_symbol", aliases: &["peace_sign"], category: Category::Symbols },
    EmojiEntry { ch: "☯️", name: "yin_yang", aliases: &["balance"], category: Category::Symbols },
    EmojiEntry { ch: "⚛️", name: "atom", aliases: &["science"], category: Category::Symbols },
    EmojiEntry { ch: "🆘", name: "sos", aliases: &["help", "emergency"], category: Category::Symbols },
    EmojiEntry { ch: "🆕", name: "new", aliases: &["fresh"], category: Category::Symbols },
    // ---- Flags ----
    EmojiEntry { ch: "🇺🇸", name: "us", aliases: &["usa", "america"], category: Category::Flags },
    EmojiEntry { ch: "🇬🇧", name: "uk", aliases: &["gb", "britain", "england"], category: Category::Flags },
    EmojiEntry { ch: "🇨🇦", name: "canada", aliases: &["ca_flag"], category: Category::Flags },
    EmojiEntry { ch: "🇲🇽", name: "mexico", aliases: &["mx_flag"], category: Category::Flags },
    EmojiEntry { ch: "🇫🇷", name: "france", aliases: &["fr_flag"], category: Category::Flags },
    EmojiEntry { ch: "🇩🇪", name: "germany", aliases: &["de_flag"], category: Category::Flags },
    EmojiEntry { ch: "🇮🇹", name: "italy", aliases: &["it_flag"], category: Category::Flags },
    EmojiEntry { ch: "🇪🇸", name: "spain", aliases: &["es_flag"], category: Category::Flags },
    EmojiEntry { ch: "🇯🇵", name: "japan_flag", aliases: &["japan"], category: Category::Flags },
    EmojiEntry { ch: "🇨🇳", name: "china_flag", aliases: &["china"], category: Category::Flags },
    EmojiEntry { ch: "🇰🇷", name: "south_korea", aliases: &["korea"], category: Category::Flags },
    EmojiEntry { ch: "🇮🇳", name: "india", aliases: &["in_flag"], category: Category::Flags },
    EmojiEntry { ch: "🇧🇷", name: "brazil", aliases: &["br_flag"], category: Category::Flags },
    EmojiEntry { ch: "🇦🇺", name: "australia", aliases: &["au_flag"], category: Category::Flags },
    EmojiEntry { ch: "🇷🇺", name: "russia", aliases: &["ru_flag"], category: Category::Flags },
    EmojiEntry { ch: "🇳🇱", name: "netherlands", aliases: &["nl_flag"], category: Category::Flags },
    EmojiEntry { ch: "🇸🇪", name: "sweden", aliases: &["se_flag"], category: Category::Flags },
    EmojiEntry { ch: "🏴‍☠️", name: "pirate_flag", aliases: &["jolly_roger"], category: Category::Flags },
    EmojiEntry { ch: "🏳️‍🌈", name: "rainbow_flag", aliases: &["pride"], category: Category::Flags },
    EmojiEntry { ch: "🏁", name: "checkered_flag", aliases: &["racing", "finish"], category: Category::Flags },
];

/// Lookup table mapping every entry's `name` and every alias to its `ch`.
/// Built once from `EMOJI`.
fn lookup_table() -> &'static HashMap<&'static str, &'static str> {
    static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut map = HashMap::with_capacity(EMOJI.len() * 2);
        for entry in EMOJI {
            map.insert(entry.name, entry.ch);
            for alias in entry.aliases {
                map.insert(*alias, entry.ch);
            }
        }
        map
    })
}

/// Look up an emoji by name or alias, case-insensitively. Used by
/// `encoding::emoji_name_to_unicode`.
pub fn lookup(name: &str) -> Option<&'static str> {
    lookup_table().get(name).copied()
}

/// Derive the console-safe **display** form of a glyph: strip `U+FE0F` (VS16)
/// wherever it appears, and stop at the first `U+200D` (ZWJ), dropping
/// everything from there on. `❤️` (U+2764 U+FE0F) becomes `❤`; `❤️‍🔥`
/// (U+2764 U+FE0F U+200D U+1F525) becomes `❤`; `🏳️‍🌈` becomes `🏳`. A glyph
/// with neither marker — including a regional-indicator flag pair like
/// `🇺🇸`, which is two plain codepoints and contains neither selector — passes
/// through unchanged.
///
/// This exists because `unicode_width` disagrees with what a terminal
/// actually draws for these two sequence kinds, and ratatui lays its buffer
/// out with `unicode_width`'s numbers:
///
/// - **VS16** (`U+FE0F`). A terminal that honours the selector draws the
///   sequence in 2 columns; one that ignores it draws the bare BMP dingbat
///   in 1. `unicode_width` always answers 2.
/// - **ZWJ** (`U+200D`). A terminal that composes the sequence draws 2
///   columns; one that can't draws the parts side by side, so 4.
///   `unicode_width` always answers 2.
///
/// Either way the row painted from `unicode_width`'s numbers can slide
/// against a real terminal's, sending the row's last cell into the popup's
/// own border and scrollbar. The console grid used to hold these 38 entries
/// back from the picker entirely to avoid that (see git history), which also
/// made them unreachable by search. `console_glyph` instead gives the grid a
/// display string whose width it can state truthfully — every entry's result
/// measures 1 or 2 columns (pinned by this module's tests) — while the value
/// actually inserted on `Enter` stays the untouched original glyph.
/// Whether the console picker can show this emoji as its own distinguishable
/// cell.
///
/// [`console_glyph`] makes a VS16 sequence width-truthful by dropping the
/// selector, which is lossless enough to draw — `❤️` still reads as a heart.
/// A **ZWJ** sequence is different: truncating it at the joiner collapses it
/// onto its own base character, which is usually another entry in this table.
/// `heart_on_fire` becomes `❤`, indistinguishable from `heart`; `rainbow_flag`
/// becomes `🏳`, indistinguishable from `white_flag`. Two identical-looking
/// cells in a picker read as a rendering bug, and with no footer there is
/// nothing to tell them apart - so the three ZWJ entries stay out of the
/// console grid. They remain in the web/GUI picker, which composes them
/// properly, and in `:shortcode:` lookup.
pub fn console_renderable(ch: &str) -> bool {
    !ch.contains('\u{200D}')
}

pub fn console_glyph(ch: &str) -> String {
    let mut out = String::new();
    for c in ch.chars() {
        if c == '\u{200D}' {
            break;
        }
        if c == '\u{FE0F}' {
            continue;
        }
        out.push(c);
    }
    out
}

/// Compact wire form for `WsMessage::InitialState.emoji_json` (Step 6):
/// `[[ch, name, "kw1 kw2", category_index], ...]`. Built once into a
/// `OnceLock<String>`.
pub fn emoji_json() -> String {
    static JSON: OnceLock<String> = OnceLock::new();
    JSON.get_or_init(|| {
        let rows: Vec<serde_json::Value> = EMOJI
            .iter()
            .map(|e| {
                let keywords = e.aliases.join(" ");
                serde_json::json!([e.ch, e.name, keywords, e.category.index()])
            })
            .collect();
        serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string())
    })
    .clone()
}

/// Compact wire form for `WsMessage::InitialState.emoji_categories_json`
/// (Navigation rework R10): `[[index, name, glyph], ...]`, one row per
/// `Category::all()` entry in tab-strip order. Lets the web/GUI picker draw
/// the same eight tab glyphs as the console without hardcoding them in JS
/// and drifting from `Category`. Built once into a `OnceLock<String>`.
pub fn emoji_categories_json() -> String {
    static JSON: OnceLock<String> = OnceLock::new();
    JSON.get_or_init(|| {
        let rows: Vec<serde_json::Value> = Category::all()
            .iter()
            .map(|c| serde_json::json!([c.index(), c.label(), c.tab_glyph()]))
            .collect();
        serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string())
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthChar;

    /// Returns the terminal display width we expect for an emoji string:
    /// any multi-codepoint sequence (VS16, ZWJ, regional-indicator pair) is
    /// treated as 2 cells (that's the whole point of those sequences), a
    /// single scalar falls back to `UnicodeWidthChar::width`.
    fn expected_width(s: &str) -> usize {
        let count = s.chars().count();
        if count > 1 {
            return 2;
        }
        s.chars()
            .next()
            .and_then(UnicodeWidthChar::width)
            .unwrap_or(0)
    }

    #[test]
    fn every_entry_is_two_cells_wide() {
        let mut failures = Vec::new();
        for entry in EMOJI {
            let w = expected_width(entry.ch);
            if w != 2 {
                failures.push(format!(
                    "{} ({}) has width {} (codepoints: {:?})",
                    entry.name,
                    entry.ch,
                    w,
                    entry.ch.chars().map(|c| c as u32).collect::<Vec<_>>()
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "entries not exactly 2 cells wide:\n{}",
            failures.join("\n")
        );
    }

    /// `tab_glyph()` feeds directly into the console tab strip (R5), which
    /// pads on the fixed 2-column assumption unlike the grid (the tab strip
    /// is not routed through `console_glyph`), so each glyph must already be
    /// plain (no VS16/ZWJ to strip — `console_glyph` is a no-op on it) and
    /// exactly 2 columns wide, and the eight glyphs must be visually
    /// distinguishable from one another. Deliberately does NOT assert a
    /// glyph is present in `EMOJI`: Nature's tab glyph 🐶 (U+1F436) is not a
    /// table entry (the table has 🐕 for `dog`), which is intentional, not a
    /// bug.
    #[test]
    fn tab_glyph_is_console_safe_two_columns_and_distinct() {
        use unicode_width::UnicodeWidthStr;
        let mut seen = std::collections::HashSet::new();
        for category in Category::all() {
            let glyph = category.tab_glyph();
            assert_eq!(
                console_glyph(glyph),
                glyph,
                "{category:?} tab_glyph {glyph:?} must have no VS16/ZWJ to strip"
            );
            assert_eq!(
                UnicodeWidthStr::width(glyph),
                2,
                "{category:?} tab_glyph {glyph:?} must be exactly 2 columns wide"
            );
            assert!(
                seen.insert(glyph),
                "{category:?} tab_glyph {glyph:?} duplicates another category's glyph"
            );
        }
    }

    /// `console_glyph` on each of the four shapes the doc comment calls out:
    /// a plain emoji is untouched, a VS16 sequence loses the selector, a ZWJ
    /// sequence is truncated at the join, and a regional-indicator flag pair
    /// — which is easy to break with a naive `take_while` that stops at any
    /// non-ASCII lead byte — passes through whole because it contains neither
    /// marker.
    #[test]
    fn console_glyph_classifies_the_known_hard_cases() {
        assert_eq!(console_glyph("\u{1F600}"), "\u{1F600}", "plain emoji unchanged");
        assert_eq!(
            console_glyph("\u{1F1FA}\u{1F1F8}"),
            "\u{1F1FA}\u{1F1F8}",
            "regional-indicator flag pair passes through whole"
        );
        assert_eq!(console_glyph("\u{2764}\u{FE0F}"), "\u{2764}", "VS16 stripped");
        assert_eq!(console_glyph("\u{2639}\u{FE0F}"), "\u{2639}", "VS16 stripped");
        assert_eq!(
            console_glyph("\u{2764}\u{FE0F}\u{200D}\u{1F525}"),
            "\u{2764}",
            "ZWJ heart-on-fire truncated to the heart"
        );
        assert_eq!(
            console_glyph("\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}"),
            "\u{1F3F3}",
            "ZWJ rainbow flag truncated to the white flag"
        );
    }

    /// Every entry's `console_glyph` must measure 1 or 2 columns under
    /// `unicode_width` — the invariant `console_renderer`'s per-cell gutter
    /// padding (`CELL_STRIDE - width`, clamped to at least 1) depends on.
    /// Nothing wider than `CELL_STRIDE` and nothing zero-width may reach the
    /// grid.
    #[test]
    fn console_glyph_is_always_one_or_two_columns_wide() {
        use unicode_width::UnicodeWidthStr;
        let mut failures = Vec::new();
        for entry in EMOJI {
            let glyph = console_glyph(entry.ch);
            let w = UnicodeWidthStr::width(glyph.as_str());
            if w != 1 && w != 2 {
                failures.push(format!("{} ({:?} -> {:?}) has width {w}", entry.name, entry.ch, glyph));
            }
        }
        assert!(
            failures.is_empty(),
            "console_glyph produced an unexpected width:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn characters_are_unique() {
        let mut seen: HashMap<&str, &str> = HashMap::new();
        let mut dups = Vec::new();
        for entry in EMOJI {
            if let Some(owner) = seen.get(entry.ch) {
                dups.push(format!("{} used by both {owner:?} and {:?}", entry.ch, entry.name));
            } else {
                seen.insert(entry.ch, entry.name);
            }
        }
        assert!(dups.is_empty(), "duplicate emoji characters:\n{}", dups.join("\n"));
    }

    #[test]
    fn names_and_aliases_are_globally_unique() {
        let mut seen: HashMap<&str, &str> = HashMap::new();
        let mut collisions = Vec::new();
        for entry in EMOJI {
            let mut keys: Vec<&str> = vec![entry.name];
            keys.extend(entry.aliases.iter().copied());
            for key in keys {
                if let Some(owner) = seen.get(key) {
                    collisions.push(format!("{key:?} used by both {owner:?} and {:?}", entry.name));
                } else {
                    seen.insert(key, entry.name);
                }
            }
        }
        assert!(
            collisions.is_empty(),
            "duplicate name/alias keys:\n{}",
            collisions.join("\n")
        );
    }

    #[test]
    fn old_match_aliases_still_resolve_to_the_same_emoji() {
        // Sampled from `git show HEAD:src/encoding.rs`'s old `emoji_name_to_unicode`
        // match (before this file existed), spanning every former section so a
        // regression in any category shows up here.
        let samples: &[(&str, &str)] = &[
            ("smile", "😊"),
            ("grinning", "😀"),
            ("laughing", "😂"),
            ("thinking", "🤔"),
            ("roll_eyes", "🙄"),
            ("sleeping", "😴"),
            ("sunglasses", "😎"),
            ("crying", "😢"),
            ("skull", "💀"),
            ("thumbsup", "👍"),
            ("thumbsdown", "👎"),
            ("+1", "👍"),
            ("handshake", "🤝"),
            ("pray", "🙏"),
            ("heart", "❤️"),
            ("broken_heart", "💔"),
            ("dog", "🐕"),
            ("unicorn", "🦄"),
            ("dragon", "🐉"),
            ("pizza", "🍕"),
            ("coffee", "☕"),
            ("champagne", "🍾"),
            ("trophy", "🏆"),
            ("gem", "💎"),
            ("fire", "🔥"),
            ("rocket", "🚀"),
        ];
        for (alias, expected) in samples {
            assert_eq!(
                lookup(alias),
                Some(*expected),
                "alias {alias:?} should still resolve to {expected:?}"
            );
        }
    }

    #[test]
    fn emoji_json_parses_and_has_one_row_per_entry() {
        let json = emoji_json();
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("emoji_json() must produce valid JSON");
        let rows = parsed.as_array().expect("emoji_json() must be a JSON array");
        assert_eq!(rows.len(), EMOJI.len());
        for row in rows {
            let arr = row.as_array().expect("each row must be a JSON array");
            assert_eq!(arr.len(), 4, "row must be [ch, name, keywords, category_index]");
            assert!(arr[0].is_string());
            assert!(arr[1].is_string());
            assert!(arr[2].is_string());
            assert!(arr[3].is_u64());
        }
    }

    #[test]
    fn emoji_categories_json_matches_category_all() {
        let json = emoji_categories_json();
        let parsed: serde_json::Value = serde_json::from_str(&json)
            .expect("emoji_categories_json() must produce valid JSON");
        let rows = parsed
            .as_array()
            .expect("emoji_categories_json() must be a JSON array");
        assert_eq!(rows.len(), 8, "exactly 8 categories");
        for (row, category) in rows.iter().zip(Category::all().iter()) {
            let arr = row.as_array().expect("each row must be a JSON array");
            assert_eq!(arr.len(), 3, "row must be [index, name, glyph]");
            assert_eq!(arr[0].as_u64(), Some(category.index() as u64));
            assert_eq!(arr[1].as_str(), Some(category.label()));
            assert_eq!(arr[2].as_str(), Some(category.tab_glyph()));
        }
    }
}


