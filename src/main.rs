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
use {
    audio::AudioManager,
    chrono::{Local, Timelike},
    clock::Clock,
    config::AppConfig,
    std::{
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    },
    tray::Tray,
    tray_icon::menu::MenuEvent,
    winit::{
        application::ApplicationHandler,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
        window::WindowId,
    },
};

#[cfg(any(target_os = "windows", target_os = "macos"))]
#[derive(Debug)]
enum UserEvent {
    TrayMenu(MenuEvent),
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
struct BonbonApp {
    config: Arc<Mutex<AppConfig>>,
    audio: AudioManager,
    clock: Clock,
    tray: Option<Tray>,
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
impl BonbonApp {
    fn new(config: AppConfig) -> Self {
        let config = Arc::new(Mutex::new(config));

        let audio = AudioManager::new(Arc::clone(&config));

        Self {
            config,
            audio,
            clock: Clock::new(),
            tray: None,
        }
    }

    fn check_hourly(&mut self) {
        if let Some(count) = self.clock.check(Local::now()) {
            let enabled = self.config.lock().unwrap().enabled;

            if enabled {
                self.audio.hourly(count);
            }
        }
    }

    fn schedule_next_check(&self, event_loop: &ActiveEventLoop) {
        let now = Local::now();

        // 次の秒境界まで待機
        let nanos = now.nanosecond();
        let wait = Duration::from_nanos(1_000_000_000 - nanos as u64);

        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + wait));
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
impl ApplicationHandler<UserEvent> for BonbonApp {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, _cause: winit::event::StartCause) {
        self.check_hourly();
        self.schedule_next_check(event_loop);
    }

    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        if self.tray.is_none() {
            let current = self.config.lock().unwrap().clone();

            self.tray = Some(Tray::new(&current));
        }

        self.check_hourly();
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::TrayMenu(menu_event) => {
                if let Some(tray) = &self.tray {
                    if tray.handle_event(&menu_event, &self.config, &self.audio) {
                        event_loop.exit();
                    }
                }
            }
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        _event: WindowEvent,
    ) {
        // ウィンドウは作成しない
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn start_tray_event_forwarder(proxy: EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        let receiver = MenuEvent::receiver();

        while let Ok(event) = receiver.recv() {
            if proxy.send_event(UserEvent::TrayMenu(event)).is_err() {
                break;
            }
        }
    });
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn main() {
    platform::prepare();

    let config = AppConfig::load().unwrap_or_else(|e| {
        eprintln!("設定読み込みエラー: {e}");
        AppConfig::default()
    });

    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("イベントループを作成できません");

    let proxy = event_loop.create_proxy();
    start_tray_event_forwarder(proxy);

    let mut app = BonbonApp::new(config);

    event_loop
        .run_app(&mut app)
        .expect("イベントループの実行に失敗しました");
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn main() {
    eprintln!("WindowsとmacOSのみ対応しています");
}
