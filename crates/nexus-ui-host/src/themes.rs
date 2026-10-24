//! Finding themes, resolving inheritance and checking files. No Slint here.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use nexus_api::{Files, PluginId, PluginKind, RelPath};
use nexus_core::Core;
use nexus_ui_api::contract::{self, RESERVED_FILES, SHELL_FILE};
use nexus_ui_api::{ComponentId, MergedTheme, ScreenId, ThemeError, ThemeManifest, merge_chain};

/// One theme in a chain: its manifest and its files.
#[derive(Clone)]
pub struct ThemeLayer {
    pub manifest: ThemeManifest,
    pub files: Arc<dyn Files>,
}

/// Every installed theme.
#[derive(Clone, Default)]
pub struct ThemeSet {
    themes: BTreeMap<PluginId, ThemeLayer>,
}

impl ThemeSet {
    /// The theme plugins the core loaded. A theme whose plugin fails to start is left out.
    pub async fn from_core(core: &Core) -> Self {
        let kernel = core.kernel();
        let mut themes = BTreeMap::new();
        for id in kernel.of_kind(PluginKind::Theme) {
            let manifest = match kernel.manifest(&id).map(ThemeManifest::from_plugin) {
                Ok(Ok(m)) => m,
                Ok(Err(e)) => {
                    tracing::warn!(plugin_id = %id, error = %e, "theme skipped");
                    continue;
                }
                Err(_) => continue,
            };
            match kernel.theme_files(&id) {
                Ok(files) => {
                    themes.insert(id, ThemeLayer { manifest, files });
                }
                Err(e) => tracing::warn!(plugin_id = %id, error = %e, "theme skipped"),
            }
        }
        Self { themes }
    }

    pub fn insert(&mut self, layer: ThemeLayer) {
        self.themes.insert(layer.manifest.id().clone(), layer);
    }

    pub fn ids(&self) -> impl Iterator<Item = &PluginId> {
        self.themes.keys()
    }

    pub fn contains(&self, id: &PluginId) -> bool {
        self.themes.contains_key(id)
    }

    /// The theme and its parents, the theme first.
    pub fn chain(&self, id: &PluginId) -> Result<Vec<ThemeLayer>, ThemeError> {
        let mut chain: Vec<ThemeLayer> = Vec::new();
        let mut next = Some(id.clone());
        while let Some(current) = next.take() {
            if chain.iter().any(|l| l.manifest.id() == &current) {
                let ids = chain.iter().map(|l| l.manifest.id().clone()).collect();
                return Err(ThemeError::Cycle { theme: id.clone(), chain: ids });
            }
            let layer = self.themes.get(&current).cloned().ok_or_else(|| match chain.last() {
                Some(child) => {
                    ThemeError::MissingParent { theme: child.manifest.id().clone(), parent: current.clone() }
                }
                None => ThemeError::Unknown(current.clone()),
            })?;
            next = layer.manifest.theme.extends.clone();
            chain.push(layer);
        }
        Ok(chain)
    }

    /// Resolves `id` with the player's option values.
    pub fn resolve(&self, id: &PluginId, options: &serde_json::Value) -> Result<ResolvedTheme, ThemeError> {
        let layers = self.chain(id)?;
        let ui_api = nexus_ui_api::ui_api_version();
        for layer in &layers {
            layer.manifest.check_ui_api(&ui_api)?;
        }
        let manifests: Vec<&ThemeManifest> = layers.iter().map(|l| &l.manifest).collect();
        let merged = merge_chain(&manifests, options)?;
        let resolved = ResolvedTheme { merged, layers };
        resolved.check()?;
        Ok(resolved)
    }
}

/// A theme with its chain, ready to compile.
#[derive(Clone)]
pub struct ResolvedTheme {
    pub merged: MergedTheme,
    pub layers: Vec<ThemeLayer>,
}

impl ResolvedTheme {
    pub fn id(&self) -> &PluginId {
        &self.merged.id
    }

    /// The first layer (the theme, then its parents) that has `path`.
    pub fn find(&self, path: &RelPath) -> Option<(usize, Vec<u8>)> {
        self.layers.iter().enumerate().find_map(|(i, l)| l.files.read(path).map(|bytes| (i, bytes)))
    }

    fn file_error(&self, layer: usize, file: &str, message: String) -> ThemeError {
        ThemeError::File { theme: self.layers[layer].manifest.id().clone(), file: file.to_owned(), message }
    }

    /// Every `.slint` file the theme serves, with inheritance applied, and the theme's own
    /// rule checks done.
    pub fn slint_sources(&self) -> Result<BTreeMap<String, String>, ThemeError> {
        let mut sources = BTreeMap::new();
        for (layer, theme) in self.layers.iter().enumerate().rev() {
            for path in theme.files.list() {
                if path.extension() != Some("slint") {
                    continue;
                }
                let bytes = theme.files.read(&path).unwrap_or_default();
                let text = String::from_utf8(bytes)
                    .map_err(|_| self.file_error(layer, path.as_str(), "is not UTF-8".into()))?;
                sources.insert(path.as_str().to_owned(), (layer, text));
            }
        }
        let expected = expected_exports();
        for (path, (layer, text)) in &sources {
            contract::lint(text, expected.get(path.as_str()).copied()).map_err(|m| self.file_error(*layer, path, m))?;
        }
        Ok(sources.into_iter().map(|(path, (_, text))| (path, text)).collect())
    }

