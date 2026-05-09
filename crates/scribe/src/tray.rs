use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tray_item::TrayItem;

use scribe_core::{audio, config, opener, process_recording};

enum TrayEvent {
    StartRecording,
    StopRecording,
    OpenNotes,
    OpenSettings,
    Quit,
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

pub async fn run(cfg: config::Config) -> Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<TrayEvent>(4);
    let recording = Arc::new(AtomicBool::new(false));

    let rt = tokio::runtime::Handle::current();

    let tx1 = tx.clone();
    let tx2 = tx.clone();
    let tx3 = tx.clone();
    let tx4 = tx.clone();
    let tx5 = tx;

    std::thread::spawn(move || {
        let icon = create_default_icon();
        let mut tray = TrayItem::new("Scribe", icon).expect("Failed to create tray icon");

        tray.add_label("Scribe — Meeting Notes").ok();

        let rt1 = rt.clone();
        tray.add_menu_item("Start Recording", move || {
            let _ = rt1.block_on(tx1.send(TrayEvent::StartRecording));
        })
        .ok();

        let rt2 = rt.clone();
        tray.add_menu_item("Stop & Process", move || {
            let _ = rt2.block_on(tx2.send(TrayEvent::StopRecording));
        })
        .ok();

        tray.inner_mut().add_separator().ok();

        let rt3 = rt.clone();
        tray.add_menu_item("Open Notes Folder", move || {
            let _ = rt3.block_on(tx3.send(TrayEvent::OpenNotes));
        })
        .ok();

        let rt4 = rt.clone();
        tray.add_menu_item("Settings", move || {
            let _ = rt4.block_on(tx4.send(TrayEvent::OpenSettings));
        })
        .ok();

        tray.inner_mut().add_separator().ok();

        let rt5 = rt.clone();
        tray.add_menu_item("Quit", move || {
            let _ = rt5.block_on(tx5.send(TrayEvent::Quit));
        })
        .ok();

        // Keep tray alive
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    });

    println!("Scribe running in system tray.");

    while let Some(event) = rx.recv().await {
        match event {
            TrayEvent::StartRecording => {
                if recording.load(Ordering::Relaxed) {
                    tracing::info!(
                        "tray start command ignored because recording is already active"
                    );
                    println!("Already recording.");
                    continue;
                }

                // Prompt for name via GUI dialog
                let name = tokio::task::spawn_blocking(prompt_session_name_gui)
                    .await
                    .unwrap_or(None);

                let session_dir = match audio::create_session_dir(&cfg, name.as_deref()) {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::error!(error = %e, "tray failed to create recording session");
                        eprintln!("Failed to create session: {e}");
                        continue;
                    }
                };
                tracing::info!(session_dir = %session_dir.display(), "tray recording session created");
                println!("Session: {}", session_dir.display());

                recording.store(true, Ordering::Relaxed);
                let rec = recording.clone();
                let sample_rate = cfg.sample_rate;
                tokio::task::spawn_blocking(move || {
                    if let Err(e) = audio::record_loopback(rec, sample_rate, session_dir) {
                        tracing::error!(error = %e, "tray recording failed");
                        eprintln!("Recording error: {e}");
                    }
                });
                tracing::info!("tray recording started");
                println!("Recording started.");
            }
            TrayEvent::StopRecording => {
                if !recording.load(Ordering::Relaxed) {
                    tracing::info!("tray stop command ignored because no recording is active");
                    println!("Not recording.");
                    continue;
                }
                recording.store(false, Ordering::Relaxed);
                tracing::info!("tray recording stop requested");
                println!("Recording stopped. Processing...");
                if let Err(e) = process_recording(&cfg).await {
                    tracing::error!(error = %e, "tray processing failed");
                    eprintln!("Processing error: {e}");
                }
            }
            TrayEvent::OpenNotes => {
                if let Ok(dir) = config::effective_output_dir(&cfg) {
                    let _ = opener::open_folder(&dir);
                }
            }
            TrayEvent::OpenSettings => {
                if let Ok(path) = config::config_path() {
                    let _ = std::process::Command::new("notepad.exe")
                        .arg(path.to_string_lossy().as_ref())
                        .spawn();
                }
            }
            TrayEvent::Quit => {
                recording.store(false, Ordering::Relaxed);
                tracing::info!("scribe tray exiting");
                println!("Bye.");
                std::process::exit(0);
            }
        }
    }

    Ok(())
}
