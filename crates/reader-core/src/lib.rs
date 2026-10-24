//! The code reader's headless core: immutable versioned documents, the reloadable-content cache and the
//! session that owns every semantic state transition. Free of GPUI; adapters translate input into commands
//! and present the state snapshot.
#![deny(unsafe_code)]

pub mod actions;
mod analysis;
mod draft;
pub mod save;
pub mod close;
pub mod external;
pub mod blame;
pub mod branches;
pub mod cache;
pub mod chrome;
pub mod decor;
pub mod document;
pub mod expand;
pub mod find;
pub mod folds;
pub mod index;
mod index_work;
mod index_cache;
pub mod jobs;
pub mod keymap;
pub mod libraries;
pub mod markdown;
pub mod places;
pub mod preferences;
pub mod project;
pub mod postgres_assistance;
pub mod search;
pub mod semantics;
pub mod session;
pub mod shown;
pub mod breadcrumbs;
pub mod tabs;
pub mod watch;

pub mod workspace;

pub mod navigation;

pub mod text;

mod sql_assistance;
pub mod sql_dialect;
pub mod duckdb_assistance;
pub mod projection;
mod projection_jobs;
pub mod layout;
mod layout_jobs;

pub mod native;
