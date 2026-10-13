import { convertFileSrc, invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LazyStore } from "@tauri-apps/plugin-store";

export type Diagnostic = {
  severity: "error" | "warning";
  message: string;
  from: number;
  to: number;
};

export type RenderedPage = { index: number; svg: string; width: number; height: number };

export type Compiled = {
  page: string;
  seq: number;
  pageCount: number | null;
  pages: RenderedPage[];
  diagnostics: Diagnostic[];
};

export const inDesktopApp = isTauri;

export const appVersion = () => invoke<string>("app_version");

export const compile = (page: string, seq: number, text: string, fresh: boolean) =>
  invoke<void>("compile", { page, seq, text, fresh });

export const onCompiled = (handler: (result: Compiled) => void) =>
  listen<Compiled>("compiled", (event) => handler(event.payload));

export const onDownloading = (handler: (packageName: string) => void) =>
  listen<string>("downloading", (event) => handler(event.payload));

export const onCompileFailed = (handler: (message: string) => void) =>
  listen<string>("compile-failed", (event) => handler(event.payload));

export type Notebook = { name: string; pages: string[] };

export type Project = { name: string; notebooks: Notebook[] };

export const defaultWorkspace = () => invoke<string>("default_workspace");

export const pickFolder = () => invoke<string | null>("pick_folder");

export type PickedFile = { name: string; text: string };

export const pickTypstFile = () => invoke<PickedFile | null>("pick_typst_file");

export const openWorkspace = (folder: string) => invoke<Project[]>("open_workspace", { folder });

export const listTree = () => invoke<Project[]>("list_tree");

export const createEntry = (path: string) => invoke<void>("create_entry", { path });

export const moveEntry = (from: string, to: string) => invoke<void>("move_entry", { from, to });

export const deleteEntry = (path: string) => invoke<void>("delete_entry", { path });

export const readPage = (path: string) => invoke<string>("read_page", { path });

export const writePage = (path: string, text: string) => invoke<void>("write_page", { path, text });

export const onPageChanged = (handler: (page: string) => void) =>
  listen<string>("page-changed", (event) => handler(event.payload));

export const onWorkspaceChanged = (handler: () => void) => listen("workspace-changed", handler);

export const onClosing = (handler: (keepOpen: () => void) => Promise<void>) =>
  getCurrentWindow().onCloseRequested((event) => handler(() => event.preventDefault()));

export type TemplateKind = "document" | "slides" | "poster";

export type Template = { label: string; kind: TemplateKind; snippet: string };

export type FontFile = { path: string; weight: number; italic: boolean };

export type Theme = {
  id: string;
  name: string;
  version: string;
  logo: string | null;
  logoDark: string | null;
  fontFamily: string | null;
  fonts: FontFile[];
  light: Record<string, string>;
  dark: Record<string, string>;
  templates: Template[];
};

export const listThemes = () => invoke<Theme[]>("list_themes");

export const importTheme = () => invoke<Theme | null>("import_theme");

export const removeTheme = (id: string) => invoke<void>("remove_theme", { id });

export const themeFileUrl = (path: string) => convertFileSrc(path);

export type ExportFormat = "pdf" | "pptx";

export type Exported = { path: string; missingFonts: string[] };

export const exportPage = (page: string, text: string, format: ExportFormat) =>
  invoke<Exported | null>("export_page", { page, text, format });

export const openExport = () => invoke<void>("open_export");

export const showExport = () => invoke<void>("show_export");

const settingsStore = new LazyStore("settings.json");

export const readSettings = async () => Object.fromEntries(await settingsStore.entries<unknown>());

export const writeSetting = (key: string, value: unknown) => settingsStore.set(key, value);
