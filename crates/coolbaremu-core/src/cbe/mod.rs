//! CBE Format Module
//!
//! This module provides parsing and handling for the Coolbar/CBE application format.
//! CBE files are container archives used by Coolbar mobile phones.

pub mod actor;
pub mod archive;
pub mod map;
pub mod resource;
pub mod sce;

// Re-export main types
pub use archive::CbeArchive;
pub use resource::{ResourceEntry, ResourceType};
