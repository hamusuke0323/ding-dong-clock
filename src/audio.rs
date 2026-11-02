use crate::config::AppConfig;
use rodio::{Decoder, DeviceSinkBuilder, Player, Source};
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
    },
    thread,
    time::{Duration, Instant},
};

type CachedSound = rodio::source::Buffered<rodio::buffer::SamplesBuffer>;

struct SoundCache {
    path: PathBuf,
    sound: CachedSound,
}

impl SoundCache {
    fn load(path: PathBuf) -> Result<Self, String> {
        let file = File::open(&path).map_err(|e| e.to_string())?;

        let decoder = Decoder::try_from(file).map_err(|e| e.to_string())?;

        // ファイル全体をデコードしてPCMとして保存
        let samples = decoder.record();

        // clone可能な音声ソースにする
        let sound = samples.buffered();

        Ok(Self { path, sound })
    }
}

struct SoundStoreState {
    // 現在使用可能な音声
    current: Option<CachedSound>,

    // 現在使用可能な音声のファイルパス
    current_path: Option<PathBuf>,

    // 最後に要求された読み込みの識別番号
    generation: u64,

    // 最後に要求されたファイルパス
    requested_path: Option<PathBuf>,
}

struct SoundStore {
    state: Mutex<SoundStoreState>,
}

impl SoundStore {
    fn new() -> Self {
        Self {
            state: Mutex::new(SoundStoreState {
                current: None,
                current_path: None,
                generation: 0,
                requested_path: None,
            }),
        }
    }

    // 再生側は短時間のロックだけで取得できる
    fn current_sound(&self) -> Option<CachedSound> {
        self.state.lock().unwrap()
            .current
            .clone()
    }

    // 音声読み込みを別スレッドで開始
    fn preload(self: &Arc<Self>, path: PathBuf) {
        let generation = {
            let mut state = self.state.lock().unwrap();

            state.generation =
                state.generation.wrapping_add(1);

            state.requested_path = Some(path.clone());
            state.generation
        };

        let store = Arc::clone(self);

        thread::spawn(move || {
            let result = SoundCache::load(path.clone());

            match result {
                Ok(loaded) => {
                    let mut state = store.state.lock().unwrap();

                    // 古い読み込み結果は採用しない
                    if state.generation != generation {
                        return;
                    }

                    state.current = Some(loaded.sound);
                    state.current_path = Some(path);

                    println!("鐘の音声キャッシュを更新しました");
                }

                Err(e) => {
                    let state = store.state.lock().unwrap();

                    // すでに別のファイルが選ばれていたら
                    // 古い要求のエラーは無視する
                    if state.generation == generation {
                        eprintln!(
                            "音声の先読みエラー: {e}"
                        );
                    }
                }
            }
        });
    }
}

const MAX_PLAYERS: usize = 12;
const CANCEL_POLL_MS: u64 = 5;

type Mixer = rodio::mixer::Mixer;

enum Command {
    Hourly(u32),
    Test { count: u32, generation: u64 },
}

struct PlaybackState {
    hourly_active: bool,
    active_tests: usize,
    generation: u64,
    test_cancel: Arc<AtomicBool>,
}

struct Shared {
    state: Mutex<PlaybackState>,
    changed: Condvar,
}

impl Shared {
    fn new() -> Self {
        Self {
            state: Mutex::new(PlaybackState {
                hourly_active: false,
                active_tests: 0,
                generation: 0,
                test_cancel: Arc::new(AtomicBool::new(false)),
            }),
            changed: Condvar::new(),
        }
    }

    fn finish_hourly(&self) {
        let mut state = self.state.lock().unwrap();
        state.hourly_active = false;
        state.test_cancel = Arc::new(AtomicBool::new(false));
        self.changed.notify_all();
    }

    fn wait_for_tests(&self) {
        let mut state = self.state.lock().unwrap();

        while state.active_tests != 0 {
            state = self.changed.wait(state).unwrap();
        }
    }

    fn begin_test(self: &Arc<Self>, generation: u64) -> Option<(TestPermit, Arc<AtomicBool>)> {
        let mut state = self.state.lock().unwrap();

        if state.hourly_active
            || state.generation != generation
            || state.active_tests >= MAX_PLAYERS
        {
            return None;
        }

        state.active_tests += 1;

        let cancel = Arc::clone(&state.test_cancel);

        Some((
            TestPermit {
                shared: Arc::clone(self),
            },
            cancel,
        ))
    }

