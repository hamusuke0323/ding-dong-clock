#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;

use crate::config::AppConfig;
use chrono::{Local, Timelike};
use rodio::{Decoder, DeviceSinkBuilder, Player};
use std::sync::{Arc, Mutex};
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::mpsc::{self, SyncSender},
    thread,
    time::{Duration, Instant},
};
use tray_icon::menu::Submenu;
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem},
};
use winit::{
    event::Event,
    event_loop::{ControlFlow, EventLoop},
};

fn bell_file_path_with_name(name: &str) -> PathBuf {
    let path = PathBuf::from(name);

    if path.is_absolute() {
        return path;
    }

    #[cfg(test)]
    {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path.clone())
    }

    #[cfg(not(test))]
    {
        let exe = std::env::current_exe().expect("実行ファイルの場所を取得できません");

        exe.parent().unwrap_or(Path::new(".")).join(path)
    }
}

// 指定回数だけ録音音声を再生

fn play_bell(path: &Path, count: u8, volume: f32, interval_ms: u64) -> Result<(), String> {
    // Windowsの既定の音声出力デバイスを開く
    let device_sink = DeviceSinkBuilder::open_default_sink().map_err(|e| e.to_string())?;

    // 音声プレイヤーを作成
    let player = Player::connect_new(device_sink.mixer());
    player.set_volume(volume);

    for _i in 0..count {
        // 録音した鐘の音を読み込む
        let file = File::open(path).map_err(|e| e.to_string())?;

        let source = Decoder::try_from(file).map_err(|e| e.to_string())?;

        // 再生
        player.append(source);
        thread::sleep(Duration::from_millis(interval_ms));
        player.stop();
    }

    Ok(())
}

// 時報再生専用スレッド
fn start_audio_worker(config: Arc<Mutex<AppConfig>>) -> SyncSender<u32> {
    let (tx, rx) = mpsc::sync_channel::<u32>(0);

    thread::spawn(move || {
        while let Ok(count) = rx.recv() {
            let current = { config.lock().unwrap().clone() };

            let path = bell_file_path_with_name(&current.sound_file);

            if let Err(e) = play_bell(&path, count as u8, current.volume, current.bell_interval_ms)
            {
                eprintln!("鐘の再生に失敗: {e}");
            }
        }
    });

    tx
}

// 12時間制の鐘の回数
fn hour_count(hour: u32) -> u32 {
    match hour % 12 {
        0 => 12,
        n => n,
    }
}

