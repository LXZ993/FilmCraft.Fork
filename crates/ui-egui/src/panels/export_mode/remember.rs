//! The export settings in use when the app closes come back at the next launch: the preset and
//! its (possibly edited) settings, the output folder, the open sections, and the Quick Export
//! preset and folder. Kept in [`EXPORT_FILE`] beside the preferences, like the remembered
//! workspace layout (#2421). The file name is not kept: it follows the sequence being exported.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ExportUi;

/// The remembered export settings, in the data directory.
pub const EXPORT_FILE: &str = "export.json";

/// Preferences are untrusted; bound reading before JSON can allocate from it.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RememberedExport {
    pub preset: String,
    /// [`filmcraft_engine::export::ExportSettings`] as JSON: settings a newer or older version
    /// wrote that no longer read back are ignored, not an error.
    pub settings: Value,
    pub location: String,
    pub open_sections: Vec<String>,
    pub quick_preset: String,
    pub quick_folder: String,
}

impl RememberedExport {
    /// What of `ex` is worth keeping across launches.
    pub fn of(ex: &ExportUi) -> Self {
        let quick = std::path::Path::new(ex.quick_path.trim());
        let quick_folder =
            if ex.quick_path.ends_with(['/', '\\']) { Some(quick) } else { quick.parent() }.map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
        RememberedExport {
            preset: ex.preset.clone(),
            settings: super::settings_json(&ex.settings),
            location: ex.location.trim().to_string(),
            open_sections: ex.open_sections.clone(),
            quick_preset: ex.quick_preset.clone(),
            quick_folder,
        }
    }

    /// Bring the remembered settings back into `ex`. Folders that no longer exist are skipped, so
    /// an unplugged drive falls back to the default folder.
    /// `known_preset` says whether a preset name is in the library: settings remembered under a
    /// preset that is gone are not brought back (they may come from another version).
    pub fn apply(&self, ex: &mut ExportUi, known_preset: impl Fn(&str) -> bool) {
        let preset_ok = self.preset == super::CUSTOM || known_preset(&self.preset);
        if preset_ok
            && !self.settings.is_null()
            && let Ok(settings) = serde_json::from_value(self.settings.clone())
        {
            ex.settings = settings;
            ex.preset = self.preset.clone();
        }
        if !self.location.is_empty() && dir_exists(&self.location) {
            ex.location = self.location.clone();
        }
        if !self.open_sections.is_empty() {
            ex.open_sections = self.open_sections.clone();
        }
        if !self.quick_preset.is_empty() && known_preset(&self.quick_preset) {
            ex.quick_preset = self.quick_preset.clone();
        }
        if !self.quick_folder.is_empty() && dir_exists(&self.quick_folder) {
            ex.quick_path = format!("{}{}", self.quick_folder.trim_end_matches(['/', '\\']), std::path::MAIN_SEPARATOR);
        }
    }

    pub fn load(path: &std::path::Path) -> Option<Self> {
        use std::io::Read as _;
        let file = std::fs::File::open(path).ok()?;
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() as u64 > MAX_BYTES {
            return None;
        }
        serde_json::from_slice(&bytes).ok()
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(tmp, path)
    }
}

fn dir_exists(d: &str) -> bool {
    std::path::Path::new(&filmcraft_engine::export_tools::expand_home(d.trim())).is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_folder_and_quick_export_come_back() {
        let dir = std::env::temp_dir().join(format!("fc-export-remember-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let folder = dir.to_string_lossy().to_string();
        let mut ex = ExportUi {
            preset: "Custom".into(),
            location: folder.clone(),
            file_name: "Not kept".into(),
            open_sections: vec!["audio".into()],
            quick_preset: "Match Source - High bitrate".into(),
            quick_path: dir.join("cut.mp4").to_string_lossy().to_string(),
            ..Default::default()
        };
        ex.settings.scale = 0.5;

        let file = dir.join(EXPORT_FILE);
        RememberedExport::of(&ex).save(&file).unwrap();
        let mut back = ExportUi::default();
        RememberedExport::load(&file).unwrap().apply(&mut back, |_| false);

        assert_eq!(back.preset, "Custom");
        assert_eq!(back.settings.scale, 0.5);
        assert_eq!(back.location, folder);
        assert!(back.file_name.is_empty(), "the file name follows the sequence");
        assert_eq!(back.open_sections, vec!["audio".to_string()]);
        assert!(back.quick_preset.is_empty(), "a preset not in the library is not brought back");
        assert!(back.quick_path.starts_with(&folder) && back.quick_path.ends_with(std::path::MAIN_SEPARATOR));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_folders_and_unreadable_settings_keep_the_defaults() {
        let remembered = RememberedExport {
            preset: "Gone".into(),
            settings: serde_json::json!({"noSuchField": 1}),
            location: "/no/such/folder/anywhere".into(),
            quick_folder: "/no/such/folder/anywhere".into(),
            ..Default::default()
        };
        let mut ex = ExportUi::default();
        let before = ExportUi::default();
        remembered.apply(&mut ex, |_| false);
        assert_eq!(ex.preset, before.preset);
        assert_eq!(super::super::settings_json(&ex.settings), super::super::settings_json(&before.settings));
        assert!(ex.location.is_empty() && ex.quick_path.is_empty());
    }
}
