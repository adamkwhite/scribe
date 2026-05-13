use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;
use tray_item::TrayItem;

use scribe_core::{audio, config, notes, runtime};

enum TrayEvent {
    StartRecording,
    StopRecording,
    OpenNotes,
    OpenSettings,
    Quit,
}

/// Non-blocking send from a tray menu callback. The callback runs on the
/// Windows tray thread (not the tokio runtime), so we use `try_send` instead
/// of `block_on(send)` — a full channel drops the rapid click and traces it
/// rather than freezing the tray menu.
fn dispatch_tray_event(
    tx: &tokio::sync::mpsc::Sender<TrayEvent>,
    event: TrayEvent,
    label: &'static str,
) {
    if let Err(e) = tx.try_send(event) {
        tracing::warn!(error = ?e, "dropped tray event: {label}");
    }
}

/// Create a simple 16x16 icon programmatically (a green square with S).
fn create_default_icon() -> tray_item::IconSource {
    // Create a 16x16 icon using CreateIcon Windows API
    // AND mask: 0 = opaque, 1 = transparent (1 bit per pixel)
    // XOR mask: color data (32 bits per pixel: BGRA)
    unsafe {
        let width: i32 = 16;
        let height: i32 = 16;

        // AND mask: all opaque (all zeros), 1 bit per pixel, padded to WORD boundary
        let and_mask = vec![0u8; (width * height / 8) as usize];

        // XOR mask: 32-bit BGRA color data
        let mut xor_mask = vec![0u8; (width * height * 4) as usize];

        for y in 0..height {
            for x in 0..width {
                let idx = ((y * width + x) * 4) as usize;
                // Dark teal/green background
                xor_mask[idx] = 0x80; // B
                xor_mask[idx + 1] = 0xB0; // G
                xor_mask[idx + 2] = 0x40; // R
                xor_mask[idx + 3] = 0xFF; // A
            }
        }

        // Draw a simple "S" shape in white
        let s_pixels: &[(i32, i32)] = &[
            // Top bar
            (4, 2),
            (5, 2),
            (6, 2),
            (7, 2),
            (8, 2),
            (9, 2),
            (10, 2),
            (11, 2),
            // Left side upper
            (3, 3),
            (3, 4),
            (3, 5),
            (3, 6),
            // Middle bar
            (4, 7),
            (5, 7),
            (6, 7),
            (7, 7),
            (8, 7),
            (9, 7),
            (10, 7),
            (11, 7),
            // Right side lower
            (12, 8),
            (12, 9),
            (12, 10),
            (12, 11),
            // Bottom bar
            (4, 12),
            (5, 12),
            (6, 12),
            (7, 12),
            (8, 12),
            (9, 12),
            (10, 12),
            (11, 12),
        ];

        for &(x, y) in s_pixels {
            let idx = ((y * width + x) * 4) as usize;
            xor_mask[idx] = 0xFF; // B
            xor_mask[idx + 1] = 0xFF; // G
            xor_mask[idx + 2] = 0xFF; // R
            xor_mask[idx + 3] = 0xFF; // A
        }

        let hicon = windows_sys::Win32::UI::WindowsAndMessaging::CreateIcon(
            0, // hInstance
            width,
            height,
            1,  // planes
            32, // bits per pixel
            and_mask.as_ptr(),
            xor_mask.as_ptr(),
        );

        if hicon == 0 {
            // Fallback: try Resource anyway
            return tray_item::IconSource::Resource("default");
        }

        tray_item::IconSource::RawIcon(hicon)
    }
}

