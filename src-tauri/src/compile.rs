use std::any::Any;
use std::collections::HashMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use typst::diag::{FileResult, PackageError, Severity, SourceDiagnostic, SourceResult, Warned};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::package::PackageSpec;
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::{LazyHash, hash128};
use typst::{Library, LibraryExt, World, WorldExt};
use typst_kit::datetime::Time;
use typst_kit::downloader::{Downloader, SystemDownloader};
use typst_kit::files::{FileLoader, FileStore, FsRoot};
use typst_kit::fonts::{self, FontStore};
use typst_kit::packages::SystemPackages;
use typst_layout::PagedDocument;
use typst_svg::SvgOptions;

use crate::theme;
use crate::workspace::Workspace;

static LIBRARY: LazyLock<LazyHash<Library>> = LazyLock::new(|| LazyHash::new(Library::default()));

pub fn font_store(themes: &Path) -> FontStore {
    let mut store = FontStore::new();
    store.extend(fonts::system());
    store.extend(fonts::embedded());
    store.extend(fonts::scan(themes));
    store
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    severity: &'static str,
    message: String,
    from: usize,
    to: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderedPage {
    index: usize,
    svg: String,
    width: f64,
    height: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Compiled {
    page_count: Option<usize>,
    pages: Vec<RenderedPage>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompiledEvent<'a> {
    page: &'a str,
    seq: u64,
    #[serde(flatten)]
    result: &'a Compiled,
}

struct Announcing<F> {
    inner: SystemDownloader,
    announce: F,
}

impl<F: Fn(&str) + Send + Sync + 'static> Downloader for Announcing<F> {
    fn stream(&self, key: &dyn Any, url: &str) -> io::Result<(Option<usize>, Box<dyn Read>)> {
        if let Some(spec) = key.downcast_ref::<PackageSpec>() {
            (self.announce)(&spec.to_string());
        }
        self.inner.stream(key, url)
    }
}

struct Loader {
    main: FileId,
    text: String,
    project: FsRoot,
    themes: PathBuf,
    packages: SystemPackages,
}

impl FileLoader for Loader {
    fn load(&self, id: FileId) -> FileResult<Bytes> {
        if id == self.main {
            return Ok(Bytes::from_string(self.text.clone()));
        }
        match id.root() {
            VirtualRoot::Package(spec) if spec.namespace == "themes" => {
                let package = self.themes.join(spec.name.as_str()).join(spec.version.to_string()).join("typst");
                if !theme::valid_id(&spec.name) || !package.is_dir() {
                    return Err(PackageError::NotFound(spec.clone()).into());
                }
                FsRoot::new(package).load(id.vpath())
            }
            VirtualRoot::Package(spec) => self.packages.obtain(spec)?.load(id.vpath()),
            VirtualRoot::Project => self.project.load(id.vpath()),
        }
    }
}

pub struct PageWorld {
    main: FileId,
    files: FileStore<Loader>,
    fonts: Arc<FontStore>,
    time: Time,
    hashes: Vec<u128>,
}

impl World for PageWorld {
    fn library(&self) -> &LazyHash<Library> {
        &LIBRARY
    }

    fn book(&self) -> &LazyHash<FontBook> {
        self.fonts.book()
    }

    fn main(&self) -> FileId {
        self.main
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        self.files.source(id)
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.files.file(id)
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.font(index)
    }

    fn today(&self, offset: Option<Duration>) -> Option<Datetime> {
        self.time.today(offset)
    }
}

impl PageWorld {
    pub fn new(
        project: &Path,
        main: &str,
        themes: &Path,
        fonts: Arc<FontStore>,
        announce_download: impl Fn(&str) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let path = VirtualPath::new(main).map_err(|error| error.to_string())?;
        let main = FileId::unique(RootedPath::new(VirtualRoot::Project, path));
        let downloader = Announcing {
            inner: SystemDownloader::new(concat!("znote/", env!("CARGO_PKG_VERSION"))),
            announce: announce_download,
        };
        let loader = Loader {
            main,
            text: String::new(),
            project: FsRoot::new(project.to_path_buf()),
            themes: themes.to_path_buf(),
            packages: SystemPackages::new(downloader),
        };
        Ok(Self { main, files: FileStore::new(loader), fonts, time: Time::system(), hashes: Vec::new() })
    }

    pub fn forget_sent_pages(&mut self) {
        self.hashes.clear();
    }

    fn typeset(&mut self, text: String) -> Warned<SourceResult<PagedDocument>> {
        self.files.loader_mut().text = text;
        self.files.reset();
        self.time.reset();
        typst::compile::<PagedDocument>(self)
    }

    pub fn document(&mut self, text: String) -> Result<PagedDocument, String> {
        self.typeset(text).output.map_err(|_| "Fix the problems on this page before you export it.".to_string())
    }

    pub fn compile(&mut self, text: String) -> Compiled {
        let warned = self.typeset(text);
        let mut diagnostics: Vec<Diagnostic> =
            warned.warnings.iter().filter(|d| !self.template_font_fallback(d)).map(|d| self.diagnostic(d)).collect();

        let document = match warned.output {
            Ok(document) => document,
            Err(errors) => {
                diagnostics.extend(errors.iter().map(|d| self.diagnostic(d)));
                return Compiled { page_count: None, pages: Vec::new(), diagnostics };
            }
        };

        let hashes: Vec<u128> = document.pages().iter().map(hash128).collect();
        let pages = document
            .pages()
            .iter()
            .enumerate()
            .filter(|(index, _)| self.hashes.get(*index) != Some(&hashes[*index]))
            .map(|(index, page)| RenderedPage {
                index,
                svg: typst_svg::svg(page, &SvgOptions::default()),
                width: page.frame.width().to_pt(),
                height: page.frame.height().to_pt(),
            })
            .collect();
        self.hashes = hashes;
        Compiled { page_count: Some(self.hashes.len()), pages, diagnostics }
    }

    fn template_font_fallback(&self, diagnostic: &SourceDiagnostic) -> bool {
        diagnostic.span.id() != Some(self.main) && diagnostic.message.starts_with("unknown font family")
    }

    fn diagnostic(&self, diagnostic: &SourceDiagnostic) -> Diagnostic {
        let mut message = diagnostic.message.to_string();
        for hint in &diagnostic.hints {
            message.push_str("\nHint: ");
            message.push_str(&hint.v);
        }
        let (from, to) = self.utf16_range(diagnostic).unwrap_or((0, 0));
        Diagnostic {
            severity: match diagnostic.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            },
            message,
            from,
            to,
        }
    }

    fn utf16_range(&self, diagnostic: &SourceDiagnostic) -> Option<(usize, usize)> {
        if diagnostic.span.id()? != self.main {
            return None;
        }
        let range = self.range(diagnostic.span)?;
        let source = self.source(self.main).ok()?;
        let lines = source.lines();
        Some((lines.byte_to_utf16(range.start)?, lines.byte_to_utf16(range.end)?))
    }
}

