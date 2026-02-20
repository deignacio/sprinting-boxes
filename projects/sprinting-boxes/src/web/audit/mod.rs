// Audit system for sprinting-boxes
//
// This module provides the complete audit system including:
// - Data models (CliffData, AuditSettings, AuditState)
// - Utility functions (recalculate_audit, timestamp formatting)
// - HTTP handlers (API endpoints)
//
// The code has been split into sub-modules:
// - models.rs: Data structures
// - utils.rs: Business logic
// - handlers.rs: HTTP handlers

pub mod handlers;
pub mod models;
pub mod utils;
pub use handlers::{
    get_cliffs_handler, get_features_handler, get_studio_clips_handler, get_vlc_playlist_handler,
    get_youtube_chapters_handler, recalculate_audit_handler, save_audit_handler,
    save_vlc_playlist_handler, serve_run_crop_handler, update_audit_settings_handler,
    update_cliff_field_handler,
};
