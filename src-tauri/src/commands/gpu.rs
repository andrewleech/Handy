//! GPU execution provider Tauri commands.

use std::sync::Arc;

use log::info;
use tauri::{AppHandle, Manager};

use crate::managers::model::{EngineType, ModelManager};
use crate::managers::streaming::StreamingManager;
use crate::managers::transcription::TranscriptionManager;
use crate::settings;

/// Map a settings string to a transcribe_rs::GpuProvider variant.
pub fn parse_gpu_provider(s: &str) -> Result<transcribe_rs::GpuProvider, String> {
    match s {
        "auto" => Ok(transcribe_rs::GpuProvider::Auto),
        "cpu" => Ok(transcribe_rs::GpuProvider::CpuOnly),
        "directml" => Ok(transcribe_rs::GpuProvider::DirectMl),
        "cuda" => Ok(transcribe_rs::GpuProvider::Cuda),
        "coreml" => Ok(transcribe_rs::GpuProvider::CoreMl),
        "webgpu" => Ok(transcribe_rs::GpuProvider::WebGpu),
        _ => Err(format!("Unknown GPU provider: {}", s)),
    }
}

/// Map a transcribe_rs::GpuProvider variant to a settings string.
fn gpu_provider_to_string(p: transcribe_rs::GpuProvider) -> &'static str {
    match p {
        transcribe_rs::GpuProvider::Auto => "auto",
        transcribe_rs::GpuProvider::CpuOnly => "cpu",
        transcribe_rs::GpuProvider::DirectMl => "directml",
        transcribe_rs::GpuProvider::Cuda => "cuda",
        transcribe_rs::GpuProvider::CoreMl => "coreml",
        transcribe_rs::GpuProvider::WebGpu => "webgpu",
    }
}

/// Return which GPU providers are available in this build.
#[tauri::command]
#[specta::specta]
pub fn get_available_gpu_providers() -> Vec<String> {
    transcribe_rs::available_providers()
        .into_iter()
        .map(|p| gpu_provider_to_string(p).to_string())
        .collect()
}

/// Returns true for engine types that use ORT (and thus respect the
/// GpuProvider setting).  Whisper uses whisper.cpp — reloading it on
/// provider change is a no-op waste of time.
fn is_ort_engine(engine_type: &EngineType) -> bool {
    matches!(
        engine_type,
        EngineType::Parakeet
            | EngineType::Moonshine
            | EngineType::SenseVoice
            | EngineType::Qwen3
    )
}

/// Change the GPU provider setting, update the global, and reload models.
#[tauri::command]
#[specta::specta]
pub async fn change_gpu_provider_setting(
    app: AppHandle,
    provider: String,
) -> Result<(), String> {
    let gpu_provider = parse_gpu_provider(&provider)?;

    // Check if the value actually changed — skip write + reload if not
    let mut s = settings::get_settings(&app);
    if s.gpu_provider == provider {
        return Ok(());
    }

    // Update the global atomic in transcribe-rs
    transcribe_rs::set_gpu_provider(gpu_provider);
    info!("GPU provider changed to: {:?}", gpu_provider);

    // Persist to settings
    let previous = s.gpu_provider.clone();
    s.gpu_provider = provider.clone();
    settings::write_settings(&app, s.clone());

    // Reload loaded models so they pick up the new execution providers
    if previous != provider {
        // Reload batch transcription model (if it's ORT-based)
        if let Some(tm) = app.try_state::<Arc<TranscriptionManager>>() {
            if let Some(current_model_id) = tm.get_current_model() {
                // Reject if a transcription is currently in flight.
                // During transcription the engine is taken out of the mutex,
                // so is_model_loaded() returns false even though current_model_id is set.
                if !tm.is_model_loaded() {
                    // Revert settings — we can't reload right now
                    let mut reverted = settings::get_settings(&app);
                    reverted.gpu_provider = previous;
                    settings::write_settings(&app, reverted);
                    transcribe_rs::set_gpu_provider(
                        parse_gpu_provider(&s.gpu_provider)
                            .unwrap_or(transcribe_rs::GpuProvider::Auto),
                    );
                    return Err(
                        "Cannot change GPU provider while a model is loading or transcription is in progress. Try again when idle."
                            .to_string(),
                    );
                }

                // Skip reload for non-ORT engines (Whisper uses whisper.cpp)
                let should_reload = app
                    .try_state::<Arc<ModelManager>>()
                    .and_then(|mm| mm.get_model_info(&current_model_id))
                    .map(|info| is_ort_engine(&info.engine_type))
                    .unwrap_or(false);

                if should_reload {
                    info!(
                        "Reloading batch model '{}' for new GPU provider",
                        current_model_id
                    );
                    if let Err(e) = tm.unload_model() {
                        log::warn!("Failed to unload batch model: {}", e);
                    }
                    if let Err(e) = tm.load_model(&current_model_id) {
                        return Err(format!("Failed to reload batch model: {}", e));
                    }
                } else {
                    info!(
                        "Skipping batch model reload: '{}' is not ORT-based",
                        current_model_id
                    );
                }
            }
        }

        // Reload streaming model (if streaming is enabled)
        if s.streaming_enabled {
            if let Some(sm) = app.try_state::<Arc<StreamingManager>>() {
                if let Some(mm) = app.try_state::<Arc<ModelManager>>() {
                    if mm.get_model_path(&s.streaming_model).is_ok() {
                        info!(
                            "Reloading streaming model '{}' for new GPU provider",
                            s.streaming_model
                        );
                        sm.unload_engine();
                        sm.preload_model(&s.streaming_model);
                    }
                }
            }
        }
    }

    Ok(())
}
