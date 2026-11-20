//! Plugin translations (plugin API 1.6, `docs/marketplace/sdk/localization.md`).
//!
//! Every installed version's `locales/<tag>.json` are read from its
//! install directory and checked against the version's integrity record
//! (the digests recorded at install, from the registry's pins or the
//! bundled pins) before they are parsed: a file that differs is never
//! used, and nothing is ever fetched. The parsed catalogs are cached per
//! plugin, version and manifest digest, so an update or a rollback (another
//! version) and an uninstall ([`Locales::forget`]) drop the old ones, and
//! no plugin's translations outlive it.
//!
//! [`Locales::localized`] gives a record whose manifests show their `t:`
//! texts in the app's language; [`Translator`] is one plugin's catalog for
//! its own invocations (`locale.get`, `locale.translate`, dialog texts):
//! it holds that plugin's catalog only, so a plugin can never resolve
//! another plugin's keys or DeckPilot's.
//!
//! A key that no locale has is logged once per plugin and key (debug), and
//! shown as a label made from its last segment; it never fails a load.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use deckpilot_ipc::plugin_dialogs::DialogSpec;
use deckpilot_plugin_abi::locale::{Translate, TranslateArg, Translation};
use deckpilot_plugin_manifest::locale::{
    Arg, Catalog, FALLBACK_LOCALE, LocaleTag, MAX_VALUE_LEN, clamp, text_key,
};
use deckpilot_plugin_manifest::{Manifest, Sha256Digest};
use deckpilot_plugin_store::{PluginRecord, VersionRecord};
use sha2::{Digest as _, Sha256};
use tracing::{debug, warn};

/// What a cached catalog belongs to: a plugin's version and manifest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CacheKey {
    plugin: String,
    version: String,
    manifest: Sha256Digest,
}

#[derive(Debug, Default)]
struct State {
    /// The app's language; `None` until an app said (English).
    locale: Option<LocaleTag>,
    catalogs: BTreeMap<CacheKey, Arc<Catalog>>,
    /// Keys reported missing, per plugin (each logged once).
    reported: BTreeSet<(String, String)>,
}

/// The app's language and every installed plugin's verified catalogs.
#[derive(Debug, Clone, Default)]
pub(super) struct Locales(Arc<Mutex<State>>);

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn sha256(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}

/// The bytes of `path` in `dir`, only when they are what the version's
/// integrity record says.
fn verified(dir: &Path, version: &VersionRecord, path: &str) -> Option<Vec<u8>> {
    let bytes = std::fs::read(dir.join(path)).ok()?;
    match version.integrity.verify_file(path, &sha256(&bytes)) {
        Ok(()) => Some(bytes),
        Err(error) => {
            warn!(%error, "a plugin's locale file does not match its package; it is not used");
            None
        }
    }
}

impl Locales {
    /// Sets the app's language; whether it changed. A tag that does not
    /// parse counts as English.
    pub fn set_locale(&self, tag: &str) -> bool {
        let parsed = LocaleTag::parse(tag).ok();
        let mut state = lock(&self.0);
        if state.locale == parsed {
            return false;
        }
        state.locale = parsed;
        true
    }

    /// The app's language (`en` until an app said).
    pub fn tag(&self) -> String {
        lock(&self.0)
            .locale
            .as_ref()
            .map_or_else(|| FALLBACK_LOCALE.to_owned(), |t| t.as_str().to_owned())
    }

    fn locale(&self) -> Option<LocaleTag> {
        lock(&self.0).locale.clone()
    }

    /// The verified catalog of `version` of `plugin` installed in `dir`
    /// (cached). A version without locales, or whose locale files cannot be
    /// read or are not the installed ones, has an empty catalog: its keys
    /// fall back, nothing fails.
    pub fn catalog(&self, plugin: &str, version: &VersionRecord, dir: &Path) -> Arc<Catalog> {
        let key = CacheKey {
            plugin: plugin.to_owned(),
            version: version.version.to_string(),
            manifest: version.integrity.manifest_sha256,
        };
        if let Some(catalog) = lock(&self.0).catalogs.get(&key) {
            return catalog.clone();
        }
        let catalog = match version
            .manifest
            .catalog(|path| verified(dir, version, path))
        {
            Ok(catalog) => catalog,
            Err((file, error)) => {
                warn!(plugin, %file, %error, "a plugin's translations cannot be used");
                Catalog::default()
            }
        };
        let catalog = Arc::new(catalog);
        let mut state = lock(&self.0);
        // Only one version of a plugin is cached per manifest: another
        // version (an update, a rollback) replaces the older entries.
        state
            .catalogs
            .retain(|k, _| k.plugin != plugin || k.version == key.version);
        state.catalogs.insert(key, catalog.clone());
        catalog
    }

