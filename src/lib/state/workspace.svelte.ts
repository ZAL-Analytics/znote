import * as ipc from "$lib/ipc";
import type { Project } from "$lib/ipc";
import { messageOf } from "$lib/utils";
import { Document, doc } from "./document.svelte";
import { SettingsState, loadSettings, settings } from "./settings.svelte";

type Files = Pick<
  typeof ipc,
  "openWorkspace" | "listTree" | "createEntry" | "moveEntry" | "deleteEntry" | "readPage" | "writePage"
>;

const saveDelayMs = 1000;
const refreshDelayMs = 200;

const sample = { project: "Getting started", notebook: "Notes", page: "Welcome" };

const sampleText = `= Welcome to znote

Type Typst on the left. The pages on the right update as you type.

- *Bold*, _emphasis_ and \`code\`
- Maths: $ sum_(k=1)^n k = (n(n+1)) / 2 $
`;

export const nameOf = (path: string) => path.slice(path.lastIndexOf("/") + 1);

export const parentOf = (path: string) => path.slice(0, Math.max(0, path.lastIndexOf("/")));

export function childOf(parent: string, name: string) {
  const trimmed = name.trim();
  if (trimmed === "") throw new Error("Type a name first.");
  if (trimmed.includes("/")) throw new Error("A name cannot contain a slash.");
  return parent === "" ? trimmed : `${parent}/${trimmed}`;
}

export function freeName(name: string, taken: string[]) {
  if (!taken.includes(name)) return name;
  let copy = 2;
  while (taken.includes(`${name} ${copy}`)) copy += 1;
  return `${name} ${copy}`;
}

export function moved(path: string | null, from: string, to: string) {
  if (path === from) return to;
  return path !== null && path.startsWith(`${from}/`) ? to + path.slice(from.length) : path;
}

export const pagesOf = (tree: Project[]) =>
  tree.flatMap((project) =>
    project.notebooks.flatMap((notebook) => notebook.pages.map((page) => `${project.name}/${notebook.name}/${page}`)),
  );

export class Workspace {
  tree = $state<Project[]>([]);
  project = $state<string | null>(null);
  page = $state<string | null>(null);
  failure = $state<string | null>(null);
  needsFolder = $state(false);

  private saved = $state("");
  private timer: ReturnType<typeof setTimeout> | undefined;

  constructor(
    private files: Files,
    private doc: Document,
    private settings: SettingsState,
  ) {}

  get notebooks() {
    return this.tree.find((project) => project.name === this.project)?.notebooks ?? [];
  }

  get unsaved() {
    return this.page !== null && this.doc.text !== this.saved;
  }

  async attempt(task: () => Promise<void>) {
    try {
      await task();
      this.failure = null;
    } catch (error) {
      this.failure = messageOf(error);
    }
  }

  async flush() {
    clearTimeout(this.timer);
    while (this.unsaved) {
      const page = this.page;
      const text = this.doc.text;
      if (page === null) return;
      await this.files.writePage(page, text);
      if (this.page === page) this.saved = text;
    }
  }

  save() {
    return this.attempt(() => this.flush());
  }

  edit(text: string) {
    this.doc.edit(text);
    clearTimeout(this.timer);
    if (this.settings.values.autosave) this.timer = setTimeout(() => this.save(), saveDelayMs);
  }

  rewrite(change: (text: string) => string) {
    if (this.page === null) return;
    this.doc.open(this.page, change(this.doc.text));
    this.edit(this.doc.text);
  }

  private show(page: string | null, text: string) {
    this.page = page;
    this.saved = text;
    if (page !== null) this.project = page.split("/")[0];
    this.doc.open(page ?? "", text);
    this.settings.set("lastPage", page);
  }

  async open(page: string | null) {
    if (page === this.page) return;
    const text = page === null ? "" : await this.files.readPage(page);
    await this.flush();
    this.show(page, text);
  }

  async useFolder(folder: string, firstRun: boolean) {
    await this.flush();
    this.tree = await this.files.openWorkspace(folder);
    this.settings.set("workspace", folder);
    this.needsFolder = false;
    this.page = null;
    if (firstRun && this.tree.length === 0) {
      const notebook = `${sample.project}/${sample.notebook}`;
      const page = `${notebook}/${sample.page}`;
      await this.files.createEntry(sample.project);
      await this.files.createEntry(notebook);
      await this.files.createEntry(page);
      await this.files.writePage(page, sampleText);
      this.tree = await this.files.listTree();
    }
    const pages = pagesOf(this.tree);
    const last = this.settings.values.lastPage;
    const page = last !== null && pages.includes(last) ? last : (pages[0] ?? null);
    this.project = this.tree[0]?.name ?? null;
    this.show(page, page === null ? "" : await this.files.readPage(page));
  }

