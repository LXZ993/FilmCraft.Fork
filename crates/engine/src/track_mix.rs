//! Track mix template: the Audio Track Mixer settings of a sequence (each audio track's volume,
//! pan, mute and effect inserts, and the Mix track's volume and inserts), kept beside the
//! preferences so they carry over to other sequences and projects.
//!
//! - `mixer.saveTrackMixTemplate` keeps the active sequence's track mix as the template.
//! - `mixer.applyTrackMixTemplate` applies it to the active sequence (one undoable step).
//! - `mixer.clearTrackMixTemplate` forgets it.
//!
//! New sequences (File ▸ New ▸ Sequence, a sequence made from a clip or by dropping media on an
//! empty Timeline) start with the template. Tracks are matched by position: the template's first
//! audio track sets A1, the second A2 and so on; tracks the template has no entry for keep their
//! defaults. Automation lanes, sends and outputs are not part of the template.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use filmcraft_project::{EffectInstance, Sequence};

use crate::commands::{CommandSpec, NamedKeys, always, bad, has_seq};
use crate::{Result, Session};

/// The template's file in the data directory (beside the preferences).
pub const TRACK_MIX_FILE: &str = "track-mix.json";

/// The template file is untrusted; bound reading before JSON can allocate from it.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// One audio track's mixer settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TrackMix {
    pub volume_db: f64,
    pub pan: f64,
    pub muted: bool,
    pub effects: Vec<EffectInstance>,
}

/// A sequence's track mix.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TrackMixTemplate {
    /// A1, A2, … in order.
    pub tracks: Vec<TrackMix>,
    pub master_volume_db: f64,
    pub master_effects: Vec<EffectInstance>,
}

impl TrackMixTemplate {
    pub fn of(q: &Sequence) -> Self {
        TrackMixTemplate {
            tracks: q.audio_tracks.iter().map(|t| TrackMix { volume_db: t.volume_db, pan: t.pan, muted: t.muted, effects: t.effects.clone() }).collect(),
            master_volume_db: q.master_volume_db,
            master_effects: q.master_effects.clone(),
        }
    }