    /// Drops everything cached of `plugin` (uninstalled, updated, rolled
    /// back): its next use reads its verified files again.
    pub fn forget(&self, plugin: &str) {
        let mut state = lock(&self.0);
        state.catalogs.retain(|k, _| k.plugin != plugin);
        state.reported.retain(|(p, _)| p != plugin);
    }

    /// Keeps the cache to the plugins in `installed`.
    pub fn retain(&self, installed: &BTreeSet<String>) {
        let mut state = lock(&self.0);
        state.catalogs.retain(|k, _| installed.contains(&k.plugin));
        state.reported.retain(|(p, _)| installed.contains(p));
    }

    /// Whether a catalog of `plugin` is cached (tests).
    #[cfg(test)]
    pub fn is_cached(&self, plugin: &str) -> bool {
        lock(&self.0).catalogs.keys().any(|k| k.plugin == plugin)
    }

    fn report(&self, plugin: &str, key: &str) {
        let mut state = lock(&self.0);
        if state.reported.insert((plugin.to_owned(), key.to_owned())) {
            debug!(
                plugin,
                key, "a plugin's translation key is in none of its locales"
            );
        }
    }

    /// `manifest` of `version` with its `t:` texts in the app's language.
    pub fn manifest(&self, plugin: &str, version: &VersionRecord, dir: &Path) -> Manifest {
        if !version.manifest.reads_translation_keys() {
            return version.manifest.clone();
        }
        let catalog = self.catalog(plugin, version, dir);
        let locale = self.locale();
        version
            .manifest
            .localized(&catalog, locale.as_ref(), &mut |key| {
                self.report(plugin, key);
            })
    }

    /// `record` with every version's manifest in the app's language (for
    /// what the app shows; identifiers are unchanged). `resolve` gives a
    /// version's install directory.
    pub fn localized(
        &self,
        record: &PluginRecord,
        resolve: &dyn Fn(&str) -> std::path::PathBuf,
    ) -> PluginRecord {
        let mut shown = record.clone();
        for version in &mut shown.versions {
            if version.manifest.reads_translation_keys() {
                let dir = resolve(&version.install_dir);
                version.manifest = self.manifest(record.id.as_str(), version, &dir);
            }
        }
        shown
    }

    /// Every text the action `action` of `version` is named in, in any of
    /// its locales and in the app's language now: a key title showing one
    /// of them is the action's default name, which the agent redraws in
    /// another language. Empty for a plain name (it never changes).
    pub fn action_names(
        &self,
        plugin: &str,
        version: &VersionRecord,
        dir: &Path,
        action: &str,
    ) -> BTreeSet<String> {
        let Some(declared) = version
            .manifest
            .contributes
            .actions()
            .iter()
            .find(|a| a.id == action)
        else {
            return BTreeSet::new();
        };
        let Some(key) =
            text_key(&declared.name).filter(|_| version.manifest.reads_translation_keys())
        else {
            return BTreeSet::new();
        };
        let catalog = self.catalog(plugin, version, dir);
        let mut names: BTreeSet<String> = catalog
            .all_texts(key)
            .into_iter()
            .map(|text| {
                let mut text = text.to_owned();
                clamp(&mut text, 48);
                text
            })
            .collect();
        names.insert(deckpilot_plugin_manifest::locale::humanize(key));
        names
    }

    /// The translator of one plugin's invocations.
    pub fn translator(&self, plugin: &str, version: &VersionRecord, dir: &Path) -> Translator {
        Translator {
            plugin: plugin.to_owned(),
            catalog: self.catalog(plugin, version, dir),
            locale: self.locale(),
            locales: self.clone(),
            reads_keys: version.manifest.reads_translation_keys(),
        }
    }
}