#[derive(Default)]
pub struct Compiler {
    latest: Mutex<HashMap<String, u64>>,
    world: Mutex<Option<(PathBuf, PageWorld)>>,
    fonts: Mutex<Option<Arc<FontStore>>>,
}

impl Compiler {
    pub fn forget_themes(&self) -> Result<(), String> {
        *self.world.lock().map_err(|error| error.to_string())? = None;
        *self.fonts.lock().map_err(|error| error.to_string())? = None;
        Ok(())
    }
}

type OpenWorld = Option<(PathBuf, PageWorld)>;

fn open_world<'a>(app: &AppHandle, open: &'a mut OpenWorld, page: &str) -> Result<&'a mut PageWorld, String> {
    let (project, main) = app.state::<Workspace>().typst_source(page)?;
    let file = project.join(&main);
    if open.as_ref().is_none_or(|(current, _)| *current != file) {
        let announcer = app.clone();
        let themes = theme::folder(app)?;
        let fonts = app
            .state::<Compiler>()
            .fonts
            .lock()
            .map_err(|error| error.to_string())?
            .get_or_insert_with(|| Arc::new(font_store(&themes)))
            .clone();
        let world = PageWorld::new(&project, &main, &themes, fonts, move |package| {
            let _ = announcer.emit("downloading", package);
        })?;
        *open = Some((file, world));
    }
    open.as_mut().map(|(_, world)| world).ok_or_else(|| "The page could not be opened.".to_string())
}

pub fn document(app: &AppHandle, page: &str, text: String) -> Result<PagedDocument, String> {
    let compiler = app.state::<Compiler>();
    let mut open = compiler.world.lock().map_err(|error| error.to_string())?;
    open_world(app, &mut open, page)?.document(text)
}