  async refresh() {
    this.tree = await this.files.listTree();
    if (!this.tree.some((project) => project.name === this.project)) this.project = this.tree[0]?.name ?? null;
    if (this.page !== null && !this.unsaved && !pagesOf(this.tree).includes(this.page)) this.show(null, "");
  }

  async reload(page: string) {
    if (page !== this.page || this.unsaved) return;
    const text = await this.files.readPage(page);
    if (page !== this.page || this.unsaved || text === this.saved) return;
    this.saved = text;
    this.doc.open(page, text);
  }

  async importPage(notebook: string, file: { name: string; text: string }) {
    const [project, notebookName] = notebook.split("/");
    const siblings = this.tree.find(({ name }) => name === project)?.notebooks.find(({ name }) => name === notebookName);
    const path = childOf(notebook, freeName(file.name, siblings?.pages ?? []));
    await this.files.createEntry(path);
    await this.files.writePage(path, file.text);
    await this.refresh();
    await this.open(path);
  }

  async create(path: string) {
    await this.files.createEntry(path);
    await this.refresh();
    const depth = path.split("/").length;
    if (depth === 1) this.project = path;
    if (depth === 3) await this.open(path);
  }

  async move(from: string, to: string) {
    if (from === to) return;
    await this.flush();
    await this.files.moveEntry(from, to);
    const page = moved(this.page, from, to);
    if (page !== this.page) {
      this.page = page;
      this.doc.open(page ?? "", this.doc.text);
      this.settings.set("lastPage", page);
    }
    this.project = moved(this.project, from, to);
    await this.refresh();
  }

  async remove(path: string) {
    await this.flush();
    await this.files.deleteEntry(path);
    if (moved(this.page, path, "") !== this.page) this.show(null, "");
    await this.refresh();
  }
}

export const workspace = new Workspace(ipc, doc, settings);

export async function start() {
  if (!ipc.inDesktopApp()) return;
  let refreshTimer: ReturnType<typeof setTimeout> | undefined;
  let warned = false;
  await workspace.attempt(async () => {
    await loadSettings();
    await Promise.all([
      ipc.onPageChanged((page) => workspace.attempt(() => workspace.reload(page))),
      ipc.onWorkspaceChanged(() => {
        clearTimeout(refreshTimer);
        refreshTimer = setTimeout(() => workspace.attempt(() => workspace.refresh()), refreshDelayMs);
      }),
      ipc.onClosing(async (keepOpen) => {
        try {
          await workspace.flush();
        } catch (error) {
          if (warned) return;
          warned = true;
          keepOpen();
          workspace.failure = `Your latest changes could not be saved. Close again to leave without them. ${messageOf(error)}`;
        }
      }),
    ]);
    const folder = settings.values.workspace;
    if (folder === null) workspace.needsFolder = true;
    else await workspace.useFolder(folder, false);
  });
}