/// One plugin's translations, for its own invocations only.
#[derive(Debug, Clone)]
pub(super) struct Translator {
    plugin: String,
    catalog: Arc<Catalog>,
    locale: Option<LocaleTag>,
    locales: Locales,
    /// Whether the plugin's manifest is of plugin API 1.6 or later: only
    /// then is a `t:` dialog text a key (before, it is the text itself).
    reads_keys: bool,
}

fn arg(value: &TranslateArg) -> Arg {
    match value {
        TranslateArg::Text(text) => Arg::Text(text.clone()),
        TranslateArg::Number(n) => n
            .as_i64()
            .map_or_else(|| Arg::Float(n.as_f64().unwrap_or_default()), Arg::Int),
    }
}

impl Translator {
    /// A translator with no translations (a plugin without locales, and
    /// tests).
    pub fn empty(plugin: &str) -> Self {
        Self {
            plugin: plugin.to_owned(),
            catalog: Arc::new(Catalog::default()),
            locale: None,
            locales: Locales::default(),
            reads_keys: false,
        }
    }

    /// The plugin it translates for.
    pub fn plugin(&self) -> &str {
        &self.plugin
    }

    /// The app's language as the plugin sees it (`locale.get`).
    pub fn tag(&self) -> String {
        self.locale
            .as_ref()
            .map_or_else(|| FALLBACK_LOCALE.to_owned(), |t| t.as_str().to_owned())
    }

    /// `locale.translate`: the request was checked by the ABI (key form,
    /// parameters); the parameters are checked again by the catalog's
    /// rules, and the text is cut to [`MAX_VALUE_LEN`].
    pub fn translate(&self, request: &Translate) -> Result<Translation, String> {
        let params: Vec<(String, Arg)> = request
            .params
            .iter()
            .map(|(name, value)| (name.clone(), arg(value)))
            .collect();
        deckpilot_plugin_manifest::locale::check_params(&params).map_err(|e| e.to_string())?;
        let resolved = self
            .catalog
            .translate(self.locale.as_ref(), &request.key, &params, None);
        if resolved.is_missing() {
            self.locales.report(&self.plugin, &request.key);
        }
        let mut text = resolved.text;
        clamp(&mut text, MAX_VALUE_LEN);
        Ok(Translation {
            missing: resolved.source == deckpilot_plugin_manifest::locale::Source::Fallback,
            text,
        })
    }

    /// `text` in the app's language when it is a `t:` key of the plugin's
    /// own, cut to `max` characters; a plain text as it is.
    pub fn text(&self, text: &str, max: usize) -> String {
        if !self.reads_keys || text_key(text).is_none() {
            return text.to_owned();
        }
        let mut resolved = self
            .catalog
            .resolve_text(self.locale.as_ref(), text, &mut |key| {
                self.locales.report(&self.plugin, key);
            });
        clamp(&mut resolved, max);
        resolved
    }

