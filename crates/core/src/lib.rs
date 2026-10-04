//! Core logic for LYNO//HARDWIRED.
//!
//! The launcher never replaces MO2: it reads and writes a portable MO2
//! instance (mod folders, `meta.ini`, `modlist.txt`) and drives
//! `ModOrganizer.exe` through its command line.

pub mod error;
pub mod hash;
pub mod manifest;
pub mod meta;
pub mod mo2;
pub mod modlist;
pub mod plan;

pub use error::{Error, Result};