if (import.meta.vitest) {
  const { test, expect, vi, beforeEach } = import.meta.vitest;

  function treeOf(disk: Record<string, string>) {
    const tree: Project[] = [];
    for (const path of Object.keys(disk)) {
      const [projectName, notebookName, page] = path.split("/");
      const project = tree.find(({ name }) => name === projectName) ?? { name: projectName, notebooks: [] };
      if (!tree.includes(project)) tree.push(project);
      const notebook = project.notebooks.find(({ name }) => name === notebookName) ?? { name: notebookName, pages: [] };
      if (!project.notebooks.includes(notebook)) project.notebooks.push(notebook);
      notebook.pages.push(page);
    }
    return tree;
  }

  function setup(disk: Record<string, string> = { "Work/Notes/A": "a", "Work/Notes/B": "b" }) {
    const files = {
      openWorkspace: vi.fn(async () => treeOf(disk)),
      listTree: vi.fn(async () => treeOf(disk)),
      createEntry: vi.fn(async () => {}),
      moveEntry: vi.fn(async (from: string, to: string) => {
        for (const path of Object.keys(disk)) {
          const target = moved(path, from, to);
          if (target === null || target === path) continue;
          disk[target] = disk[path];
          delete disk[path];
        }
      }),
      deleteEntry: vi.fn(async () => {}),
      readPage: vi.fn(async (path: string) => disk[path]),
      writePage: vi.fn(async (path: string, text: string) => {
        disk[path] = text;
      }),
    };
    const document = new Document(() => Promise.resolve());
    const chosen = new SettingsState(() => Promise.resolve());
    return { files, disk, document, chosen, workspace: new Workspace(files, document, chosen) };
  }

  beforeEach(() => {
    vi.useFakeTimers();
  });

  test("an imported page gets a free name and never replaces a page", async () => {
    expect(freeName("A", ["B"])).toBe("A");
    expect(freeName("A", ["A", "A 2"])).toBe("A 3");
    const { workspace, files, disk } = setup();
    await workspace.useFolder("/notes", false);
    await workspace.importPage("Work/Notes", { name: "A", text: "imported" });
    expect(files.createEntry).toHaveBeenCalledWith("Work/Notes/A 2");
    expect(disk["Work/Notes/A"]).toBe("a");
    expect(disk["Work/Notes/A 2"]).toBe("imported");
    expect(workspace.page).toBe("Work/Notes/A 2");
  });

  test("paths are built and followed through renames", () => {
    expect(childOf("", " Work ")).toBe("Work");
    expect(childOf("Work/Notes", "A")).toBe("Work/Notes/A");
    expect(() => childOf("Work", "a/b")).toThrow();
    expect(() => childOf("Work", "  ")).toThrow();
    expect(parentOf("Work/Notes/A")).toBe("Work/Notes");
    expect(parentOf("Work")).toBe("");
    expect(nameOf("Work/Notes/A")).toBe("A");
    expect(moved("Work/Notes/A", "Work/Notes", "Work/Old")).toBe("Work/Old/A");
    expect(moved("Work/Notes/A", "Work", "Job")).toBe("Job/Notes/A");
    expect(moved("Work/Notes/A", "Work/Not", "Work/X")).toBe("Work/Notes/A");
    expect(moved(null, "Work", "Job")).toBe(null);
  });

  test("typing is saved after a pause, and not at all while autosave is off", async () => {
    const { workspace, disk, files, chosen } = setup();
    await workspace.open("Work/Notes/A");
    workspace.edit("a1");
    workspace.edit("a12");
    await vi.advanceTimersByTimeAsync(saveDelayMs - 1);
    expect(disk["Work/Notes/A"]).toBe("a");
    await vi.advanceTimersByTimeAsync(1);
    expect(disk["Work/Notes/A"]).toBe("a12");
    expect(files.writePage).toHaveBeenCalledTimes(1);

    chosen.set("autosave", false);
    workspace.edit("a123");
    await vi.advanceTimersByTimeAsync(saveDelayMs * 5);
    expect(disk["Work/Notes/A"]).toBe("a12");
    await workspace.save();
    expect(disk["Work/Notes/A"]).toBe("a123");
  });

  test("leaving a page saves it first, and a failed save keeps it open", async () => {
    const { workspace, disk, files, document } = setup();
    await workspace.open("Work/Notes/A");
    workspace.edit("changed");
    await workspace.open("Work/Notes/B");
    expect(disk["Work/Notes/A"]).toBe("changed");
    expect(document.text).toBe("b");

    workspace.edit("precious");
    files.writePage.mockRejectedValueOnce("disk full");
    await expect(workspace.open("Work/Notes/A")).rejects.toBe("disk full");
    expect(workspace.page).toBe("Work/Notes/B");
    expect(document.text).toBe("precious");
  });

  test("a change on disk is loaded only when nothing here is unsaved", async () => {
    const { workspace, disk, document } = setup();
    await workspace.open("Work/Notes/A");
    disk["Work/Notes/A"] = "from outside";
    await workspace.reload("Work/Notes/A");
    expect(document.text).toBe("from outside");

    workspace.edit("mine");
    disk["Work/Notes/A"] = "outside again";
    await workspace.reload("Work/Notes/A");
    expect(document.text).toBe("mine");
    await workspace.reload("Work/Notes/B");
    expect(document.text).toBe("mine");
  });

  test("renaming the open page or its notebook keeps the text and saves to the new place", async () => {
    const { workspace, disk, document } = setup();
    await workspace.open("Work/Notes/A");
    workspace.edit("unsaved");
    await workspace.move("Work/Notes", "Work/Old");
    expect(workspace.page).toBe("Work/Old/A");
    expect(disk["Work/Old/A"]).toBe("unsaved");
    workspace.edit("more");
    await workspace.save();
    expect(disk["Work/Old/A"]).toBe("more");
    expect(disk["Work/Notes/A"]).toBeUndefined();
    expect(document.text).toBe("more");
  });

  test("the first run makes a sample page, and a later start reopens the last page", async () => {
    const first = setup({});
    await first.workspace.useFolder("/notes", true);
    expect(Object.keys(first.disk)).toEqual(["Getting started/Notes/Welcome"]);

    const later = setup();
    later.chosen.set("lastPage", "Work/Notes/B");
    await later.workspace.useFolder("/notes", false);
    expect(later.workspace.page).toBe("Work/Notes/B");
    expect(later.workspace.project).toBe("Work");
    expect(later.document.text).toBe("b");
  });
}