    /// A dialog's texts (plugin API 1.6): title, message, button, field
    /// and option labels and descriptions given as `t:` keys are resolved
    /// before the app draws them, within the dialog's limits. Ids, values
    /// and defaults are never touched.
    pub fn dialog(&self, spec: &mut DialogSpec) {
        use deckpilot_plugin_abi::dialog::{
            MAX_BUTTON_LABEL, MAX_DIALOG_MESSAGE, MAX_DIALOG_TITLE, MAX_FIELD_DESCRIPTION,
            MAX_FIELD_LABEL, MAX_OPTION_LABEL,
        };
        let each = |text: &mut String, max: usize| *text = self.text(text, max);
        each(&mut spec.title, MAX_DIALOG_TITLE);
        if let Some(message) = &mut spec.message {
            each(message, MAX_DIALOG_MESSAGE);
        }
        for label in [&mut spec.confirm_label, &mut spec.cancel_label]
            .into_iter()
            .flatten()
        {
            each(label, MAX_BUTTON_LABEL);
        }
        for field in &mut spec.fields {
            each(&mut field.label, MAX_FIELD_LABEL);
            if let Some(description) = &mut field.description {
                each(description, MAX_FIELD_DESCRIPTION);
            }
            for option in &mut field.options {
                each(&mut option.label, MAX_OPTION_LABEL);
            }
        }
        for option in &mut spec.options {
            each(&mut option.label, MAX_OPTION_LABEL);
            if let Some(description) = &mut option.description {
                each(description, MAX_FIELD_DESCRIPTION);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckpilot_plugin_manifest::locale::Messages;

    fn translator(locale: &str) -> Translator {
        let mut locales = BTreeMap::new();
        locales.insert(
            LocaleTag::parse("en").unwrap(),
            Messages::from_pairs([
                ("dialog.title", "Delete {{name}}?"),
                ("repos_one", "{{count}} repository"),
                ("repos_other", "{{count}} repositories"),
            ])
            .unwrap(),
        );
        locales.insert(
            LocaleTag::parse("pl").unwrap(),
            Messages::from_pairs([
                ("dialog.title", "Usunąć?"),
                ("dialog.ok", "Usuń"),
                ("repos_one", "{{count}} repozytorium"),
                ("repos_few", "{{count}} repozytoria"),
                ("repos_many", "{{count}} repozytoriów"),
            ])
            .unwrap(),
        );
        Translator {
            plugin: "acme.repo".into(),
            catalog: Arc::new(Catalog::new(LocaleTag::parse("en").unwrap(), locales)),
            locale: LocaleTag::parse(locale).ok(),
            locales: Locales::default(),
            reads_keys: true,
        }
    }

    fn request(key: &str, params: &[(&str, TranslateArg)]) -> Translate {
        Translate {
            key: key.into(),
            params: params
                .iter()
                .map(|(n, v)| ((*n).to_owned(), v.clone()))
                .collect(),
        }
    }

    #[test]
    fn a_plugin_translates_its_own_keys_with_plurals() {
        let pl = translator("pl-PL");
        assert_eq!(pl.tag(), "pl-PL");
        let count = |n: i64| {
            pl.translate(&request("repos", &[("count", TranslateArg::from(n))]))
                .unwrap()
                .text
        };
        assert_eq!(count(1), "1 repozytorium");
        assert_eq!(count(3), "3 repozytoria");
        assert_eq!(count(5), "5 repozytoriów");
        let missing = pl.translate(&request("core.face.back", &[])).unwrap();
        assert!(missing.missing);
        assert_eq!(missing.text, "Back");
        let en = translator("de");
        assert_eq!(
            en.translate(&request("repos", &[("count", TranslateArg::from(2))]))
                .unwrap()
                .text,
            "2 repositories"
        );
        // Interpolated values stay text.
        let named = en
            .translate(&request(
                "dialog.title",
                &[("name", TranslateArg::from("<b>x</b>"))],
            ))
            .unwrap();
        assert_eq!(named.text, "Delete <b>x</b>?");
        assert!(
            en.translate(&request(
                "dialog.title",
                &[("name", TranslateArg::from("y".repeat(300)))]
            ))
            .is_err()
        );
    }

    /// A version of `acme.repo` with English and Polish translations,
    /// installed (written) in `dir` with the integrity record of its
    /// package.
    fn installed(dir: &Path, version: &str, polish_name: &str) -> VersionRecord {
        let mut manifest: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../crates/deckpilot-plugin-package/tests/fixtures/example-manifest.json"
        ))
        .unwrap();
        manifest["id"] = "acme.repo".into();
        manifest["publisher"]["id"] = "acme".into();
        manifest["version"] = version.into();
        manifest["engines"]["pluginApi"] = "^1.6".into();
        manifest["name"] = "t:plugin.name".into();
        manifest["contributes"]["actions"][0]["name"] = "t:actions.count".into();
        manifest["locales"] = serde_json::json!(["en", "pl"]);
        let pl = format!(
            r#"{{"plugin": {{"name": "{polish_name}"}}, "actions": {{"count": "Policz"}}}}"#
        );
        let files: Vec<(&str, Vec<u8>)> = vec![
            ("manifest.json", serde_json::to_vec(&manifest).unwrap()),
            (
                "plugin.wasm",
                deckpilot_plugin_package::wasm::minimal_abi_module_with_note(version),
            ),
            ("LICENSE", b"MIT\n".to_vec()),
            ("README.md", b"# Repo\n".to_vec()),
            ("assets/icon-128.png", vec![1; 64]),
            ("assets/icon-32.png", vec![2; 32]),
            ("assets/keys/pr.png", vec![3; 32]),
            (
                "locales/en.json",
                br#"{"plugin": {"name": "Repositories"}, "actions": {"count": "Count"}}"#.to_vec(),
            ),
            ("locales/pl.json", pl.into_bytes()),
        ];
        let mut builder = deckpilot_plugin_package::PackageBuilder::new();
        for (path, bytes) in &files {
            builder.add(path, bytes.clone()).unwrap();
            let target = dir.join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, bytes).unwrap();
        }
        let built = builder.build().unwrap();
        VersionRecord {
            version: built.manifest.version.clone(),
            role: deckpilot_plugin_store::VersionRole::Active,
            install_dir: format!("acme.repo/{version}"),
            integrity: built.integrity,
            manifest: built.manifest,
            installed_at: 0,
            activated_at: None,
        }
    }

