//! The manifest's display texts and their translation (plugin API 1.6,
//! [`crate::locale`]).

use std::collections::BTreeMap;

use crate::locale::{
    Catalog, DEFAULT_LOCALE, LOCALIZATION_API, LocaleError, LocaleTag, Messages, clamp,
    locale_file, text_key,
};
use crate::model::{MAX_NAVIGATION_LABEL, Manifest, ParameterKind};

/// Calls `f` with the path, limit and value of every label of `kind`.
fn kind_texts(path: &str, kind: &mut ParameterKind, f: &mut dyn FnMut(&str, usize, &mut String)) {
    match kind {
        ParameterKind::Lines {
            item_label: Some(label),
            ..
        } => f(&format!("{path}.itemLabel"), 32, label),
        ParameterKind::Pairs {
            name_label,
            value_label,
            ..
        } => {
            if let Some(label) = name_label {
                f(&format!("{path}.nameLabel"), 32, label);
            }
            if let Some(label) = value_label {
                f(&format!("{path}.valueLabel"), 32, label);
            }
        }
        _ => {}
    }
}

impl Manifest {
    /// Whether the manifest's texts may name translation keys (`t:…`): it
    /// requires plugin API 1.6. For an older manifest `t:…` is plain text.
    pub fn reads_translation_keys(&self) -> bool {
        self.engines.plugin_api.minimum >= LOCALIZATION_API
    }

    /// `defaultLocale`, `en` when absent.
    pub fn default_locale(&self) -> &str {
        self.default_locale.as_deref().unwrap_or(DEFAULT_LOCALE)
    }

    /// The declared `locales`.
    pub fn declared_locales(&self) -> &[String] {
        self.locales.as_deref().unwrap_or_default()
    }

    /// The package file of every declared locale (`locales/<tag>.json`).
    pub fn locale_files(&self) -> Vec<String> {
        self.declared_locales()
            .iter()
            .map(|tag| locale_file(tag))
            .collect()
    }

    /// Calls `f` with the JSON path, the length limit and the value of
    /// every text the app shows to the user (names, descriptions, labels,
    /// error messages, justifications). Identifiers are never visited.
    pub fn for_each_text_mut(&mut self, f: &mut dyn FnMut(&str, usize, &mut String)) {
        f("name", 48, &mut self.name);
        f("description", 300, &mut self.description);
        f("publisher.name", 64, &mut self.publisher.name);
        for (i, permission) in self.permissions.iter_mut().enumerate() {
            f(
                &format!("permissions[{i}].justification"),
                200,
                permission.justification_mut(),
            );
        }
        for (i, dependency) in self.dependencies.iter_mut().flatten().enumerate() {
            if let Some(text) = &mut dependency.justification {
                f(&format!("dependencies[{i}].justification"), 200, text);
            }
        }
        let contributes = &mut self.contributes;
        for (i, action) in contributes.actions.iter_mut().flatten().enumerate() {
            let path = format!("contributes.actions[{i}]");
            f(&format!("{path}.name"), 48, &mut action.name);
            f(&format!("{path}.description"), 300, &mut action.description);
            f(&format!("{path}.category"), 32, &mut action.category);
            if let Some(text) = action.title.as_mut().and_then(|t| t.default.as_mut()) {
                f(&format!("{path}.title.default"), 32, text);
            }
            for (j, parameter) in action.parameters.iter_mut().enumerate() {
                let path = format!("{path}.parameters[{j}]");
                f(&format!("{path}.label"), 48, &mut parameter.label);
                if let Some(text) = &mut parameter.description {
                    f(&format!("{path}.description"), 300, text);
                }
                kind_texts(&format!("{path}.kind"), &mut parameter.kind, f);
            }
            if let Some(outputs) = action.node.as_mut().and_then(|n| n.outputs.as_mut()) {
                for (k, output) in outputs.iter_mut().enumerate() {
                    f(
                        &format!("{path}.node.outputs[{k}].label"),
                        48,
                        &mut output.label,
                    );
                }
            }
        }
        for (i, variable) in contributes.variables.iter_mut().flatten().enumerate() {
            let path = format!("contributes.variables[{i}]");
            f(&format!("{path}.name"), 48, &mut variable.name);
            if let Some(text) = &mut variable.description {
                f(&format!("{path}.description"), 300, text);
            }
        }
        if let Some(fields) = contributes
            .settings
            .as_mut()
            .and_then(|s| s.fields.as_mut())
        {
            for (i, setting) in fields.iter_mut().enumerate() {
                let path = format!("contributes.settings.fields[{i}]");
                f(&format!("{path}.label"), 48, &mut setting.label);
                if let Some(text) = &mut setting.description {
                    f(&format!("{path}.description"), 300, text);
                }
                kind_texts(&format!("{path}.kind"), &mut setting.kind, f);
            }
        }
        for (i, slot) in contributes.credentials.iter_mut().flatten().enumerate() {
            let path = format!("contributes.credentials[{i}]");
            f(&format!("{path}.label"), 48, &mut slot.label);
            if let Some(text) = &mut slot.description {
                f(&format!("{path}.description"), 300, text);
            }
        }
        for (code, message) in contributes.error_codes.iter_mut().flatten() {
            f(&format!("contributes.errorCodes.{code}"), 200, message);
        }
        if let Some(assets) = &mut contributes.visual_assets {
            for (i, icon) in assets.icons.iter_mut().flatten().enumerate() {
                f(
                    &format!("contributes.visualAssets.icons[{i}].name"),
                    crate::visual::MAX_VISUAL_ASSET_NAME,
                    &mut icon.name,
                );
            }
            for (i, background) in assets.backgrounds.iter_mut().flatten().enumerate() {
                f(
                    &format!("contributes.visualAssets.backgrounds[{i}].name"),
                    crate::visual::MAX_VISUAL_ASSET_NAME,
                    &mut background.name,
                );
            }
        }
        for (i, entry) in contributes.navigation.iter_mut().flatten().enumerate() {
            f(
                &format!("contributes.navigation[{i}].label"),
                MAX_NAVIGATION_LABEL,
                &mut entry.label,
            );
        }
    }

