use crate::config::AppConfig;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};
use std::sync::mpsc::SyncSender;
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self},
    },
    thread,
    time::Duration,
};

pub fn sound_path(name: &str) -> PathBuf {
    let path = PathBuf::from(name);

    if path.is_absolute() {
        return path;
    }

    #[cfg(test)]
    {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(&path)
    }

    #[cfg(not(test))]
    {
        std::env::current_exe()
            .expect("実行ファイルの場所を取得できません")
            .parent()
            .unwrap_or(Path::new("."))
            .join(&path)
    }
}

fn play_sequence(
    mixer: &rodio::mixer::Mixer,
    path: &Path,
    count: u32,
    volume: f32,
    interval_ms: u64,
) -> Result<(), String> {
    let mut handles = Vec::new();

    for i in 0..count {
        let mixer = mixer.clone();
        let path = path.to_path_buf();

        // 鐘1回につき独立したスレッドを起動
        let handle = thread::spawn(move || -> Result<(), String> {
            let player = Player::connect_new(&mixer);
            player.set_volume(volume.clamp(0.0, 1.0));

            let file = File::open(&path).map_err(|e| e.to_string())?;

            let source = Decoder::try_from(file).map_err(|e| e.to_string())?;

            player.append(source);

            // この鐘の余韻が終わるまでPlayerを保持
            // 他の鐘の開始は妨げない
            player.sleep_until_end();

            Ok(())
        });

        handles.push(handle);

        // 最後の鐘の後には待機しない
        if i + 1 < count {
            thread::sleep(Duration::from_millis(interval_ms));
        }
    }

    // 全ての鐘の再生が終わるまで待つ
    // 各鐘は並列再生なので、次の鐘の開始は妨げない
    for handle in handles {
        match handle.join() {
            Ok(result) => result?,
            Err(_) => return Err("鐘の再生スレッドが異常終了しました".into()),
        }
    }

    Ok(())
}

pub fn start_worker(config: Arc<Mutex<AppConfig>>) -> SyncSender<u32> {
    let (tx, rx) = mpsc::sync_channel::<u32>(0);

    thread::spawn(move || {
        // 音声出力はこのスレッドで1回だけ開く
        let device: MixerDeviceSink = match DeviceSinkBuilder::open_default_sink() {
            Ok(device) => device,
            Err(e) => {
                eprintln!("音声デバイスを開けません: {e}");
                return;
            }
        };

        // Mixerは複数のPlayerで共有できる
        let mixer = device.mixer().clone();

        // 再生要求を受け取る
        while let Ok(count) = rx.recv() {
            let current = config.lock().unwrap().clone();
            let mixer = mixer.clone();
            let path = sound_path(&current.sound_file);

            if let Err(e) = play_sequence(
                &mixer,
                &path,
                count,
                current.volume,
                current.bell_interval_ms,
            ) {
                eprintln!("鐘の再生エラー: {e}");
            }
        }

        // deviceは管理スレッドの終了まで保持される
        drop(device);
    });

    tx
}

#[test]
#[ignore = "実際に鐘を再生するため手動実行"]
fn test_overlapping_chimes() {
    let path = sound_path("bell.mp3");

    let device =
        DeviceSinkBuilder::open_default_sink().unwrap();

    let mixer = device.mixer().clone();

    // 500ms間隔で4回鳴らす
    play_sequence(
        &mixer,
        &path,
        4,
        1.0,
        1500,
    )
        .unwrap();
}
