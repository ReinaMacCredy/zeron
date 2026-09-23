use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value, json};

use super::audit_file::{create_private_dir, replace_private};
use super::scroll_calibration::{
    MAX_SCROLL_GEARING, MIN_SCROLL_GEARING, observed_gearing, smoothed_gearing,
};

pub const SCROLL_GEARING_FILE_NAME: &str = "computer-scroll-gearing.json";
const MAX_APP_ENTRIES: usize = 64;
const FILE_VERSION: u64 = 1;
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppGearingEntry {
    gearing: f64,
    samples: u64,
    updated_at: f64,
}

fn parse_entry(value: &Value) -> Option<AppGearingEntry> {
    let gearing = value
        .get("gearing")?
        .as_f64()
        .filter(|g| g.is_finite() && (MIN_SCROLL_GEARING..=MAX_SCROLL_GEARING).contains(g))?;
    let samples = value
        .get("samples")?
        .as_f64()
        .filter(|s| s.fract() == 0.0 && *s >= 1.0 && *s <= MAX_SAFE_INTEGER)?;
    let updated_at = value.get("updatedAt")?.as_f64().filter(|t| t.is_finite())?;
    Some(AppGearingEntry {
        gearing,
        samples: samples as u64,
        updated_at,
    })
}

type Clock = Box<dyn Fn() -> f64 + Send + Sync>;

pub struct ScrollGearingFile {
    file_path: Option<PathBuf>,
    apps: Vec<(String, AppGearingEntry)>,
    now: Clock,
}

impl ScrollGearingFile {
    pub fn new(dir: Option<&Path>) -> Self {
        Self::with_clock(
            dir,
            Box::new(|| chrono::Utc::now().timestamp_millis() as f64),
        )
    }

    pub fn with_clock(dir: Option<&Path>, now: Clock) -> Self {
        let file_path = dir.map(|dir| dir.join(SCROLL_GEARING_FILE_NAME));
        let mut file = Self {
            apps: file_path.as_deref().map(load).unwrap_or_default(),
            file_path,
            now,
        };
        while file.apps.len() > MAX_APP_ENTRIES {
            file.evict_stalest();
        }
        file
    }

    pub fn get(&self, app_key: Option<&str>) -> Option<f64> {
        let app_key = app_key?;
        self.apps
            .iter()
            .find(|(key, _)| key == app_key)
            .map(|(_, entry)| entry.gearing)
    }

    pub fn learn(&mut self, app_key: Option<&str>, injected: f64, traveled: f64) {
        let Some(app_key) = app_key else {
            return;
        };
        let Some(observed) = observed_gearing(injected, traveled) else {
            return;
        };
        let previous = self
            .apps
            .iter()
            .position(|(key, _)| key == app_key)
            .map(|index| self.apps[index].1);
        if previous.is_none() && self.apps.len() >= MAX_APP_ENTRIES {
            self.evict_stalest();
        }
        let entry = AppGearingEntry {
            gearing: smoothed_gearing(previous.map(|entry| entry.gearing), observed),
            samples: previous.map_or(0, |entry| entry.samples) + 1,
            updated_at: (self.now)(),
        };
        match self.apps.iter_mut().find(|(key, _)| key == app_key) {
            Some((_, slot)) => *slot = entry,
            None => self.apps.push((app_key.to_owned(), entry)),
        }
        self.persist();
    }

    fn evict_stalest(&mut self) {
        let stalest = self.apps.iter().enumerate().fold(
            None::<(usize, f64)>,
            |stalest, (index, (_, entry))| match stalest {
                Some((_, updated_at)) if entry.updated_at >= updated_at => stalest,
                _ => Some((index, entry.updated_at)),
            },
        );
        if let Some((index, _)) = stalest {
            self.apps.remove(index);
        }
    }

    // Synchronous like the engine's other preference writes; a disk error must not fail a scroll.
    fn persist(&self) {
        let Some(path) = &self.file_path else {
            return;
        };
        let apps: Map<String, Value> = self
            .apps
            .iter()
            .map(|(key, entry)| (key.clone(), json!(entry)))
            .collect();
        let content = json!({ "version": FILE_VERSION, "apps": apps }).to_string();
        let written = path
            .parent()
            .map_or(Ok(()), create_private_dir)
            .and_then(|()| replace_private(path, content.as_bytes()));
        if let Err(error) = written {
            tracing::debug!(%error, "scroll gearing save failed");
        }
    }
}

