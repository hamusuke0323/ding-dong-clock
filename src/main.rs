#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod audio;
mod clock;
mod config;
mod tray;

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod platform;

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn main() {
    use chrono::Local;
    use clock::Clock;
    use config::AppConfig;
    use std::{
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    use winit::{
        event::Event,
        event_loop::{ControlFlow, EventLoop},
    };

    platform::prepare();

    let initial_config = AppConfig::load().unwrap_or_else(|e| {
        eprintln!("設定読み込みエラー: {e}");
        AppConfig::default()
    });

    let config = Arc::new(Mutex::new(initial_config.clone()));

    let event_loop = EventLoop::new().unwrap();

    let tray = tray::Tray::new(&initial_config);
    let audio_tx = audio::start_worker(Arc::clone(&config));
    let mut clock = Clock::new();

    event_loop
        .run(move |event, elwt| {
            elwt.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(100),
            ));

            if let Event::NewEvents(_) = event {
                // メニュー操作
                if tray.handle_events(&config, &audio_tx) {
                    elwt.exit();
                    return;
                }

                // 時報判定
                if let Some(count) = clock.check(Local::now()) {
                    let enabled = config.lock().unwrap().enabled;

                    if enabled {
                        let _ = audio_tx.try_send(count);
                    }
                }
            }
        })
        .unwrap();
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn main() {
    eprintln!("WindowsとmacOSのみ対応しています");
}