    #[test]
    fn verified_catalogs_are_cached_per_version_and_dropped() {
        let temp = tempfile::tempdir().unwrap();
        let v1_dir = temp.path().join("1.0.0");
        let v1 = installed(&v1_dir, "1.0.0", "Repozytoria");
        let locales = Locales::default();
        assert_eq!(locales.tag(), "en");
        assert!(locales.set_locale("pl"));
        assert!(!locales.set_locale("pl"), "unchanged");
        let shown = locales.manifest("acme.repo", &v1, &v1_dir);
        assert_eq!(shown.name, "Repozytoria");
        assert_eq!(shown.contributes.actions()[0].name, "Policz");
        assert_eq!(shown.id, v1.manifest.id);
        assert!(locales.is_cached("acme.repo"));
        let names = locales.action_names(
            "acme.repo",
            &v1,
            &v1_dir,
            &v1.manifest.contributes.actions()[0].id,
        );
        assert!(
            names.contains("Count") && names.contains("Policz"),
            "{names:?}"
        );

        // An update: the new version's own translations, not the old ones.
        let v2_dir = temp.path().join("1.1.0");
        let v2 = installed(&v2_dir, "1.1.0", "Repozytoria 2");
        assert_eq!(
            locales.manifest("acme.repo", &v2, &v2_dir).name,
            "Repozytoria 2"
        );
        // Uninstalled: nothing of it is kept.
        locales.forget("acme.repo");
        assert!(!locales.is_cached("acme.repo"));

        // A locale file changed on disk after install is never used: the
        // keys fall back, nothing fails.
        let tampered_dir = temp.path().join("tampered");
        let tampered = installed(&tampered_dir, "1.0.0", "Repozytoria");
        std::fs::write(
            tampered_dir.join("locales/pl.json"),
            br#"{"plugin": {"name": "Podmienione"}}"#,
        )
        .unwrap();
        let shown = locales.manifest("acme.repo", &tampered, &tampered_dir);
        assert_eq!(shown.name, "Name", "a label from the key");
        // English without an app's language; a plugin's translator holds
        // its own catalog only.
        let fresh = Locales::default();
        assert_eq!(
            fresh.manifest("acme.repo", &v1, &v1_dir).name,
            "Repositories"
        );
        let translator = fresh.translator("acme.repo", &v1, &v1_dir);
        assert_eq!(translator.plugin(), "acme.repo");
        assert_eq!(translator.tag(), "en");
        let other = translator.translate(&request("face.back", &[])).unwrap();
        assert!(other.missing, "DeckPilot's keys are not the plugin's");
    }

    #[test]
    fn dialog_texts_are_resolved_before_the_app_draws_them() {
        let spec: DialogSpec = serde_json::from_value(serde_json::json!({
            "type": "confirm", "title": "t:dialog.title", "confirmLabel": "t:dialog.ok",
            "message": "Plain text stays", "timeoutSeconds": 30,
        }))
        .unwrap();
        let mut pl = spec.clone();
        translator("pl").dialog(&mut pl);
        assert_eq!(pl.title, "Usunąć?");
        assert_eq!(pl.confirm_label.as_deref(), Some("Usuń"));
        assert_eq!(pl.message.as_deref(), Some("Plain text stays"));
        let mut en = spec;
        translator("en").dialog(&mut en);
        // No English button text: the label from the key.
        assert_eq!(en.confirm_label.as_deref(), Some("Ok"));
        assert_eq!(en.title, "Delete {{name}}?");
        // A plugin before 1.6: `t:` is the text itself.
        let mut old: DialogSpec = serde_json::from_value(serde_json::json!({
            "type": "alert", "title": "t:dialog.title", "timeoutSeconds": 30,
        }))
        .unwrap();
        Translator::empty("acme.repo").dialog(&mut old);
        assert_eq!(old.title, "t:dialog.title");
    }
}
