use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub enabled: bool,
    pub volume: f32,
    pub bell_interval_ms: u64,
    pub sound_file: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            volume: 1.0,
            bell_interval_ms: 1500,
            sound_file: "bell.mp3".to_string(),
        }
    }
}

impl AppConfig {
    pub fn config_path() -> io::Result<PathBuf> {
        let dirs = directories::BaseDirs::new()
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "設定フォルダを取得できません",
                )
            })?;

        #[cfg(target_os = "windows")]
        let path = dirs.config_dir()
            .join("BonbonClock")
            .join("config.json");

        #[cfg(target_os = "macos")]
        let path = dirs.data_dir()
            .join("BonbonClock")
            .join("config.json");

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let path = dirs.config_dir()
            .join("BonbonClock")
            .join("config.json");

        Ok(path)
    }

    // 設定を読み込む
    pub fn load() -> io::Result<Self> {
        let path = Self::config_path()?;

        if !path.exists() {
            let config = Self::default();
            config.save()?;
            return Ok(config);
        }

        let text = fs::read_to_string(path)?;

        let mut config: Self = serde_json::from_str(&text)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        config.normalize();

        Ok(config)
    }

    // 設定を保存する
    pub fn save(&self) -> io::Result<()> {
        let path = Self::config_path()?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(self)?;

        fs::write(path, json)?;

        Ok(())
    }

    pub fn normalize(&mut self) {
        self.volume = if self.volume.is_finite() {
            self.volume.clamp(0.0, 1.0)
        } else {
            1.0
        };

        self.bell_interval_ms = self.bell_interval_ms.clamp(0, 10_000);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();

        assert!(config.enabled);
        assert_eq!(config.volume, 1.0);
        assert_eq!(config.bell_interval_ms, 1500);
        assert_eq!(config.sound_file, "bell.mp3");
    }

    #[test]
    fn test_json_serialization() {
        let config = AppConfig::default();

        let json = serde_json::to_string(&config).unwrap();

        let restored: AppConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.enabled, config.enabled);
        assert_eq!(restored.volume, config.volume);
        assert_eq!(restored.bell_interval_ms, config.bell_interval_ms);
        assert_eq!(restored.sound_file, config.sound_file);
    }

    #[test]
    fn test_missing_fields() {
        let json = r#"{"enabled": false}"#;

        let config: AppConfig = serde_json::from_str(json).unwrap();

        assert!(!config.enabled);
        assert_eq!(config.volume, 1.0);
        assert_eq!(config.bell_interval_ms, 1500);
    }
}