fn load(path: &Path) -> Vec<(String, AppGearingEntry)> {
    let Some(data) = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    else {
        return Vec::new();
    };
    if data.get("version").and_then(Value::as_f64) != Some(FILE_VERSION as f64) {
        return Vec::new();
    }
    let Some(apps) = data.get("apps").and_then(Value::as_object) else {
        return Vec::new();
    };
    apps.iter()
        .filter_map(|(key, value)| Some((key.clone(), parse_entry(value)?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    fn clock() -> (Arc<AtomicU64>, Clock) {
        let time = Arc::new(AtomicU64::new(1_000));
        let reader = time.clone();
        (time, Box::new(move || reader.load(Ordering::SeqCst) as f64))
    }

    #[test]
    fn learns_per_app_gearing_and_restores_it_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let (_, now) = clock();
        let mut file = ScrollGearingFile::with_clock(Some(dir.path()), now);
        assert_eq!(file.get(Some("com.google.Chrome")), None);
        file.learn(Some("com.google.Chrome"), 400.0, 2_800.0);
        assert_eq!(file.get(Some("com.google.Chrome")), Some(7.0));
        file.learn(Some("com.google.Chrome"), 400.0, 1_200.0);
        assert_eq!(file.get(Some("com.google.Chrome")), Some(5.0));

        let saved: Value = serde_json::from_slice(
            &std::fs::read(dir.path().join(SCROLL_GEARING_FILE_NAME)).unwrap(),
        )
        .unwrap();
        assert_eq!(saved["version"], 1);
        assert_eq!(saved["apps"]["com.google.Chrome"]["samples"], 2);
        assert_eq!(saved["apps"]["com.google.Chrome"]["updatedAt"], 1_000.0);

        let restored = ScrollGearingFile::new(Some(dir.path()));
        assert_eq!(restored.get(Some("com.google.Chrome")), Some(5.0));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(SCROLL_GEARING_FILE_NAME))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn applies_the_hot_stores_admissibility_rules() {
        let mut file = ScrollGearingFile::new(None);
        file.learn(Some("app"), 400.0, 0.0);
        file.learn(Some("app"), 400.0, -2_800.0);
        file.learn(Some("app"), 20.0, 140.0);
        file.learn(Some("app"), 400.0, 400_000.0);
        file.learn(None, 400.0, 2_800.0);
        assert_eq!(file.get(Some("app")), None);
        assert_eq!(file.get(None), None);
    }

    #[test]
    fn reads_missing_corrupt_or_unknown_version_files_as_never_measured() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SCROLL_GEARING_FILE_NAME);
        assert_eq!(
            ScrollGearingFile::new(Some(dir.path())).get(Some("a")),
            None
        );
        for content in [
            "not json".to_owned(),
            json!({ "version": 2, "apps": { "a": { "gearing": 2, "samples": 1, "updatedAt": 1 } } })
                .to_string(),
            json!({ "version": 1, "apps": [] }).to_string(),
        ] {
            std::fs::write(&path, content).unwrap();
            assert_eq!(ScrollGearingFile::new(Some(dir.path())).get(Some("a")), None);
        }
    }

    #[test]
    fn skips_individually_invalid_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(SCROLL_GEARING_FILE_NAME),
            json!({
                "version": 1,
                "apps": {
                    "good": { "gearing": 3, "samples": 2, "updatedAt": 5 },
                    "too-fast": { "gearing": 51, "samples": 1, "updatedAt": 5 },
                    "no-samples": { "gearing": 3, "samples": 0, "updatedAt": 5 },
                    "fractional": { "gearing": 3, "samples": 1.5, "updatedAt": 5 },
                    "no-time": { "gearing": 3, "samples": 1 },
                    "null": null,
                },
            })
            .to_string(),
        )
        .unwrap();
        let file = ScrollGearingFile::new(Some(dir.path()));
        assert_eq!(file.get(Some("good")), Some(3.0));
        for key in ["too-fast", "no-samples", "fractional", "no-time", "null"] {
            assert_eq!(file.get(Some(key)), None, "{key}");
        }
    }

    #[test]
    fn evicts_the_least_recently_updated_app_not_the_first_inserted() {
        let (time, now) = clock();
        let mut file = ScrollGearingFile::with_clock(None, now);
        for index in 0..MAX_APP_ENTRIES {
            time.store(1_000 + index as u64, Ordering::SeqCst);
            file.learn(Some(&format!("app-{index}")), 400.0, 800.0);
        }
        time.store(5_000, Ordering::SeqCst);
        file.learn(Some("app-0"), 400.0, 800.0);
        file.learn(Some("new"), 400.0, 800.0);
        assert_eq!(file.get(Some("app-0")), Some(2.0));
        assert_eq!(file.get(Some("app-1")), None);
        assert_eq!(file.get(Some("new")), Some(2.0));
    }

    #[test]
    fn trims_an_oversized_file_to_the_stalest_first() {
        let dir = tempfile::tempdir().unwrap();
        let apps: Map<String, Value> = (0..MAX_APP_ENTRIES + 2)
            .map(|index| {
                (
                    format!("app-{index}"),
                    json!({ "gearing": 2, "samples": 1, "updatedAt": index }),
                )
            })
            .collect();
        std::fs::write(
            dir.path().join(SCROLL_GEARING_FILE_NAME),
            json!({ "version": 1, "apps": apps }).to_string(),
        )
        .unwrap();
        let file = ScrollGearingFile::new(Some(dir.path()));
        assert_eq!(file.get(Some("app-0")), None);
        assert_eq!(file.get(Some("app-1")), None);
        assert_eq!(file.get(Some("app-2")), Some(2.0));
        assert_eq!(
            file.get(Some(&format!("app-{}", MAX_APP_ENTRIES + 1))),
            Some(2.0)
        );
    }

    #[test]
    fn swallows_write_failures() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "x").unwrap();
        let mut file = ScrollGearingFile::new(Some(&blocker));
        file.learn(Some("app"), 400.0, 800.0);
        assert_eq!(file.get(Some("app")), Some(2.0));
    }
}
