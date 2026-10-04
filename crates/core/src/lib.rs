//! Core logic for LYNO//HARDWIRED.
//!
//! The launcher never replaces MO2: it installs a portable MO2 instance
//! from packages published on GitHub Releases, keeps it updated, and
//! drives `ModOrganizer.exe` through its command line.

pub mod archive;
pub mod author;
pub mod download;
pub mod error;
pub mod files;
pub mod game;
pub mod github;
pub mod hash;
pub mod hash_cache;
pub mod install;
pub mod manifest;
pub mod meta;
pub mod mo2;
pub mod mod_install;
pub mod modlist;
pub mod nexus;
pub mod nexus_sso;
pub mod nxm;
pub mod package;
pub mod plan;
pub mod prefetch;
pub mod publish;
pub mod release;
pub mod report;
pub mod rules;
pub mod state;
pub mod tracking;
pub mod tree;
pub mod verify;

pub use error::{Error, Result};