    fn test_allowed(&self, generation: u64) -> bool {
        let state = self.state.lock().unwrap();

        !state.hourly_active && state.generation == generation
    }
}

struct TestPermit {
    shared: Arc<Shared>,
}

impl Drop for TestPermit {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap();
        state.active_tests -= 1;
        self.shared.changed.notify_all();
    }
}

struct HourlyGuard {
    shared: Arc<Shared>,
}

impl Drop for HourlyGuard {
    fn drop(&mut self) {
        self.shared.finish_hourly();
    }
}

#[derive(Clone)]
pub struct AudioManager {
    tx: Sender<Command>,
    shared: Arc<Shared>,
    sounds: Arc<SoundStore>,
}

impl AudioManager {
    pub fn new(config: Arc<Mutex<AppConfig>>) -> Self {
        let (tx, rx) = mpsc::channel::<Command>();

        let shared = Arc::new(Shared::new());
        let sounds = Arc::new(SoundStore::new());

        // 起動時の設定から先読みを開始
        let initial_path = {
            let current = config.lock().unwrap();
            sound_path(&current.sound_file)
        };

        sounds.preload(initial_path);

        let worker_shared = Arc::clone(&shared);
        let worker_sounds = Arc::clone(&sounds);

        thread::spawn(move || {
            let device =
                match DeviceSinkBuilder::open_default_sink() {
                    Ok(device) => device,
                    Err(e) => {
                        eprintln!("音声出力エラー: {e}");
                        return;
                    }
                };

            let mixer = device.mixer().clone();

            while let Ok(command) = rx.recv() {
                let current = config.lock().unwrap().clone();

                match command {
                    Command::Hourly(count) => {
                        let shared =
                            Arc::clone(&worker_shared);
                        let sounds =
                            Arc::clone(&worker_sounds);
                        let mixer = mixer.clone();

                        thread::spawn(move || {
                            let _guard = HourlyGuard {
                                shared: Arc::clone(&shared),
                            };

                            shared.wait_for_tests();

                            let Some(sound) =
                                sounds.current_sound()
                            else {
                                eprintln!(
                                    "時報: 音声の先読みが未完了です"
                                );
                                return;
                            };

                            play_hourly(
                                mixer,
                                sound,
                                count,
                                current.volume,
                                current.bell_interval_ms,
                            );
                        });
                    }

                    Command::Test {
                        count,
                        generation,
                    } => {
                        if !worker_shared
                            .test_allowed(generation)
                        {
                            continue;
                        }

                        let Some(sound) =
                            worker_sounds.current_sound()
                        else {
                            eprintln!(
                                "テスト: 音声の先読みが未完了です"
                            );
                            continue;
                        };

                        let mixer = mixer.clone();
                        let shared =
                            Arc::clone(&worker_shared);

                        thread::spawn(move || {
                            play_test(
                                mixer,
                                sound,
                                count,
                                current.volume,
                                current.bell_interval_ms,
                                shared,
                                generation,
                            );
                        });
                    }
                }
            }

            drop(device);
        });

        Self {
            tx,
            shared,
            sounds,
        }
    }

    // 設定変更時に呼び出す
    pub fn reload_sound(&self, path: &str) {
        self.sounds.preload(sound_path(path));
    }

    // 時報を最優先で開始する
    pub fn hourly(&self, count: u32) {
        if count == 0 {
            return;
        }

        {
            let mut state = self.shared.state.lock().unwrap();

            // 時報の二重起動を防ぐ
            if state.hourly_active {
                return;
            }

            state.hourly_active = true;
            state.generation = state.generation.wrapping_add(1);

            // 既存のテスト再生に停止を通知
            state.test_cancel.store(true, Ordering::Release);
        }

        if self.tx.send(Command::Hourly(count.min(12))).is_err() {
            self.shared.finish_hourly();
            eprintln!("時報要求を送信できません");
        }
    }

