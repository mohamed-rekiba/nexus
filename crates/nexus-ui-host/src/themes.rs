//! Finding themes, applying their options and checking their files. No Slint here.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use nexus_api::{Files, PluginId, PluginKind, RelPath};
use nexus_core::Core;
use nexus_ui_api::contract::{self, RESERVED_FILES, SHELL_FILE};
use nexus_ui_api::{Art, MergedTheme, ScreenId, ThemeError, ThemeManifest};

/// An installed theme: its manifest and its files.
#[derive(Clone)]
pub struct InstalledTheme {
    pub manifest: ThemeManifest,
    pub files: Arc<dyn Files>,
}

/// Every installed theme.
#[derive(Clone, Default)]
pub struct ThemeSet {
    themes: BTreeMap<PluginId, InstalledTheme>,
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
                    themes.insert(id, InstalledTheme { manifest, files });
                }
                Err(e) => tracing::warn!(plugin_id = %id, error = %e, "theme skipped"),
            }
        }
        Self { themes }
    }

    pub fn insert(&mut self, theme: InstalledTheme) {
        self.themes.insert(theme.manifest.id().clone(), theme);
    }

    pub fn ids(&self) -> impl Iterator<Item = &PluginId> {
        self.themes.keys()
    }

    pub fn contains(&self, id: &PluginId) -> bool {
        self.themes.contains_key(id)
    }

    /// The one theme that says `fallback = true`: the app starts with it, falls back to it, and
    /// uses it in safe mode. None or more than one is an error that names them.
    pub fn fallback(&self) -> Result<PluginId, ThemeError> {
        let marked: Vec<&PluginId> =
            self.themes.iter().filter(|(_, l)| l.manifest.theme.fallback).map(|(id, _)| id).collect();
        match marked.as_slice() {
            [one] => Ok((*one).clone()),
            [] => Err(ThemeError::Fallback("no installed theme says `fallback = true`".into())),
            many => Err(ThemeError::Fallback(format!(
                "only one theme may say `fallback = true`; these do: {}",
                many.iter().map(|id| id.as_str()).collect::<Vec<_>>().join(", ")
            ))),
        }
    }

    /// Resolves `id` with the player's option values.
    pub fn resolve(&self, id: &PluginId, options: &serde_json::Value) -> Result<ResolvedTheme, ThemeError> {
        let theme = self.themes.get(id).cloned().ok_or_else(|| ThemeError::Unknown(id.clone()))?;
        theme.manifest.check_ui_api(&nexus_ui_api::ui_api_version())?;
        let merged = nexus_ui_api::merge(&theme.manifest, options)?;
        let resolved = ResolvedTheme { merged, theme };
        resolved.check()?;
        Ok(resolved)
    }
}

/// A theme with its options applied, ready to compile.
#[derive(Clone)]
pub struct ResolvedTheme {
    pub merged: MergedTheme,
    pub theme: InstalledTheme,
}

impl ResolvedTheme {
    pub fn id(&self) -> &PluginId {
        &self.merged.id
    }

    fn file_error(&self, file: &str, message: String) -> ThemeError {
        ThemeError::File { theme: self.id().clone(), file: file.to_owned(), message }
    }

    fn asset_error(&self, asset: &RelPath, message: &str) -> ThemeError {
        ThemeError::Asset { theme: self.id().clone(), asset: asset.to_string(), message: message.to_owned() }
    }

    /// The bytes of the theme's file at `path`, or an asset error naming it.
    fn asset(&self, path: &RelPath) -> Result<Vec<u8>, ThemeError> {
        self.theme.files.read(path).ok_or_else(|| self.asset_error(path, "is missing"))
    }