    /// Every translation key the manifest's texts name, with the text's
    /// path. Empty for a manifest below plugin API 1.6.
    pub fn translation_keys(&self) -> Vec<(String, String)> {
        if !self.reads_translation_keys() {
            return Vec::new();
        }
        let mut keys = Vec::new();
        self.clone().for_each_text_mut(&mut |path, _, text| {
            if let Some(key) = text_key(text) {
                keys.push((path.to_owned(), key.to_owned()));
            }
        });
        keys
    }

    /// The manifest with every `t:` text translated into `locale` by
    /// `catalog` (fallback chain and [`crate::locale::humanize`] as
    /// [`Catalog::translate`] says) and cut to its field's limit. Plain
    /// texts and identifiers stay as they are; below plugin API 1.6 the
    /// manifest is returned unchanged. `missing` is called with every key
    /// no locale has.
    #[must_use]
    pub fn localized(
        &self,
        catalog: &Catalog,
        locale: Option<&LocaleTag>,
        missing: &mut dyn FnMut(&str),
    ) -> Manifest {
        let mut manifest = self.clone();
        if !self.reads_translation_keys() {
            return manifest;
        }
        manifest.for_each_text_mut(&mut |_, max, text| {
            if text_key(text).is_some() {
                let mut resolved = catalog.resolve_text(locale, text, missing);
                clamp(&mut resolved, max);
                *text = resolved;
            }
        });
        manifest
    }

    /// The catalog of the manifest's declared locales, each file read with
    /// `read` (package-relative path; the caller hands over only verified
    /// bytes) and checked. A declared locale whose file `read` does not
    /// have is an error, as is a file that does not pass
    /// [`Messages::parse`]. Below plugin API 1.6, or without locales, the
    /// catalog is empty.
    pub fn catalog(
        &self,
        mut read: impl FnMut(&str) -> Option<Vec<u8>>,
    ) -> Result<Catalog, (String, LocaleError)> {
        let default =
            LocaleTag::parse(self.default_locale()).map_err(|e| ("defaultLocale".to_owned(), e))?;
        if !self.reads_translation_keys() {
            return Ok(Catalog::new(default, BTreeMap::new()));
        }
        let mut locales = BTreeMap::new();
        for tag in self.declared_locales() {
            let file = locale_file(tag);
            let parsed = LocaleTag::parse_canonical(tag).map_err(|e| (file.clone(), e))?;
            let bytes = read(&file).ok_or_else(|| {
                (
                    file.clone(),
                    LocaleError::Syntax("the file is missing".into()),
                )
            })?;
            let messages = Messages::parse(&bytes).map_err(|e| (file.clone(), e))?;
            locales.insert(parsed, messages);
        }
        Ok(Catalog::new(default, locales))
    }
}
