//! Compiling a resolved theme with the Slint interpreter.
//!
//! Every import goes through the file loader, which serves the theme's sources from a virtual
//! folder. Slint cleans `..` out of paths before it asks, so an import that tries to leave the
//! folder arrives as a path outside it and is refused. Built-in files (`std-widgets.slint`)
//! never reach the loader.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use nexus_ui_api::ThemeError;
use nexus_ui_api::contract::{self, ASSETS_FILE, OPTIONS_FILE, SHELL_COMPONENT, TOKENS_FILE, TYPES_FILE};
use slint_interpreter::{Compiler, ComponentDefinition, DiagnosticLevel};

use crate::themes::ResolvedTheme;

/// The virtual folder. Nothing is read from disk under it.
pub const VFS_ROOT: &str = "/launcher-theme";

const ROOT_FILE: &str = "__root__.slint";

/// The main file. It re-exports the globals, so the host can reach them whether or not the
/// theme's own files export them.
const ROOT_SOURCE: &str = r#"
export { Vm, Host } from "types.slint";
export { Tokens } from "tokens.slint";
export { Assets } from "assets.slint";
export { Options } from "options.slint";
export { Shell } from "shell.slint";
"#;

/// Every source the compiler may ask for, by path inside the virtual folder.
pub fn sources(theme: &ResolvedTheme) -> Result<BTreeMap<String, String>, ThemeError> {
    let mut files = theme.slint_sources()?;
    files.insert(TYPES_FILE.into(), contract::TYPES_SLINT.into());
    files.insert(TOKENS_FILE.into(), contract::tokens_slint(&theme.font_paths()?));
    files.insert(ASSETS_FILE.into(), contract::assets_slint(theme.merged.images.keys()));
    files.insert(OPTIONS_FILE.into(), contract::options_slint(&theme.merged.options));
    Ok(files)
}

fn serve(files: &BTreeMap<String, String>, path: &Path) -> std::io::Result<String> {
    let inside = path.strip_prefix(VFS_ROOT).ok().map(|p| p.to_string_lossy().replace('\\', "/"));
    match inside {
        Some(rel) => files
            .get(&rel)
            .cloned()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("no {rel} in this theme"))),
        None => Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{} is outside the theme", path.display()),
        )),
    }
}

pub fn compile(theme: &ResolvedTheme) -> Result<ComponentDefinition, ThemeError> {
    let files = Rc::new(sources(theme)?);
    let mut compiler = Compiler::default();
    compiler.set_include_paths(Vec::new());
    let loader_files = files.clone();
    compiler.set_file_loader(move |path| {
        let result = serve(&loader_files, path);
        Box::pin(async move { Some(result) })
    });
    let main = PathBuf::from(VFS_ROOT).join(ROOT_FILE);
    let result = spin_on::spin_on(compiler.build_from_source(ROOT_SOURCE.to_owned(), main));
    let errors: Vec<String> = result
        .diagnostics()
        .filter(|d| d.level() == DiagnosticLevel::Error)
        .map(|d| {
            let file = d.source_file().map(|p| p.to_string_lossy().replace(VFS_ROOT, "")).unwrap_or_default();
            let (line, column) = d.line_column();
            format!("{file}:{line}:{column}: {}", d.message())
        })
        .collect();
    if !errors.is_empty() {
        return Err(ThemeError::Compile { theme: theme.id().clone(), diagnostics: errors.join("\n") });
    }
    result.component(SHELL_COMPONENT).ok_or_else(|| ThemeError::Compile {
        theme: theme.id().clone(),
        diagnostics: format!("{} must export component {SHELL_COMPONENT}", contract::SHELL_FILE),
    })
}
