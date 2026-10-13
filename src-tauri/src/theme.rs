use std::collections::BTreeMap;
use std::fmt::Display;
use std::fs::{self, File};
use std::io::{self, Read, Seek};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use typst::syntax::package::PackageVersion;
use typst::text::{FontInfo, FontStyle};
use zip::ZipArchive;

use crate::compile::Compiler;

const MAX_ARCHIVE_BYTES: u64 = 50 * 1024 * 1024;
const MAX_UNPACKED_BYTES: u64 = 200 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_ENTRIES: usize = 2000;

const TOKENS: [&str; 32] = [
    "--radius",
    "--background",
    "--foreground",
    "--card",
    "--card-foreground",
    "--popover",
    "--popover-foreground",
    "--primary",
    "--primary-foreground",
    "--secondary",
    "--secondary-foreground",
    "--muted",
    "--muted-foreground",
    "--accent",
    "--accent-foreground",
    "--destructive",
    "--border",
    "--input",
    "--ring",
    "--chart-1",
    "--chart-2",
    "--chart-3",
    "--chart-4",
    "--chart-5",
    "--sidebar",
    "--sidebar-foreground",
    "--sidebar-primary",
    "--sidebar-primary-foreground",
    "--sidebar-accent",
    "--sidebar-accent-foreground",
    "--sidebar-border",
    "--sidebar-ring",
];

const COLOUR_FUNCTIONS: [&str; 6] = ["rgb(", "rgba(", "hsl(", "hsla(", "oklch(", "oklab("];
const IMAGES: [&str; 4] = ["svg", "png", "jpg", "jpeg"];
const FONTS: [&str; 2] = ["ttf", "otf"];

type Tokens = BTreeMap<String, String>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Document,
    Slides,
    Poster,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Template {
    label: String,
    kind: Kind,
    snippet: String,
}

