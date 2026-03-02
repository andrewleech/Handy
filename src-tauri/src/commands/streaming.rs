//! Streaming-specific Tauri commands.
//!
//! These live here rather than in `shortcut/mod.rs` because
//! `change_streaming_enabled_setting` has side effects on
//! `StreamingManager` (preload/unload engine).

use std::sync::Arc;
use tauri::{AppHandle, Manager};

use crate::managers::model::ModelManager;
use crate::managers::streaming::StreamingManager;
use crate::settings;

#[tauri::command]
#[specta::specta]
pub fn change_streaming_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    if enabled {
        let s = settings::get_settings(&app);
        if let Some(mm) = app.try_state::<Arc<ModelManager>>() {
            if mm.get_model_path(&s.streaming_model).is_err() {
                return Err("Streaming model is not downloaded".to_string());
            }
        }
    }

    let mut s = settings::get_settings(&app);
    s.streaming_enabled = enabled;
    let streaming_model = s.streaming_model.clone();
    settings::write_settings(&app, s);

    if let Some(sm) = app.try_state::<Arc<StreamingManager>>() {
        if enabled {
            sm.preload_model(&streaming_model);
        } else {
            sm.unload_engine();
        }
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_streaming_live_typing_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut s = settings::get_settings(&app);
    s.streaming_live_typing = enabled;
    settings::write_settings(&app, s);
    Ok(())
}