    /// Checks that hold before compiling: reserved names, declared screens and components,
    /// a shell somewhere in the chain.
    fn check(&self) -> Result<(), ThemeError> {
        for (layer, theme) in self.layers.iter().enumerate() {
            for reserved in RESERVED_FILES {
                if theme.files.read(&RelPath::new(reserved).expect("valid")).is_some() {
                    return Err(self.file_error(layer, reserved, "is served by the host; remove it".into()));
                }
            }
            let section = &theme.manifest.theme;
            let declared = section.screens.iter().map(|s| s.file()).chain(section.components.iter().map(|c| c.file()));
            for file in declared {
                if theme.files.read(&RelPath::new(file.clone()).expect("valid")).is_none() {
                    return Err(self.file_error(layer, &file, "is declared in theme.toml but missing".into()));
                }
            }
        }
        if self.find(&RelPath::new(SHELL_FILE).expect("valid")).is_none() {
            return Err(self.file_error(0, SHELL_FILE, "no theme in the chain has one".into()));
        }
        Ok(())
    }

    /// Absolute paths of the theme's fonts. Fonts must come from a theme folder on disk.
    pub fn font_paths(&self) -> Result<Vec<String>, ThemeError> {
        self.merged
            .fonts
            .iter()
            .map(|font| {
                let layer = &self.layers[font.layer];
                let error = |message: &str| ThemeError::Asset {
                    theme: layer.manifest.id().clone(),
                    asset: font.path.to_string(),
                    message: message.to_owned(),
                };
                let Some(root) = layer.files.root() else {
                    // A theme built into the binary has no folder. Slint imports fonts by path,
                    // so its fonts are written once to a cache folder.
                    let bytes = layer.files.read(&font.path).ok_or_else(|| error("is missing"))?;
                    return cached_font(layer.manifest.id().as_str(), &font.path, &bytes)
                        .map_err(|e| error(&format!("can't be cached: {e}")));
                };
                let root = root.canonicalize().map_err(|_| error("the theme folder is gone"))?;
                let path = font.path.under(&root).canonicalize().map_err(|_| error("is missing"))?;
                if !path.starts_with(&root) {
                    return Err(error("leaves the theme folder"));
                }
                Ok(without_verbatim_prefix(path).to_string_lossy().into_owned())
            })
            .collect()
    }

    /// Bytes of each declared image.
    pub fn images(&self) -> Result<BTreeMap<String, Vec<u8>>, ThemeError> {
        self.merged
            .images
            .iter()
            .map(|(name, image)| {
                let layer = &self.layers[image.layer];
                let bytes = layer.files.read(&image.path).ok_or_else(|| ThemeError::Asset {
                    theme: layer.manifest.id().clone(),
                    asset: image.path.to_string(),
                    message: "is missing".into(),
                })?;
                Ok((name.clone(), bytes))
            })
            .collect()
    }

    /// The theme's sound files, by name.
    pub fn sounds(&self) -> Result<BTreeMap<String, Vec<u8>>, ThemeError> {
        self.merged
            .sounds
            .iter()
            .map(|(name, sound)| {
                let layer = &self.layers[sound.layer];
                let bytes = layer.files.read(&sound.path).ok_or_else(|| ThemeError::Asset {
                    theme: layer.manifest.id().clone(),
                    asset: sound.path.to_string(),
                    message: "is missing".into(),
                })?;
                Ok((name.clone(), bytes))
            })
            .collect()
    }

    /// Folders on disk in the chain, for dev-mode reload.
    pub fn disk_roots(&self) -> Vec<PathBuf> {
        self.layers.iter().filter_map(|l| l.files.root()).collect()
    }

    /// The newest change time of any file in the chain's folders on disk.
    pub fn stamp(&self) -> Option<SystemTime> {
        self.layers
            .iter()
            .filter_map(|l| Some((l.files.root()?, l.files.list())))
            .flat_map(|(root, files)| files.into_iter().map(move |f| f.under(&root)))
            .filter_map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
            .max()
    }

    /// Screens this theme takes from a parent rather than defining itself.
    pub fn inherited_screens(&self) -> BTreeSet<ScreenId> {
        ScreenId::ALL
            .into_iter()
            .filter(|s| self.layers[0].files.read(&RelPath::new(s.file()).expect("valid")).is_none())
            .collect()
    }
}

fn expected_exports() -> BTreeMap<String, &'static str> {
    ScreenId::ALL
        .into_iter()
        .map(|s| (s.file(), s.component()))
        .chain(ComponentId::ALL.into_iter().map(|c| (c.file(), c.component())))
        .chain([(SHELL_FILE.to_owned(), contract::SHELL_COMPONENT)])
        .collect()
}

/// `path` without the `\\?\` prefix that `canonicalize` puts on Windows paths: Slint can't
/// open a font by such a path. Other systems' paths come back as they are.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    if let Some(rest) = path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
        return match rest.strip_prefix(r"UNC\") {
            Some(share) => PathBuf::from(format!(r"\\{share}")),
            None => PathBuf::from(rest),
        };
    }
    path
}

/// Writes a built-in theme's font to `<temp>/launcher-fonts/<theme>/<path>`, unless the same
/// bytes are there already, and returns that path.
fn cached_font(theme: &str, path: &RelPath, bytes: &[u8]) -> std::io::Result<String> {
    let file = path.under(&std::env::temp_dir().join("launcher-fonts").join(theme));
    if std::fs::read(&file).ok().as_deref() != Some(bytes) {
        // Through a file of its own and a rename, so another process reading it never sees
        // half a font.
        let folder = file.parent().expect("a file has a parent");
        std::fs::create_dir_all(folder)?;
        let partial = folder.join(format!(".{}.{}", std::process::id(), uuid_like()));
        std::fs::write(&partial, bytes)?;
        std::fs::rename(&partial, &file)?;
    }
    Ok(file.to_string_lossy().into_owned())
}

/// A name no other writer in this process uses at the same time.
fn uuid_like() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
