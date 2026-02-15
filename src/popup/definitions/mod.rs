//! Popup definitions
//!
//! Each popup has a factory function that creates a PopupDefinition
//! with all fields, buttons, and layout configured.

pub mod actions;
pub mod confirm;
pub mod connections;
pub mod filter;
pub mod help;
pub mod menu;
pub mod setup;
pub mod web;
pub mod notes_list;
pub mod world_editor;
pub mod world_selector;

pub use actions::*;
pub use confirm::*;
pub use connections::*;
pub use filter::*;
pub use help::*;
pub use menu::*;
pub use notes_list::*;
pub use setup::*;
pub use web::*;
pub use world_editor::*;
pub use world_selector::*;