    // 時報中のテスト要求は受け付けない
    pub fn test(&self, count: u32) {
        if count == 0 {
            return;
        }

        let generation = {
            let state = self.shared.state.lock().unwrap();

            if state.hourly_active {
                return;
            }

            state.generation
        };

        let _ = self.tx.send(Command::Test {
            count: count.min(12),
            generation,
        });
    }
}

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

fn play_one(mixer: &Mixer, sound: CachedSound, volume: f32, cancel: Option<&AtomicBool>) {
    if cancel.is_some_and(|c| c.load(Ordering::Acquire)) {
        return;
    }

    let player = Player::connect_new(mixer);
    player.set_volume(volume.clamp(0.0, 1.0));

    // デコード済み音声を再生
    player.append(sound);

    if let Some(cancel) = cancel {
        while !player.empty() {
            if cancel.load(Ordering::Acquire) {
                player.stop();
                break;
            }

            thread::sleep(Duration::from_millis(CANCEL_POLL_MS));
        }
    } else {
        player.sleep_until_end();
    }
}

fn wait_until(start: Instant, interval_ms: u64, i: u32) {
    let offset = Duration::from_millis(interval_ms).saturating_mul(i);

    if let Some(target) = start.checked_add(offset) {
        if let Some(wait) = target.checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
    }
}

fn play_hourly(mixer: Mixer, sound: CachedSound, count: u32, volume: f32, interval_ms: u64) {
    let start = Instant::now();
    let mut handles = Vec::new();

    for i in 0..count.min(12) {
        wait_until(start, interval_ms, i);

        let mixer = mixer.clone();
        let sound = sound.clone();

        handles.push(thread::spawn(move || play_one(&mixer, sound, volume, None)));
    }

    for handle in handles {
        if handle.join().is_err() {
            eprintln!("鐘のスレッドが異常終了しました");
        }
    }
}

fn play_test(
    mixer: Mixer,
    sound: CachedSound,
    count: u32,
    volume: f32,
    interval_ms: u64,
    shared: Arc<Shared>,
    generation: u64,
) {
    let start = Instant::now();
    let mut handles = Vec::new();

    for i in 0..count.min(12) {
        // 次の鐘までの待機中にも
        // 時報開始を検出する
        loop {
            if !shared.test_allowed(generation) {
                break;
            }

            let offset = Duration::from_millis(interval_ms).saturating_mul(i);

            let Some(target) = start.checked_add(offset) else {
                break;
            };

            let now = Instant::now();
            if now >= target {
                break;
            }

            thread::sleep(
                target
                    .duration_since(now)
                    .min(Duration::from_millis(CANCEL_POLL_MS)),
            );
        }

        let Some((permit, cancel)) = shared.begin_test(generation) else {
            break;
        };

        let mixer = mixer.clone();
        let sound = sound.clone();

        handles.push(thread::spawn(move || {
            let _permit = permit;
            play_one(&mixer, sound, volume, Some(&cancel));
        }));
    }

    for handle in handles {
        let _ = handle.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hourly_blocks_tests() {
        let shared = Arc::new(Shared::new());

        // テスト再生枠を1個確保
        let (permit, cancel) = shared.begin_test(0).unwrap();

        assert!(!cancel.load(Ordering::Acquire));

        // 時報開始時と同じ状態変更
        {
            let mut state = shared.state.lock().unwrap();

            state.hourly_active = true;
            state.generation += 1;
            state.test_cancel.store(true, Ordering::Release);
        }

        // 既存テストに停止要求が届く
        assert!(cancel.load(Ordering::Acquire));

        // 新しいテストは開始できない
        assert!(shared.begin_test(1).is_none());

        // 既存のテスト枠を解放
        drop(permit);
        shared.wait_for_tests();

        // 時報終了
        shared.finish_hourly();

        // 古い世代のテストは開始できない
        assert!(shared.begin_test(0).is_none());

        // 新しい世代のテストは開始可能
        assert!(shared.begin_test(1).is_some());
    }

    #[test]
    #[ignore = "bell.wav を読み込むため手動実行"]
    fn test_sound_preload() {
        let sounds = Arc::new(SoundStore::new());

        sounds.preload(sound_path("bell.mp3"));

        let deadline =
            Instant::now() + Duration::from_secs(5);

        while sounds.current_sound().is_none()
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }

        assert!(
            sounds.current_sound().is_some(),
            "音声の先読みが完了しませんでした"
        );
    }
}
