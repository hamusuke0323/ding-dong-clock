use crate::config::AppConfig;
use std::sync::{Arc, Mutex};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, Submenu},
};

pub struct Tray {
    _icon: TrayIcon,
    enabled: CheckMenuItem,
    volume_items: Vec<(u32, CheckMenuItem)>,
    interval_items: Vec<(u64, CheckMenuItem)>,
    select_sound: MenuItem,
    test: MenuItem,
    quit: MenuItem,
}

fn create_icon() -> Icon {
    let mut rgba = Vec::new();

    for y in 0i32..32 {
        for x in 0i32..32 {
            let dx = x - 16;
            let dy = y - 16;

            if dx * dx + dy * dy <= 196 {
                rgba.extend_from_slice(&[210, 160, 55, 255]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }

    Icon::from_rgba(rgba, 32, 32).unwrap()
}

impl Tray {
    pub fn new(config: &AppConfig) -> Self {
        let menu = Menu::new();

        let enabled = CheckMenuItem::new("時報を鳴らす", true, config.enabled, None);

        let volume_menu = Submenu::new("音量", true);
        let volume_items: Vec<_> = [0, 20, 40, 60, 80, 100]
            .into_iter()
            .map(|percent| {
                let item = CheckMenuItem::new(
                    format!("{percent}%"),
                    true,
                    (config.volume * 100.0).round() as u32 == percent,
                    None,
                );
                volume_menu.append(&item).unwrap();
                (percent, item)
            })
            .collect();

        let interval_menu = Submenu::new("鐘の間隔", true);
        let interval_items: Vec<_> = [0, 250, 500, 750, 1000]
            .into_iter()
            .map(|ms| {
                let item = CheckMenuItem::new(
                    format!("{ms} ms"),
                    true,
                    config.bell_interval_ms == ms,
                    None,
                );
                interval_menu.append(&item).unwrap();
                (ms, item)
            })
            .collect();

        let select_sound = MenuItem::new("音声ファイルを選択...", true, None);
        let test = MenuItem::new("試し鳴らし", true, None);
        let quit = MenuItem::new("終了", true, None);

        menu.append(&enabled).unwrap();
        menu.append(&volume_menu).unwrap();
        menu.append(&interval_menu).unwrap();
        menu.append(&select_sound).unwrap();
        menu.append(&test).unwrap();
        menu.append(&quit).unwrap();

        let icon = TrayIconBuilder::new()
            .with_icon(create_icon())
            .with_tooltip("ボンボン時計")
            .with_menu(Box::new(menu))
            .build()
            .unwrap();

        Self {
            _icon: icon,
            enabled,
            volume_items,
            interval_items,
            select_sound,
            test,
            quit,
        }
    }

    pub fn handle_event(
        &self,
        ev: &MenuEvent,
        config: &Arc<Mutex<AppConfig>>,
        audio: &crate::audio::AudioManager,
    ) -> bool {
        if ev.id == self.quit.id() {
            return true;
        }

        if ev.id == self.test.id() {
            let _ = audio.test(1);
            return false;
        }

        if ev.id == self.enabled.id() {
            let mut cfg = config.lock().unwrap();
            cfg.enabled = self.enabled.is_checked();
            save(&cfg);
            return false;
        }

        for (percent, item) in &self.volume_items {
            if ev.id == item.id() {
                let mut cfg = config.lock().unwrap();
                cfg.volume = *percent as f32 / 100.0;

                for (value, other) in &self.volume_items {
                    other.set_checked(value == percent);
                }
                save(&cfg);

                return false;
            }
        }

        for (ms, item) in &self.interval_items {
            if ev.id == item.id() {
                let mut cfg = config.lock().unwrap();
                cfg.bell_interval_ms = *ms;

                for (value, other) in &self.interval_items {
                    other.set_checked(value == ms);
                }
                save(&cfg);

                return false;
            }
        }

        if ev.id == self.select_sound.id() {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("音声ファイル", &["wav", "mp3"])
                .pick_file()
            {
                let path_string = path.to_string_lossy().into_owned();

                {
                    let mut current = config.lock().unwrap();
                    current.sound_file = path_string.clone();

                    if let Err(e) = current.save() {
                        eprintln!("設定保存エラー: {e}");
                    }
                }

                // バックグラウンドで新しい音声を読み込む
                audio.reload_sound(&path_string);
            }

            return false;
        }

        false
    }
}

fn save(config: &AppConfig) {
    if let Err(e) = config.save() {
        eprintln!("設定保存エラー: {e}");
    }
}
