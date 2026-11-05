use crate::{catalog, works};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io::Write, path::Path};
use tempfile::NamedTempFile;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CloseBehavior {
    #[default]
    Ask,
    Background,
    Quit,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub locale: String,
    pub appearance: String,
    pub enabled: bool,
    pub work_id: String,
    pub parameters: Value,
    pub work_parameters: BTreeMap<String, Value>,
    pub application_path: String,
    pub launch_at_login: bool,
    pub close_behavior: CloseBehavior,
    pub market_url: String,
    pub refresh_market_on_startup: bool,
    pub check_updates_on_startup: bool,
    pub ignored_update_version: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            locale: "zh".into(),
            appearance: "dark".into(),
            enabled: true,
            work_id: String::new(),
            parameters: json!({}),
            work_parameters: BTreeMap::new(),
            application_path: String::new(),
            launch_at_login: false,
            close_behavior: CloseBehavior::Ask,
            market_url: catalog::DEFAULT_MARKET_URL.into(),
            refresh_market_on_startup: true,
            check_updates_on_startup: true,
            ignored_update_version: None,
        }
    }
}

impl Settings {
    pub fn migrate_work_ids(&mut self, aliases: &BTreeMap<String, String>) -> bool {
        let before = self.clone();
        for (legacy, current) in aliases {
            if let Some(values) = self.work_parameters.remove(legacy) {
                let target = self
                    .work_parameters
                    .entry(current.clone())
                    .or_insert(json!({}));
                if let (Some(target), Some(values)) = (target.as_object_mut(), values.as_object()) {
                    for (key, value) in values {
                        target.entry(key.clone()).or_insert_with(|| value.clone());
                    }
                }
            }
        }
        if let Some(current) = aliases.get(&self.work_id) {
            self.work_id = current.clone();
            if let Some(mut values) = self.work_parameters.remove(current) {
                if let (Some(target), Some(active)) =
                    (values.as_object_mut(), self.parameters.as_object())
                {
                    target.extend(
                        active
                            .iter()
                            .map(|(key, value)| (key.clone(), value.clone())),
                    );
                    self.parameters = values;
                }
            }
        }
        *self != before
    }

    pub fn select_default_work(&mut self, installed: &[works::Work], preferred: Option<&str>) {
        if !self.work_id.is_empty() || !self.enabled {
            return;
        }
        self.work_id = preferred
            .and_then(|id| installed.iter().find(|work| work.id == id))
            .or_else(|| installed.first())
            .map(|work| work.id.clone())
            .unwrap_or_default();
    }

    pub fn resolve_installed_works(&mut self, installed: &[works::Work], preferred: Option<&str>) {
        self.select_default_work(installed, preferred);
        if !installed.iter().any(|work| work.id == self.work_id) {
            self.reset_work_selection(installed);
        }
        if let Some(work) = installed.iter().find(|work| work.id == self.work_id) {
            self.parameters = works::validate_parameters(work, &self.parameters)
                .or_else(|_| works::validate_parameters(work, &json!({})))
                .unwrap_or_else(|_| json!({}));
        }
        self.work_parameters.retain(|id, values| {
            id != &self.work_id
                && installed
                    .iter()
                    .any(|work| work.id == *id && works::validate_parameters(work, values).is_ok())
        });
    }

    pub fn reset_work_selection(&mut self, installed: &[works::Work]) {
        self.work_id = installed
            .first()
            .map(|work| work.id.clone())
            .unwrap_or_default();
        self.parameters = self
            .work_parameters
            .remove(&self.work_id)
            .unwrap_or(json!({}));
        self.enabled = false;
    }