    /// Set `q`'s track mix from the template. Effects that this version does not know are left
    /// out. Returns how many audio tracks were set.
    pub fn apply(&self, q: &mut Sequence) -> usize {
        let known = |e: &&EffectInstance| e.def().is_some_and(|d| d.kind == filmcraft_project::EffectKind::Audio && !d.intrinsic);
        let max = filmcraft_project::mixer::MAX_INSERTS;
        let mut n = 0;
        for (t, m) in q.audio_tracks.iter_mut().zip(&self.tracks) {
            t.volume_db = clamp_db(m.volume_db);
            t.pan = if m.pan.is_finite() { m.pan.clamp(-100.0, 100.0) } else { 0.0 };
            t.muted = m.muted;
            t.effects = m.effects.iter().filter(known).take(max).cloned().collect();
            n += 1;
        }
        q.master_volume_db = clamp_db(self.master_volume_db);
        q.master_effects = self.master_effects.iter().filter(known).take(max).cloned().collect();
        n
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

fn clamp_db(v: f64) -> f64 {
    if v.is_finite() { v.clamp(filmcraft_project::mixer::FADER_MIN_DB, filmcraft_project::mixer::FADER_MAX_DB) } else { 0.0 }
}

/// Where the template is kept, or None when preferences are not persisted (tests, the web build).
pub fn template_path(s: &Session) -> Option<std::path::PathBuf> {
    s.prefs_path.as_ref().and_then(|p| p.parent()).map(|d| d.join(TRACK_MIX_FILE))
}

/// The saved template, if there is one.
pub fn template(s: &Session) -> Option<TrackMixTemplate> {
    template_path(s).and_then(|p| TrackMixTemplate::load(&p))
}

/// Give a sequence that was just created the saved template. Call inside the edit that creates it.
pub fn apply_to_new(template: Option<&TrackMixTemplate>, q: &mut Sequence) {
    if let Some(t) = template {
        t.apply(q);
    }
}

fn save_template(s: &mut Session, _p: &Value) -> Result<Value> {
    let q = s.active_sequence().ok_or(crate::EngineError::NoSequence)?;
    let t = TrackMixTemplate::of(q);
    let path = template_path(s).ok_or_else(|| bad("mixer.saveTrackMixTemplate", "preferences are not saved in this session"))?;
    t.save(&path).map_err(|e| bad("mixer.saveTrackMixTemplate", format!("saving {}: {e}", path.display())))?;
    s.toast(format!("Track mix saved as the template for new sequences ({} audio tracks)", t.tracks.len()));
    Ok(json!({"tracks": t.tracks.len(), "path": path.to_string_lossy()}))
}

fn apply_template(s: &mut Session, _p: &Value) -> Result<Value> {
    let t =
        template(s).ok_or_else(|| bad("mixer.applyTrackMixTemplate", "no track mix template saved yet (Audio Track Mixer ▸ Save Track Mix as Template)"))?;
    let n = s.edit_sequence("Apply Track Mix Template", move |q, _, _| Ok(t.apply(q)))?;
    Ok(json!({"tracks": n}))
}

fn clear_template(s: &mut Session, _p: &Value) -> Result<Value> {
    let Some(path) = template_path(s) else { return Ok(json!({"cleared": false})) };
    let cleared = match std::fs::remove_file(&path) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(bad("mixer.clearTrackMixTemplate", format!("removing {}: {e}", path.display()))),
    };
    Ok(json!({"cleared": cleared}))
}

fn has_template(s: &Session) -> std::result::Result<(), String> {
    has_seq(s)?;
    if template_path(s).is_some_and(|p| p.is_file()) { Ok(()) } else { Err("no track mix template saved yet".into()) }
}

pub fn commands() -> Vec<CommandSpec> {
    let spec =
        |id, label, params, enabled, run, journal| CommandSpec { id, label, menu: &[], shortcut: None, params, enabled, run, journal, named: NamedKeys::NONE };
    vec![
        spec("mixer.saveTrackMixTemplate", "Save Track Mix as Template", "{}", has_seq, save_template, false),
        spec("mixer.applyTrackMixTemplate", "Apply Track Mix Template", "{}", has_template, apply_template, true),
        spec("mixer.clearTrackMixTemplate", "Clear Track Mix Template", "{}", always, clear_template, false),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Session {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        s
    }

    fn session_with_prefs() -> (Session, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("fc-track-mix-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = demo();
        s.prefs_path = Some(dir.join("prefs.json"));
        (s, dir)
    }

    #[test]
    fn a_saved_track_mix_carries_over_to_new_sequences_and_applies_to_others() {
        let (mut s, dir) = session_with_prefs();
        s.execute("timeline.setTrack", json!({"track": "A1", "volumeDb": -9.0, "pan": 20.0})).unwrap();
        s.execute("mixer.addInsert", json!({"strip": "A1", "effect": "graphic_eq_30"})).unwrap();
        s.execute("mixer.saveTrackMixTemplate", json!({})).unwrap();
        assert!(dir.join(TRACK_MIX_FILE).is_file());

        // a new sequence starts with it
        s.execute("file.newSequence", json!({"name": "Next"})).unwrap();
        let a1 = &s.active_sequence().unwrap().audio_tracks[0];
        assert_eq!(a1.volume_db, -9.0);
        assert_eq!(a1.pan, 20.0);
        assert_eq!(a1.effects.len(), 1);
        assert_eq!(a1.effects[0].effect, "graphic_eq_30");

        // an existing sequence takes it on request, as one undoable step
        s.execute("timeline.setTrack", json!({"track": "A1", "volumeDb": 0.0, "pan": 0.0})).unwrap();
        s.execute("mixer.removeInsert", json!({"strip": "A1", "slot": 0})).unwrap();
        s.execute("mixer.applyTrackMixTemplate", json!({})).unwrap();
        assert_eq!(s.active_sequence().unwrap().audio_tracks[0].volume_db, -9.0);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active_sequence().unwrap().audio_tracks[0].volume_db, 0.0);

        s.execute("mixer.clearTrackMixTemplate", json!({})).unwrap();
        assert!(!dir.join(TRACK_MIX_FILE).exists());
        assert!(s.execute("mixer.applyTrackMixTemplate", json!({})).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_effects_and_wild_values_are_not_applied() {
        let mut q = demo().active_sequence().unwrap().clone();
        let mut bogus = q.audio_tracks[0].effects.first().cloned().unwrap_or_else(|| filmcraft_project::find_effect("graphic_eq_30").unwrap().instance());
        bogus.effect = "no_such_effect".into();
        let t = TrackMixTemplate {
            tracks: vec![TrackMix { volume_db: f64::NAN, pan: 1e9, muted: true, effects: vec![bogus] }],
            master_volume_db: 1e9,
            master_effects: vec![],
        };
        t.apply(&mut q);
        assert_eq!(q.audio_tracks[0].volume_db, 0.0);
        assert_eq!(q.audio_tracks[0].pan, 100.0);
        assert!(q.audio_tracks[0].muted);
        assert!(q.audio_tracks[0].effects.is_empty());
        assert_eq!(q.master_volume_db, filmcraft_project::mixer::FADER_MAX_DB);
    }
}