// トレイ用の仮アイコン
fn create_icon() -> Icon {
    let mut rgba = Vec::new();

    for y in 0i32..16 {
        for x in 0i32..16 {
            let dx = x - 8;
            let dy = y - 8;

            if dx * dx + dy * dy <= 49 {
                rgba.extend_from_slice(&[210, 160, 55, 255]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }

    Icon::from_rgba(rgba, 16, 16).unwrap()
}

fn create_menu(
    initial_config: &AppConfig,
) -> (
    Menu,
    CheckMenuItem,
    Vec<(u32, CheckMenuItem)>,
    Vec<(u64, CheckMenuItem)>,
    MenuItem,
    MenuItem,
    MenuItem,
) {
    let menu = Menu::new();

    let enabled = CheckMenuItem::new("時報を鳴らす", true, initial_config.enabled, None);

    let volume_menu = Submenu::new("音量", true);
    let volume_items: Vec<(u32, CheckMenuItem)> = [0, 20, 40, 60, 80, 100]
        .into_iter()
        .map(|percent| {
            let item = CheckMenuItem::new(
                format!("{percent}%"),
                true,
                (initial_config.volume * 100.0).round() as u32 == percent,
                None,
            );
            volume_menu.append(&item).unwrap();
            (percent, item)
        })
        .collect();

    let interval_menu = Submenu::new("再生の間隔", true);

    let interval_items: Vec<(u64, CheckMenuItem)> = [0, 250, 500, 750, 1000, 1250, 1500]
        .into_iter()
        .map(|ms| {
            let item = CheckMenuItem::new(
                format!("{ms} ms"),
                true,
                initial_config.bell_interval_ms == ms,
                None,
            );
            interval_menu.append(&item).unwrap();
            (ms, item)
        })
        .collect();

    let select_sound = MenuItem::new("音声ファイルを選択...", true, None);

    let test = MenuItem::new("サウンドをテスト", true, None);

    let quit = MenuItem::new("終了", true, None);

    menu.append(&enabled).unwrap();
    menu.append(&volume_menu).unwrap();
    menu.append(&interval_menu).unwrap();
    menu.append(&select_sound).unwrap();
    menu.append(&test).unwrap();
    menu.append(&quit).unwrap();

    (
        menu,
        enabled,
        volume_items,
        interval_items,
        select_sound,
        test,
        quit,
    )
}

fn main() {
    let initial_config = AppConfig::load().unwrap_or_else(|e| {
        eprintln!("設定の読み込みに失敗: {e}");
        AppConfig::default()
    });

    let config = Arc::new(Mutex::new(initial_config.clone()));

    let (menu, enabled, volume_items, interval_items, select_sound, test, quit) =
        create_menu(&initial_config);

    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("ボンボン時計")
        .with_icon(create_icon())
        .build()
        .unwrap();

    let event_loop = EventLoop::new().unwrap();
    let audio_tx = start_audio_worker(Arc::clone(&config));

    let mut last_chime: Option<String> = None;

    event_loop
        .run(move |event, elwt| {
            elwt.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(100),
            ));

            if let Event::NewEvents(_) = event {
                // タスクトレイの操作
                while let Ok(ev) = MenuEvent::receiver().try_recv() {
                    if ev.id == quit.id() {
                        elwt.exit();
                        return;
                    }

                    if ev.id == test.id() {
                        // 再生中なら新しい要求を破棄
                        let _ = audio_tx.try_send(1);
                        continue;
                    }

                    if ev.id == enabled.id() {
                        let mut cfg = config.lock().unwrap();
                        cfg.enabled = enabled.is_checked();

                        if let Err(e) = cfg.save() {
                            eprintln!("設定の保存に失敗: {e}");
                        }
                        continue;
                    }

                    for (percent, item) in &volume_items {
                        if ev.id == item.id() {
                            let mut cfg = config.lock().unwrap();
                            cfg.volume = *percent as f32 / 100.0;

                            for (value, other) in &volume_items {
                                other.set_checked(value == percent);
                            }

                            if let Err(e) = cfg.save() {
                                eprintln!("設定の保存に失敗: {e}");
                            }
                            break;
                        }
                    }

                    for (ms, item) in &interval_items {
                        if ev.id == item.id() {
                            let mut cfg = config.lock().unwrap();
                            cfg.bell_interval_ms = *ms;

                            for (value, other) in &interval_items {
                                other.set_checked(value == ms);
                            }

                            if let Err(e) = cfg.save() {
                                eprintln!("設定の保存に失敗: {e}");
                            }
                            break;
                        }
                    }

                    if ev.id == select_sound.id() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("音声ファイル", &["wav", "mp3"])
                            .pick_file()
                        {
                            let mut cfg = config.lock().unwrap();

                            cfg.sound_file = path.to_string_lossy().into_owned();

                            if let Err(e) = cfg.save() {
                                eprintln!("設定の保存に失敗: {e}");
                            }
                        }
                    }
                }

                let cfg = config.lock().unwrap();
                if !cfg.enabled {
                    return;
                }

                let now = Local::now();
                if now.minute() != 0 || now.second() >= 2 {
                    return;
                }

                // 毎正時の最初の数秒間だけ判定
                let key = now.format("%Y-%m-%d %H").to_string();

                if last_chime.as_ref() != Some(&key) {
                    last_chime = Some(key);

                    let count = hour_count(now.hour());
                    let _ = audio_tx.try_send(count);
                }
            }
        })
        .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, Timelike};

    #[test]
    fn hourly_chimes() {
        assert_eq!(hour_count(0), 12);
        assert_eq!(hour_count(1), 1);
        assert_eq!(hour_count(12), 12);
        assert_eq!(hour_count(13), 1);
        assert_eq!(hour_count(23), 11);
    }

    // 現在時刻に合わせて鐘を鳴らす
    #[test]
    #[ignore = "実際に音声を再生するため手動実行"]
    fn test_play_bell_now() {
        // 現在時刻を取得
        let now = Local::now();
        let hour = now.hour();

        // 鳴らす回数を計算
        let count = hour_count(hour);

        println!("現在時刻: {}", now.format("%Y-%m-%d %H:%M:%S"));
        println!("鐘を {} 回鳴らします", count);

        // 音声ファイルのパスを取得
        let path = bell_file_path_with_name("bell.mp3");

        assert!(path.exists(), "鐘の音声ファイルがありません: {:?}", path);

        // 鐘を再生
        play_bell(&path, count as u8, 1.0, 1500).expect("鐘の再生に失敗しました");

        println!("再生テスト完了");
    }
}