    pub fn load(dir: &Path) -> Self {
        let Ok(contents) = fs::read(dir.join("settings.json")) else {
            return Self::default();
        };
        serde_json::from_slice::<Self>(&contents)
            .ok()
            .filter(|settings| settings.validate().is_ok())
            .unwrap_or_default()
    }

    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.locale.as_str(), "zh" | "en")
            || !matches!(self.appearance.as_str(), "system" | "light" | "dark")
            || self.work_id.len() > 64
            || self.application_path.len() > 4096
            || self.application_path.contains(['\0', '\r', '\n'])
            || !self.parameters.is_object()
            || self.work_parameters.len() > 512
            || self
                .work_parameters
                .values()
                .any(|values| !values.is_object())
            || self
                .ignored_update_version
                .as_deref()
                .is_some_and(|version| {
                    version.len() > 100 || semver::Version::parse(version).is_err()
                })
        {
            return Err("Invalid settings".into());
        }
        catalog::validate_source(&self.market_url)
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        let mut staged = NamedTempFile::new_in(dir).map_err(|error| error.to_string())?;
        staged
            .write_all(&bytes)
            .and_then(|_| staged.as_file().sync_all())
            .map_err(|error| error.to_string())?;
        staged
            .persist(dir.join("settings.json"))
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn bundled_directory(data_dir: &Path, shared: bool) -> PathBuf {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../startup");
        if !shared {
            return source;
        }
        for entry in fs::read_dir(source).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                works::import_work(data_dir, &path).unwrap();
            }
        }
        data_dir.join("startup")
    }

    fn initialize(data_dir: &Path, packaged: &Path) -> Settings {
        let plan = works::seed_packaged(data_dir, packaged).unwrap();
        let mut settings = Settings::load(data_dir);
        settings.migrate_work_ids(&plan.aliases);
        let installed = works::list(data_dir)
            .unwrap()
            .into_iter()
            .filter(|work| !plan.aliases.contains_key(&work.id))
            .collect::<Vec<_>>();
        settings.resolve_installed_works(&installed, plan.default_id.as_deref());
        settings.save(data_dir).unwrap();
        plan.archive_legacy(data_dir).unwrap();
        settings
    }

    fn assert_fresh_defaults(settings: &Settings) {
        assert!(settings.enabled);
        assert_eq!(settings.work_id, "terminal-prelude");
        assert_eq!(settings.parameters["speed"].as_f64(), Some(1.0));
        assert_eq!(settings.parameters["volume"].as_f64(), Some(0.5));
        assert_eq!(settings.parameters["sound"], json!(true));
    }

    #[test]
    fn actual_bundled_defaults_are_enabled_and_persist_for_both_package_layouts() {
        for shared in [false, true] {
            let data = tempfile::tempdir().unwrap();
            let packaged = bundled_directory(data.path(), shared);
            assert!(!data.path().join("settings.json").exists());
            let first = initialize(data.path(), &packaged);
            assert_fresh_defaults(&first);
            assert_eq!(Settings::load(data.path()), first);
            let restarted = initialize(data.path(), &packaged);
            assert_eq!(restarted, first);
            assert!(!data.path().join("startup-archive").exists());
        }
    }

    #[test]
    fn removing_settings_restores_packaged_defaults_without_reimporting_works() {
        for shared in [false, true] {
            let data = tempfile::tempdir().unwrap();
            let packaged = bundled_directory(data.path(), shared);
            initialize(data.path(), &packaged);
            let count = works::list(data.path()).unwrap().len();
            fs::remove_file(data.path().join("settings.json")).unwrap();
            assert!(data.path().join(".bundled-startup-v1").exists());
            assert_fresh_defaults(&initialize(data.path(), &packaged));
            assert_eq!(works::list(data.path()).unwrap().len(), count);
        }
    }

    #[test]
    fn startup_fills_only_missing_parameters_and_preserves_saved_user_choices() {
        for shared in [false, true] {
            let data = tempfile::tempdir().unwrap();
            let packaged = bundled_directory(data.path(), shared);
            let mut settings = initialize(data.path(), &packaged);
            settings.enabled = false;
            settings.parameters = json!({"speed":1.15849465104432,"volume":0,"sound":false});
            settings.save(data.path()).unwrap();
            let restarted = initialize(data.path(), &packaged);
            assert_eq!(restarted.work_id, settings.work_id);
            assert!(!restarted.enabled);
            assert_eq!(restarted.parameters["speed"], settings.parameters["speed"]);
            assert_eq!(restarted.parameters["volume"], json!(0));
            assert_eq!(restarted.parameters["sound"], json!(false));
            assert!(restarted.parameters["identity"].is_string());
            assert_eq!(initialize(data.path(), &packaged), restarted);
        }
    }
}