#[derive(Debug, Deserialize)]
struct UiFont {
    family: String,
    files: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ui {
    logo: Option<String>,
    logo_dark: Option<String>,
    font: Option<UiFont>,
    #[serde(default)]
    light: Tokens,
    #[serde(default)]
    dark: Tokens,
}

#[derive(Debug, Default, Deserialize)]
struct Typst {
    #[serde(default)]
    templates: Vec<Template>,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    format: u32,
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    ui: Ui,
    #[serde(default)]
    typst: Typst,
}

#[derive(Debug, Serialize, PartialEq)]
struct FontFile {
    path: String,
    weight: u16,
    italic: bool,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    id: String,
    name: String,
    version: String,
    logo: Option<String>,
    logo_dark: Option<String>,
    font_family: Option<String>,
    fonts: Vec<FontFile>,
    light: Tokens,
    dark: Tokens,
    templates: Vec<Template>,
}

fn broken(error: impl Display) -> String {
    format!("This theme file could not be read: {error}")
}

fn failed(action: &str, error: impl Display) -> String {
    format!("Could not {action} the theme: {error}")
}

pub fn valid_id(id: &str) -> bool {
    (1..=40).contains(&id.len())
        && id.starts_with(|first: char| first.is_ascii_lowercase())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_label(text: &str, extra: &str) -> bool {
    (1..=60).contains(&text.chars().count())
        && text.trim() == text
        && text.chars().all(|c| c.is_alphanumeric() || extra.contains(c))
}

fn is_number(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_digit())
        && text.chars().all(|c| c.is_ascii_digit() || c == '.')
        && text.matches('.').count() <= 1
}

fn is_length(value: &str) -> bool {
    ["rem", "em", "px", "%", ""].iter().any(|unit| value.strip_suffix(unit).is_some_and(is_number))
}

fn is_colour(value: &str) -> bool {
    let hex = value.strip_prefix('#').is_some_and(|digits| {
        matches!(digits.len(), 3 | 4 | 6 | 8) && digits.chars().all(|c| c.is_ascii_hexdigit())
    });
    let function = COLOUR_FUNCTIONS.iter().any(|start| {
        value.strip_prefix(start).and_then(|rest| rest.strip_suffix(')')).is_some_and(|inside| {
            inside.chars().any(|c| c.is_ascii_digit())
                && inside.chars().all(|c| c.is_ascii_digit() || " .,%/-".contains(c))
        })
    });
    hex || function
}

fn check_tokens(tokens: &Tokens) -> Result<(), String> {
    for (name, value) in tokens {
        if !TOKENS.contains(&name.as_str()) {
            return Err(format!("The theme sets “{name}”, which is not a setting a theme can change."));
        }
        let fits = if name == "--radius" { is_length(value) } else { is_colour(value) };
        if !fits {
            let wanted = if name == "--radius" { "size" } else { "colour" };
            return Err(format!("The theme gives “{name}” the value “{value}”, which is not a {wanted}."));
        }
    }
    Ok(())
}

fn check_file(path: &str, extensions: &[&str]) -> Result<(), String> {
    let plain = !path.is_empty()
        && path.split('/').all(|name| {
            !name.is_empty()
                && !name.starts_with('.')
                && !name.chars().any(|c| c.is_control() || r#"<>:"\|?*"#.contains(c))
        });
    let known = path.rsplit_once('.').is_some_and(|(_, extension)| extensions.contains(&extension));
    if !plain || !known {
        return Err(format!(
            "The theme points to “{path}”, which is not a file of the right kind inside the theme."
        ));
    }
    Ok(())
}

fn check(manifest: &Manifest) -> Result<(), String> {
    if manifest.format != 1 {
        return Err("This theme was made for a newer version of znote.".into());
    }
    if !valid_id(&manifest.id) {
        return Err(format!(
            "“{}” cannot be used as a theme id. Use lowercase letters, digits and dashes.",
            manifest.id
        ));
    }
    if manifest.version.parse::<PackageVersion>().is_err() {
        return Err(format!("“{}” is not a version like 1.0.0.", manifest.version));
    }
    if !is_label(&manifest.name, " -_.&") {
        return Err("The theme name must be 1 to 60 letters, digits or spaces.".into());
    }
    let ui = &manifest.ui;
    check_tokens(&ui.light)?;
    check_tokens(&ui.dark)?;
    for logo in ui.logo.iter().chain(&ui.logo_dark) {
        check_file(logo, &IMAGES)?;
    }
    if let Some(font) = &ui.font {
        if !is_label(&font.family, " -") {
            return Err(format!("“{}” is not a font name.", font.family));
        }
        for file in &font.files {
            check_file(file, &FONTS)?;
        }
    }
    for template in &manifest.typst.templates {
        if !is_label(&template.label, " -_.&") {
            return Err("A template name must be 1 to 60 letters, digits or spaces.".into());
        }
    }
    Ok(())
}

fn parse(json: &[u8]) -> Result<Manifest, String> {
    let manifest = serde_json::from_slice(json).map_err(|error| format!("The theme’s theme.json is not valid: {error}"))?;
    check(&manifest)?;
    Ok(manifest)
}

fn inside(folder: &Path, path: &str) -> Result<PathBuf, String> {
    let file = folder.join(PathBuf::from_iter(path.split('/')));
    let contained = file.is_file()
        && file.canonicalize().and_then(|file| Ok(file.starts_with(folder.canonicalize()?))).unwrap_or(false);
    if !contained {
        return Err(format!("The theme points to “{path}”, but that file is not in the theme."));
    }
    Ok(file)
}

fn shown(file: PathBuf) -> String {
    file.to_string_lossy().into_owned()
}

fn font_file(folder: &Path, path: &str) -> Result<FontFile, String> {
    let file = inside(folder, path)?;
    let data = fs::read(&file).map_err(broken)?;
    let info = FontInfo::new(&data, 0).ok_or_else(|| format!("“{path}” is not a font file."))?;
    Ok(FontFile {
        path: shown(file),
        weight: info.variant.weight.to_number(),
        italic: info.variant.style != FontStyle::Normal,
    })
}

fn view(folder: &Path, manifest: Manifest) -> Result<Theme, String> {
    let Manifest { id, name, version, ui, typst, .. } = manifest;
    let image = |path: Option<String>| path.map(|path| inside(folder, &path).map(shown)).transpose();
    let (font_family, files) = ui.font.map(|font| (Some(font.family), font.files)).unwrap_or_default();
    Ok(Theme {
        id,
        name,
        version,
        logo: image(ui.logo)?,
        logo_dark: image(ui.logo_dark)?,
        font_family,
        fonts: files.iter().map(|path| font_file(folder, path)).collect::<Result<_, _>>()?,
        light: ui.light,
        dark: ui.dark,
        templates: typst.templates,
    })
}

fn entry_path(name: &str) -> Option<PathBuf> {
    let names: Vec<&str> = name.strip_suffix('/').unwrap_or(name).split('/').collect();
    let safe = names.iter().all(|name| {
        !matches!(*name, "" | "." | "..") && !name.chars().any(|c| c.is_control() || c == '\\' || c == ':')
    });
    safe.then(|| PathBuf::from_iter(names))
}

fn unpack<R: Read + Seek>(archive: &mut ZipArchive<R>, target: &Path, limit: u64) -> Result<(), String> {
    let mut left = limit;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(broken)?;
        let name = entry.name().map_err(broken)?.into_owned();
        let path = entry_path(&name)
            .ok_or_else(|| format!("This theme tries to put “{name}” outside its own folder, so it was not imported."))?;
        let file = target.join(path);
        if entry.is_dir() {
            fs::create_dir_all(&file).map_err(broken)?;
            continue;
        }
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).map_err(broken)?;
        }
        let mut output = File::create(&file).map_err(broken)?;
        let written = io::copy(&mut entry.by_ref().take(left + 1), &mut output).map_err(broken)?;
        if written > left {
            return Err("This theme is too large once unpacked, so it was not imported.".into());
        }
        left -= written;
    }
    Ok(())
}

