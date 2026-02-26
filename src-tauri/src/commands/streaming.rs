//! Streaming-specific Tauri commands.
//!
//! These live here rather than in `shortcut/mod.rs` because
//! `change_streaming_enabled_setting` has side effects on
//! `StreamingManager` (preload/unload engine).

use std::sync::Arc;
use tauri::{AppHandle, Manager};

use crate::managers::model::{EngineType, ModelManager};
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
pub fn change_streaming_model_setting(app: AppHandle, model_id: String) -> Result<(), String> {
    // Validate that the model_id refers to a streaming engine
    if let Some(mm) = app.try_state::<Arc<ModelManager>>() {
        let info = mm
            .get_model_info(&model_id)
            .ok_or_else(|| format!("Unknown model: {}", model_id))?;
        match info.engine_type {
            EngineType::NemotronStreaming | EngineType::Qwen3Streaming => {}
            _ => return Err(format!("{} is not a streaming model", model_id)),
        }
    }

    let mut s = settings::get_settings(&app);
    let previous_model = s.streaming_model.clone();
    s.streaming_model = model_id.clone();
    let streaming_enabled = s.streaming_enabled;
    settings::write_settings(&app, s);

    // If streaming is active and the model changed, swap the engine
    if streaming_enabled && model_id != previous_model {
        if let Some(sm) = app.try_state::<Arc<StreamingManager>>() {
            sm.unload_engine();
            if let Some(mm) = app.try_state::<Arc<ModelManager>>() {
                if mm.get_model_path(&model_id).is_ok() {
                    sm.preload_model(&model_id);
                }
            }
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
