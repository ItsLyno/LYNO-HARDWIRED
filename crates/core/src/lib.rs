//! Core logic for LYNO//HARDWIRED.
//!
//! The launcher never replaces MO2: it installs a portable MO2 instance
//! from packages published on GitHub Releases, keeps it updated, and
//! drives `ModOrganizer.exe` through its command line.

pub mod download;
pub mod error;
pub mod game;
pub mod hash;
pub mod hash_cache;
pub mod install;
pub mod manifest;
pub mod meta;
pub mod mo2;
pub mod modlist;
pub mod package;
pub mod plan;
pub mod publish;
pub mod rules;
pub mod state;
pub mod tree;

pub use error::{Error, Result};
