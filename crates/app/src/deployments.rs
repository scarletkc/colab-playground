//! The deployments list: every plugin run started from the app, so it can be started again with
//! the same settings. Kept in `deployments.json` next to the settings file.
//!
//! A record holds what the run form held except secrets: the app never writes a secret to its
//! files, so secret inputs are dropped and asked for again. Of the hardware it keeps the GPU,
//! the high-RAM shape and the minimum GPU memory; the session name and the cost limits come
//! from the current defaults when the record is used again.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use colab_playground_core::{
    backend::{PluginRunSpec, RunTarget},
    plugins::{InputKind, InputValue, Manifest},
    sessions::Accelerator,
};
use serde::{Deserialize, Serialize};

pub const DEPLOYMENTS_FILE: &str = "deployments.json";

/// Records kept, the most recently used first; older ones are dropped.
pub const MAX_DEPLOYMENTS: usize = 30;

/// The hardware a deployment asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hardware {
    pub accelerator: Accelerator,
    pub high_mem: bool,
    pub min_vram_mib: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Deployment {
    pub id: String,
    pub plugin: String,
    /// Input values by id, secrets left out.
    pub inputs: BTreeMap<String, InputValue>,
    /// `None` when it ran on a session that was already running.
    pub hardware: Option<Hardware>,
    #[serde(default)]
    pub drive: bool,
    /// Seconds since the Unix epoch.
    pub last_used: u64,
}

impl Deployment {
    /// The record of `spec`, a run of the plugin described by `manifest`.
    pub fn of(spec: &PluginRunSpec, manifest: &Manifest, now: SystemTime) -> Deployment {
        let inputs = spec
            .inputs
            .iter()
            .filter(|(id, _)| {
                manifest
                    .input(id)
                    .is_some_and(|input| input.kind != InputKind::Secret)
            })
            .map(|(id, value)| (id.clone(), value.clone()))
            .collect();
        let hardware = match &spec.target {
            RunTarget::Provision(p) => Some(Hardware {
                accelerator: p.accelerator,
                high_mem: p.high_mem,
                min_vram_mib: p.min_vram_mib,
            }),
            RunTarget::Existing { .. } => None,
        };
        Deployment {
            id: colab_playground_core::accounts::nonce(),
            plugin: spec.plugin.clone(),
            inputs,
            hardware,
            drive: spec.drive,
            last_used: seconds(now),
        }
    }

    /// Same plugin, inputs, hardware and Drive choice: starting it again adds nothing new.
    fn same_settings(&self, other: &Deployment) -> bool {
        self.plugin == other.plugin
            && self.inputs == other.inputs
            && self.hardware == other.hardware
            && self.drive == other.drive
    }

    /// `template` (a fresh run request for the plugin) with this record's settings. Inputs the
    /// plugin no longer has are left out; the session name, cost limits and secrets stay as the
    /// template has them.
    pub fn apply(&self, template: &mut PluginRunSpec, manifest: &Manifest) {
        for (id, value) in &self.inputs {
            if manifest.input(id).is_some() {
                template.inputs.insert(id.clone(), value.clone());
            }
        }
        if let (Some(hardware), RunTarget::Provision(p)) = (&self.hardware, &mut template.target) {
            p.accelerator = hardware.accelerator;
            p.high_mem = hardware.high_mem;
            p.min_vram_mib = hardware.min_vram_mib;
        }
        template.drive = self.drive
            && manifest
                .needs
                .contains(&colab_playground_core::plugins::Capability::Drive);
    }
}

/// Adds `new` at the front, or moves a record with the same settings there with `new`'s time.
pub fn record(list: &mut Vec<Deployment>, new: Deployment) {
    let entry = match list.iter().position(|d| d.same_settings(&new)) {
        Some(at) => Deployment {
            last_used: new.last_used,
            ..list.remove(at)
        },
        None => new,
    };
    list.insert(0, entry);
    list.truncate(MAX_DEPLOYMENTS);
}

fn seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// `items` under their titles, each group where its first item stood and keeping the order
/// its items had.
pub fn by_title<T>(items: impl IntoIterator<Item = (String, T)>) -> Vec<(String, Vec<T>)> {
    let mut groups: Vec<(String, Vec<T>)> = Vec::new();
    for (title, item) in items {
        match groups.iter_mut().find(|(seen, _)| *seen == title) {
            Some((_, members)) => members.push(item),
            None => groups.push((title, vec![item])),
        }
    }
    groups
}

/// The deployments file in the config directory (`None`: kept in memory only).
#[derive(Clone, Debug)]
pub struct DeploymentStore {
    file: Option<PathBuf>,
}

impl DeploymentStore {
    pub fn new(dir: Option<&Path>) -> Self {
        Self {
            file: dir.map(|dir| dir.join(DEPLOYMENTS_FILE)),
        }
    }