fn compile_page(app: &AppHandle, page: &str, seq: u64, text: String, fresh: bool) -> Result<(), String> {
    let compiler = app.state::<Compiler>();
    let mut open = compiler.world.lock().map_err(|error| error.to_string())?;
    let superseded = compiler.latest.lock().map_err(|error| error.to_string())?.get(page) != Some(&seq);
    if superseded {
        return Ok(());
    }
    let world = open_world(app, &mut open, page)?;
    if fresh {
        world.forget_sent_pages();
    }
    let result = world.compile(text);
    app.emit("compiled", CompiledEvent { page, seq, result: &result }).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn compile(
    app: AppHandle,
    compiler: State<Compiler>,
    page: String,
    seq: u64,
    text: String,
    fresh: bool,
) -> Result<(), String> {
    compiler.latest.lock().map_err(|error| error.to_string())?.insert(page.clone(), seq);
    std::thread::spawn(move || {
        if let Err(message) = compile_page(&app, &page, seq, text, fresh) {
            let _ = app.emit("compile-failed", message);
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    static FONTS: LazyLock<Arc<FontStore>> =
        LazyLock::new(|| Arc::new(font_store(Path::new(env!("CARGO_MANIFEST_DIR")).join("no-themes").as_path())));

    fn world() -> PageWorld {
        let folder = std::env::temp_dir();
        PageWorld::new(&folder, "main.typ", &folder.join("no-themes"), FONTS.clone(), |_| {}).unwrap()
    }

    #[test]
    fn a_theme_package_is_read_from_the_themes_folder_with_its_fonts() {
        let themes = std::env::temp_dir().join(format!("znote-compile-themes-{}", std::process::id()));
        let package = themes.join("acme").join("0.1.0").join("typst");
        std::fs::create_dir_all(package.join("fonts")).unwrap();
        std::fs::write(package.join("typst.toml"), "[package]\nname = \"acme\"\nversion = \"0.1.0\"\nentrypoint = \"lib.typ\"\n")
            .unwrap();
        std::fs::write(package.join("lib.typ"), "#let brand = text(font: (\"Not Shipped\", \"Libertinus Serif\"))[Acme]").unwrap();
        let font = fonts::embedded().next().unwrap().0;
        std::fs::write(package.join("fonts").join("copy.ttf"), font.data()).unwrap();

        let fonts = Arc::new(font_store(&themes));
        assert!(FONTS.book().families().count() > 0);
        assert!(fonts.source(FONTS.book().families().map(|(_, infos)| infos.count()).sum::<usize>()).is_some());

        let mut world = PageWorld::new(&std::env::temp_dir(), "main.typ", &themes, fonts, |_| {}).unwrap();
        let result = world.compile("#import \"@themes/acme:0.1.0\": brand\n#brand".into());
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.page_count, Some(1));

        let own = world.compile("#set text(font: \"Not Shipped\")\nHello".into());
        assert!(own.diagnostics[0].message.contains("unknown font family"), "{:?}", own.diagnostics);

        let missing = world.compile("#import \"@themes/other:0.1.0\": brand".into());
        assert_eq!(missing.page_count, None);
        assert!(missing.diagnostics[0].message.contains("package not found"), "{:?}", missing.diagnostics);
    }

    #[test]
    fn a_page_reads_other_files_from_its_project_folder() {
        let project = std::env::temp_dir().join(format!("znote-compile-{}", std::process::id()));
        std::fs::create_dir_all(project.join("Notes")).unwrap();
        std::fs::write(project.join("shared.typ"), "#let greeting = [Hello]").unwrap();
        let mut world =
            PageWorld::new(&project, "Notes/Page.typ", &project.join("no-themes"), FONTS.clone(), |_| {}).unwrap();

        let result = world.compile("#import \"../shared.typ\": greeting\n#greeting".into());
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.page_count, Some(1));

        let escaping = world.compile("#read(\"../../outside.txt\")".into());
        assert_eq!(escaping.page_count, None);
    }

    #[test]
    fn valid_source_gives_its_pages() {
        let result = world().compile("One #pagebreak() Two #pagebreak() Three".into());
        assert_eq!(result.page_count, Some(3));
        assert_eq!(result.pages.len(), 3);
        assert!(result.pages[0].svg.starts_with("<svg"));
        assert!(result.pages[0].width > 0.0);
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn invalid_source_gives_a_diagnostic_with_a_range() {
        let result = world().compile("é #nope".into());
        assert_eq!(result.page_count, None);
        assert_eq!(result.diagnostics.len(), 1);
        let diagnostic = &result.diagnostics[0];
        assert_eq!(diagnostic.severity, "error");
        assert_eq!((diagnostic.from, diagnostic.to), (3, 7));
    }

    #[test]
    fn only_changed_pages_are_sent_again() {
        let mut world = world();
        world.compile("One #pagebreak() Two".into());
        let result = world.compile("One #pagebreak() Changed".into());
        assert_eq!(result.page_count, Some(2));
        assert_eq!(result.pages.iter().map(|page| page.index).collect::<Vec<_>>(), [1]);

        let unchanged = world.compile("One #pagebreak() Changed".into());
        assert_eq!(unchanged.page_count, Some(2));
        assert!(unchanged.pages.is_empty());

        world.forget_sent_pages();
        assert_eq!(world.compile("One #pagebreak() Changed".into()).pages.len(), 2);
    }
}