/// Prompt for a session name via Windows input dialog.
fn prompt_session_name_gui() -> Option<String> {
    let script = r#"
Add-Type -AssemblyName Microsoft.VisualBasic
$name = [Microsoft.VisualBasic.Interaction]::InputBox("Enter a name for this recording (or leave blank):", "Scribe — New Recording", "")
Write-Output $name
"#;
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", script])
        .output()
        .ok()?;

    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

fn open_folder(path: &Path) -> Result<()> {
    Command::new("explorer.exe")
        .arg(path.to_string_lossy().as_ref())
        .spawn()
        .with_context(|| format!("Failed to open {}", path.display()))?;
    Ok(())
}

pub async fn run(cfg: config::Config) -> Result<()> {
    let scribe_runtime = runtime::ScribeRuntime::from_config(&cfg)?;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<TrayEvent>(4);
    let mut recording: Option<runtime::ActiveRecording> = None;

    let tx1 = tx.clone();
    let tx2 = tx.clone();
    let tx3 = tx.clone();
    let tx4 = tx.clone();
    let tx5 = tx;

    std::thread::spawn(move || {
        let icon = create_default_icon();
        let mut tray = TrayItem::new("Scribe", icon).expect("Failed to create tray icon");

        tray.add_label("Scribe — Meeting Notes").ok();

        tray.add_menu_item("Start Recording", move || {
            dispatch_tray_event(&tx1, TrayEvent::StartRecording, "StartRecording");
        })
        .ok();

        tray.add_menu_item("Stop & Process", move || {
            dispatch_tray_event(&tx2, TrayEvent::StopRecording, "StopRecording");
        })
        .ok();

        tray.inner_mut().add_separator().ok();

        tray.add_menu_item("Open Notes Folder", move || {
            dispatch_tray_event(&tx3, TrayEvent::OpenNotes, "OpenNotes");
        })
        .ok();

        tray.add_menu_item("Settings", move || {
            dispatch_tray_event(&tx4, TrayEvent::OpenSettings, "OpenSettings");
        })
        .ok();

        tray.inner_mut().add_separator().ok();

        tray.add_menu_item("Quit", move || {
            dispatch_tray_event(&tx5, TrayEvent::Quit, "Quit");
        })
        .ok();

        // Keep tray alive
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    });

    tracing::info!("scribe tray running");

    while let Some(event) = rx.recv().await {
        match event {
            TrayEvent::StartRecording => {
                handle_start_recording(&scribe_runtime, &mut recording).await
            }
            TrayEvent::StopRecording => {
                handle_stop_recording(&scribe_runtime, &mut recording).await
            }
            TrayEvent::OpenNotes => handle_open_notes(&cfg),
            TrayEvent::OpenSettings => handle_open_settings(),
            TrayEvent::Quit => handle_quit(&mut recording).await,
        }
    }

    Ok(())
}

async fn handle_start_recording(
    scribe_runtime: &runtime::ScribeRuntime,
    recording: &mut Option<runtime::ActiveRecording>,
) {
    if recording.as_ref().is_some_and(|r| r.is_recording()) {
        tracing::info!("tray start command ignored because recording is already active");
        return;
    }

    let name = tokio::task::spawn_blocking(prompt_session_name_gui)
        .await
        .unwrap_or(None);

    let active_recording = match scribe_runtime.start_recording(runtime::StartRecordingInput {
        name,
        context: scribe_runtime.recording_context_now(),
        events: audio::AudioRecordingEventSink::printing(),
    }) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "tray failed to create recording session");
            return;
        }
    };
    let session_dir = active_recording.session_dir().to_path_buf();
    tracing::info!(session_dir = %session_dir.display(), "tray recording session created");
    *recording = Some(active_recording);
    tracing::info!("tray recording started");
}

async fn handle_stop_recording(
    scribe_runtime: &runtime::ScribeRuntime,
    recording: &mut Option<runtime::ActiveRecording>,
) {
    if !recording.as_ref().is_some_and(|r| r.is_recording()) {
        tracing::info!("tray stop command ignored because no recording is active");
        return;
    }
    let active_recording = recording.take().expect("recording presence checked above");
    let session_dir = active_recording.session_dir().to_path_buf();
    active_recording.stop();
    tracing::info!("tray recording stop requested");
    match active_recording.wait().await {
        Ok(output) => {
            tracing::info!(wav_path = %output.wav_path.display(), "tray recording finalized");
        }
        Err(e) => {
            tracing::error!(error = %e, "tray recording finalization failed");
            return;
        }
    }
    if let Err(e) = process_session(scribe_runtime, session_dir).await {
        tracing::error!(error = %e, "tray processing failed");
    }
}

fn handle_open_notes(cfg: &config::Config) {
    if let Ok(dir) = config::effective_output_dir(cfg) {
        let _ = open_folder(&dir);
    }
}

fn handle_open_settings() {
    if let Ok(path) = config::config_path() {
        let _ = std::process::Command::new("notepad.exe")
            .arg(path.to_string_lossy().as_ref())
            .spawn();
    }
}

async fn handle_quit(recording: &mut Option<runtime::ActiveRecording>) {
    if let Some(active_recording) = recording.take() {
        active_recording.stop();
        if let Err(e) = active_recording.wait().await {
            tracing::error!(error = %e, "tray recording finalization failed during quit");
        }
    }
    tracing::info!("scribe tray exiting");
    std::process::exit(0);
}

async fn process_session(
    runtime: &runtime::ScribeRuntime,
    session_dir: std::path::PathBuf,
) -> Result<()> {
    runtime
        .process_session(runtime::ProcessSessionInput {
            session_dir,
            context: runtime.note_generation_context_now(notes::NotesSystemPrompt::Default),
            events: runtime::SessionProcessingEventSink::printing(),
        })
        .await?;

    Ok(())
}