    /// The saved records; a missing or unreadable file reads as none (the list is a
    /// convenience, so a bad file never stops the app).
    pub fn load(&self) -> Vec<Deployment> {
        let Some(file) = &self.file else {
            return Vec::new();
        };
        match fs::read_to_string(file) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
                log::warn!("deployments: ignoring {} ({error})", file.display());
                Vec::new()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                log::warn!("deployments: cannot read {} ({error})", file.display());
                Vec::new()
            }
        }
    }

    pub fn save(&self, list: &[Deployment]) -> std::io::Result<()> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        let text = serde_json::to_string_pretty(list).map_err(std::io::Error::other)?;
        colab_playground_core::paths::write_atomic(file, text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use colab_playground_core::{
        Defaults,
        backend::run_template,
        plugins::{PluginInfo, PluginSource},
    };

    use super::*;

    fn default_run_template(manifest: &Manifest, defaults: &Defaults, name: &str) -> PluginRunSpec {
        let info = PluginInfo {
            manifest: manifest.clone(),
            source: PluginSource::Bundled,
            enabled: true,
        };
        let mut spec = run_template(&info, defaults, &[]);
        if let RunTarget::Provision(p) = &mut spec.target {
            p.name = name.to_owned();
        }
        spec
    }

    fn manifest() -> Manifest {
        Manifest::parse(colab_playground_core::plugins::bundled()[0].0).expect("the bundled one")
    }

    fn spec() -> PluginRunSpec {
        let mut spec = default_run_template(&manifest(), &Defaults::default(), "llm-1");
        spec.inputs
            .insert("repo".into(), InputValue::Text("org/model-GGUF".into()));
        spec.inputs
            .insert("api_key".into(), InputValue::Text("secret-key".into()));
        spec
    }

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn a_record_leaves_secrets_and_the_session_name_out() {
        let record = Deployment::of(&spec(), &manifest(), at(100));
        assert!(!record.inputs.contains_key("api_key"));
        assert_eq!(
            record.inputs.get("repo"),
            Some(&InputValue::Text("org/model-GGUF".into()))
        );
        assert!(record.hardware.is_some());
        assert_eq!(record.last_used, 100);
        let text = serde_json::to_string(&record).unwrap();
        assert!(
            !text.contains("secret-key") && !text.contains("llm-1"),
            "{text}"
        );
    }

    #[test]
    fn the_same_settings_move_to_the_front_and_the_list_is_capped() {
        let mut list = Vec::new();
        record(&mut list, Deployment::of(&spec(), &manifest(), at(1)));
        let mut other = spec();
        other
            .inputs
            .insert("repo".into(), InputValue::Text("org/other-GGUF".into()));
        record(&mut list, Deployment::of(&other, &manifest(), at(2)));
        let first = list[1].id.clone();
        record(&mut list, Deployment::of(&spec(), &manifest(), at(3)));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, first, "the earlier record is reused");
        assert_eq!(list[0].last_used, 3);
        for i in 0..(MAX_DEPLOYMENTS as u64 + 5) {
            let mut each = spec();
            each.inputs
                .insert("ctx".into(), InputValue::Number(i as f64));
            record(&mut list, Deployment::of(&each, &manifest(), at(10 + i)));
        }
        assert_eq!(list.len(), MAX_DEPLOYMENTS);
    }

    #[test]
    fn applying_a_record_keeps_the_new_name_and_drops_unknown_inputs() {
        let mut saved = spec();
        if let RunTarget::Provision(p) = &mut saved.target {
            p.accelerator = Accelerator::L4;
            p.high_mem = true;
        }
        saved.drive = true;
        let mut record = Deployment::of(&saved, &manifest(), at(1));
        record
            .inputs
            .insert("gone".into(), InputValue::Text("x".into()));
        let mut template = default_run_template(&manifest(), &Defaults::default(), "llm-2");
        record.apply(&mut template, &manifest());
        assert!(template.drive);
        assert!(!template.inputs.contains_key("gone"));
        assert!(!template.inputs.contains_key("api_key"));
        match &template.target {
            RunTarget::Provision(p) => {
                assert_eq!(p.name, "llm-2");
                assert_eq!(p.accelerator, Accelerator::L4);
                assert!(p.high_mem);
            }
            RunTarget::Existing { .. } => panic!("a fresh session"),
        }
    }

    #[test]
    fn a_store_round_trips_and_a_bad_file_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = DeploymentStore::new(Some(dir.path()));
        assert!(store.load().is_empty());
        let list = vec![Deployment::of(&spec(), &manifest(), at(5))];
        store.save(&list).unwrap();
        assert_eq!(store.load(), list);
        fs::write(dir.path().join(DEPLOYMENTS_FILE), "{ not json").unwrap();
        assert!(store.load().is_empty());
        assert!(DeploymentStore::new(None).save(&list).is_ok());
    }

    #[test]
    fn deployments_of_one_model_sit_together_in_order() {
        let items = [("b", 1), ("a", 2), ("b", 3), ("c", 4), ("a", 5)]
            .map(|(title, n)| (title.to_owned(), n));
        assert_eq!(
            by_title(items),
            [
                ("b".to_owned(), vec![1, 3]),
                ("a".to_owned(), vec![2, 5]),
                ("c".to_owned(), vec![4]),
            ]
        );
    }
}