fn open(folder: &Path) -> Result<Theme, String> {
    view(folder, parse(&fs::read(folder.join("theme.json")).map_err(broken)?)?)
}

pub fn install(themes: &Path, archive: &Path) -> Result<Theme, String> {
    let file = File::open(archive).map_err(broken)?;
    if file.metadata().map_err(broken)?.len() > MAX_ARCHIVE_BYTES {
        return Err("This theme file is larger than 50 MB, so it was not imported.".into());
    }
    let mut archive = ZipArchive::new(file).map_err(|_| "This file is not a znote theme.".to_string())?;
    if archive.len() > MAX_ENTRIES {
        return Err("This theme has too many files, so it was not imported.".into());
    }
    let mut json = Vec::new();
    archive
        .by_name("theme.json")
        .map_err(|_| "This file is not a znote theme: it has no theme.json.".to_string())?
        .take(MAX_MANIFEST_BYTES)
        .read_to_end(&mut json)
        .map_err(broken)?;
    let manifest = parse(&json)?;

    let staging = themes.join(format!(".incoming-{}", manifest.id));
    let home = themes.join(&manifest.id);
    let folder = home.join(&manifest.version);
    let _ = fs::remove_dir_all(&staging);
    let unpacked = unpack(&mut archive, &staging, MAX_UNPACKED_BYTES).and_then(|()| open(&staging)).and_then(|_| {
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).and_then(|()| fs::rename(&staging, &folder)).map_err(|error| failed("save", error))
    });
    if unpacked.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    unpacked?;
    view(&folder, manifest)
}

fn folders(folder: &Path) -> Vec<PathBuf> {
    fs::read_dir(folder).into_iter().flatten().flatten().map(|entry| entry.path()).filter(|path| path.is_dir()).collect()
}

pub fn list(themes: &Path) -> Vec<Theme> {
    let mut found: Vec<Theme> = folders(themes)
        .iter()
        .flat_map(|home| folders(home))
        .filter_map(|folder| {
            let theme = open(&folder).ok()?;
            folder.ends_with(Path::new(&theme.id).join(&theme.version)).then_some(theme)
        })
        .collect();
    found.sort_by_key(|theme| theme.name.to_lowercase());
    found
}