    /// Every `.slint` file the theme serves: the kit, then the theme's own files over it, with
    /// the theme's own rule checks done.
    pub fn slint_sources(&self) -> Result<BTreeMap<String, String>, ThemeError> {
        // The kit first, so a theme's own `kit/<name>.slint` replaces it.
        let mut files: BTreeMap<String, String> = contract::KIT
            .iter()
            .map(|(name, text)| (format!("{}/{name}", contract::KIT_DIR), (*text).to_owned()))
            .collect();
        let expected = expected_exports();
        for path in self.theme.files.list() {
            if path.extension() != Some("slint") {
                continue;
            }
            let bytes = self.theme.files.read(&path).unwrap_or_default();
            let text = String::from_utf8(bytes).map_err(|_| self.file_error(path.as_str(), "is not UTF-8".into()))?;
            contract::lint(&text, expected.get(path.as_str()).copied())
                .map_err(|m| self.file_error(path.as_str(), m))?;
            files.insert(path.as_str().to_owned(), text);
        }
        Ok(files)
    }

    /// Checks that hold before compiling: no file the host serves, a shell and every screen.
    fn check(&self) -> Result<(), ThemeError> {
        let has = |file: &str| self.theme.files.read(&RelPath::new(file).expect("valid")).is_some();
        if let Some(reserved) = RESERVED_FILES.into_iter().find(|f| has(f)) {
            return Err(self.file_error(reserved, "is served by the host; remove it".into()));
        }
        let required = std::iter::once(SHELL_FILE.to_owned()).chain(ScreenId::ALL.into_iter().map(|s| s.file()));
        for file in required {
            if !has(&file) {
                return Err(self.file_error(&file, "is missing: a theme ships its shell and every screen".into()));
            }
        }
        Ok(())
    }

    /// Absolute paths of the theme's fonts. Fonts must come from a theme folder on disk.
    pub fn font_paths(&self) -> Result<Vec<String>, ThemeError> {
        self.merged
            .fonts
            .iter()
            .map(|font| {
                let error = |message: &str| self.asset_error(font, message);
                let Some(root) = self.theme.files.root() else {
                    // A theme built into the binary has no folder. Slint imports fonts by path,
                    // so its fonts are written once to a cache folder.
                    let bytes = self.asset(font)?;
                    return cached_font(self.id().as_str(), font, &bytes)
                        .map_err(|e| error(&format!("can't be cached: {e}")));
                };
                let root = root.canonicalize().map_err(|_| error("the theme folder is gone"))?;
                let path = font.under(&root).canonicalize().map_err(|_| error("is missing"))?;
                if !path.starts_with(&root) {
                    return Err(error("leaves the theme folder"));
                }
                Ok(without_verbatim_prefix(path).to_string_lossy().into_owned())
            })
            .collect()
    }

    /// Bytes of each declared image.
    pub fn images(&self) -> Result<BTreeMap<String, Vec<u8>>, ThemeError> {
        self.merged.images.iter().map(|(name, path)| Ok((name.clone(), self.asset(path)?))).collect()
    }

    /// Bytes of each system's icon and cover the theme sets, by system id.
    pub fn system_art(&self) -> Result<BTreeMap<String, Art<Vec<u8>>>, ThemeError> {
        self.merged
            .system_art
            .iter()
            .map(|(system, art)| Ok((system.clone(), art.clone().try_map(|_, path| self.asset(&path))?)))
            .collect()
    }

    /// The theme's sound files, by name.
    pub fn sounds(&self) -> Result<BTreeMap<String, Vec<u8>>, ThemeError> {
        self.merged.sounds.iter().map(|(name, path)| Ok((name.clone(), self.asset(path)?))).collect()
    }

    /// The theme's folder on disk, for dev-mode reload.
    pub fn disk_roots(&self) -> Vec<PathBuf> {
        self.theme.files.root().into_iter().collect()
    }

    /// The newest change time of any file in the theme's folder on disk.
    pub fn stamp(&self) -> Option<SystemTime> {
        let root = self.theme.files.root()?;
        self.theme
            .files
            .list()
            .into_iter()
            .filter_map(|f| std::fs::metadata(f.under(&root)).and_then(|m| m.modified()).ok())
            .max()
    }
}

fn expected_exports() -> BTreeMap<String, &'static str> {
    ScreenId::ALL
        .into_iter()
        .map(|s| (s.file(), s.component()))
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