pub fn remove(themes: &Path, id: &str) -> Result<(), String> {
    if !valid_id(id) {
        return Err(format!("“{id}” is not a theme."));
    }
    fs::remove_dir_all(themes.join(id)).map_err(|error| failed("remove", error))
}

pub fn folder(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|error| error.to_string())?.join("themes"))
}

#[tauri::command]
pub fn list_themes(app: AppHandle) -> Result<Vec<Theme>, String> {
    Ok(list(&folder(&app)?))
}

#[tauri::command(async)]
pub fn import_theme(app: AppHandle) -> Result<Option<Theme>, String> {
    let Some(file) = app.dialog().file().add_filter("znote theme", &["znote-theme"]).blocking_pick_file() else {
        return Ok(None);
    };
    let file = file.into_path().map_err(|error| error.to_string())?;
    let theme = install(&folder(&app)?, &file)?;
    app.state::<Compiler>().forget_themes()?;
    Ok(Some(theme))
}

#[tauri::command(async)]
pub fn remove_theme(app: AppHandle, id: String) -> Result<(), String> {
    remove(&folder(&app)?, &id)?;
    app.state::<Compiler>().forget_themes()
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("znote-theme-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn archive(folder: &Path, entries: &[(&str, &[u8])]) -> PathBuf {
        let file = folder.join("test.znote-theme");
        let mut writer = ZipWriter::new(File::create(&file).unwrap());
        for (name, data) in entries {
            writer.start_file(*name, SimpleFileOptions::default()).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
        file
    }

    fn manifest(ui: &str) -> Vec<u8> {
        format!(r#"{{ "format": 1, "id": "acme", "name": "Acme", "version": "0.1.0", "ui": {ui} }}"#).into_bytes()
    }

    #[test]
    fn a_full_theme_installs_lists_and_is_removed() {
        let folder = scratch("full");
        let themes = folder.join("themes");
        let json = br##"{
            "format": 1, "id": "acme", "name": "Acme", "version": "0.1.0",
            "ui": {
                "logo": "ui/logo.svg",
                "font": { "family": "Acme Sans", "files": ["typst/fonts/regular.ttf"] },
                "light": { "--primary": "#0F172A", "--radius": "0.5rem" },
                "dark": { "--border": "oklch(1 0 0 / 10%)" }
            },
            "typst": { "templates": [{ "label": "Report", "kind": "document", "snippet": "= Report\n" }] }
        }"##;
        let font = typst_kit::fonts::embedded().next().unwrap().0;
        let file = archive(
            &folder,
            &[
                ("theme.json", json),
                ("ui/logo.svg", b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
                ("typst/fonts/regular.ttf", font.data()),
                ("typst/typst.toml", b""),
            ],
        );

        let theme = install(&themes, &file).unwrap();
        assert_eq!(theme.light["--primary"], "#0F172A");
        assert_eq!(theme.templates[0].kind, Kind::Document);
        assert_eq!(theme.fonts.len(), 1);
        assert!(Path::new(theme.logo.as_deref().unwrap()).starts_with(themes.join("acme").join("0.1.0")));
        assert!(themes.join("acme/0.1.0/typst/typst.toml").is_file());
        assert_eq!(list(&themes), [theme]);

        install(&themes, &file).unwrap();
        assert_eq!(list(&themes).len(), 1);

        remove(&themes, "acme").unwrap();
        assert!(list(&themes).is_empty());
        assert!(remove(&themes, "../acme").is_err());
    }

    #[test]
    fn an_archive_that_writes_outside_its_folder_is_refused() {
        let folder = scratch("slip");
        let themes = folder.join("themes");
        for name in ["../escaped.txt", "/escaped.txt", "ui/../../escaped.txt", "ui\\..\\..\\escaped.txt"] {
            let file = archive(&folder, &[("theme.json", &manifest("{}")), (name, b"gotcha")]);
            let error = install(&themes, &file).unwrap_err();
            assert!(error.contains("outside its own folder"), "{name}: {error}");
            assert!(list(&themes).is_empty());
            assert!(!folder.join("escaped.txt").exists());
            assert!(!themes.join("escaped.txt").exists());
        }
    }

    #[test]
    fn a_theme_that_is_too_large_is_refused() {
        let folder = scratch("large");
        let huge = folder.join("huge.znote-theme");
        File::create(&huge).unwrap().set_len(MAX_ARCHIVE_BYTES + 1).unwrap();
        assert!(install(&folder.join("themes"), &huge).unwrap_err().contains("larger than 50 MB"));

        let file = archive(&folder, &[("typst/zeros.bin", &[0; 4096])]);
        let mut bomb = ZipArchive::new(File::open(file).unwrap()).unwrap();
        assert!(unpack(&mut bomb, &folder.join("out"), 4095).unwrap_err().contains("too large"));
        assert!(unpack(&mut bomb, &folder.join("out"), 4096).is_ok());
    }

    #[test]
    fn unknown_settings_and_unsafe_values_are_refused() {
        for name in ["--evil", "background", "--font-sans", "--primary; color"] {
            let error = parse(&manifest(&format!(r##"{{ "light": {{ "{name}": "#fff" }} }}"##))).unwrap_err();
            assert!(error.contains("not a setting"), "{name}: {error}");
        }
        for value in [
            "url(https://example.com/x.png)",
            "#fff; background: url(x)",
            "red",
            "#ggg",
            "rgb(0 0 0); x",
            "expression(alert(1))",
            "var(--primary)",
            "",
        ] {
            let error = parse(&manifest(&format!(r#"{{ "dark": {{ "--primary": "{value}" }} }}"#))).unwrap_err();
            assert!(error.contains("not a colour"), "{value}: {error}");
        }
        for value in ["#fff", "#0F172Acc", "rgb(15 23 42)", "hsl(205, 100%, 10%)", "oklch(0.5 0.1 240 / 50%)"] {
            assert!(is_colour(value), "{value}");
        }
        assert!(parse(&manifest(r##"{ "light": { "--radius": "#fff" } }"##)).is_err());
        assert!(parse(&manifest(r#"{ "light": { "--radius": "calc(1px)" } }"#)).is_err());
        assert!(parse(&manifest(r#"{ "light": { "--radius": "0.75rem" } }"#)).is_ok());
    }

    #[test]
    fn unsafe_names_and_file_references_are_refused() {
        assert!(parse(&manifest(r#"{ "logo": "../../secret.svg" }"#)).is_err());
        assert!(parse(&manifest(r#"{ "logo": "ui/logo.html" }"#)).is_err());
        assert!(parse(&manifest(r#"{ "logo": "C:\\logo.svg" }"#)).is_err());
        assert!(parse(&manifest(r#"{ "font": { "family": "x; } body { display: none", "files": [] } }"#)).is_err());
        assert!(parse(&manifest(r#"{ "font": { "family": "Acme Sans", "files": ["ui/font.exe"] } }"#)).is_err());
        assert!(parse(&manifest(r#"{ "logo": "ui/logo.svg" }"#)).is_ok());

        let named = |id: &str, version: &str| {
            parse(format!(r#"{{ "format": 1, "id": "{id}", "name": "Acme", "version": "{version}" }}"#).as_bytes())
        };
        assert!(named("acme", "0.1.0").is_ok());
        assert!(named("../acme", "0.1.0").is_err());
        assert!(named("Acme", "0.1.0").is_err());
        assert!(named("acme", "../1").is_err());
        assert!(parse(br#"{ "format": 2, "id": "acme", "name": "Acme", "version": "0.1.0" }"#).is_err());

        let folder = scratch("missing");
        let file = archive(&folder, &[("theme.json", &manifest(r#"{ "logo": "ui/logo.svg" }"#))]);
        assert!(install(&folder.join("themes"), &file).unwrap_err().contains("not in the theme"));
        assert!(list(&folder.join("themes")).is_empty());
    }
}
