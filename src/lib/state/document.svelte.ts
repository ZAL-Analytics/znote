import { compile, inDesktopApp, onCompileFailed, onCompiled, onDownloading } from "$lib/ipc";
import type { Compiled, Diagnostic } from "$lib/ipc";

export type PageView = { url: string; width: number; height: number };

type Send = (page: string, seq: number, text: string, fresh: boolean) => Promise<void>;

const debounceMs = 150;

export class Document {
  text = $state("");
  pages = $state<PageView[]>([]);
  diagnostics = $state<Diagnostic[]>([]);
  downloading = $state<string | null>(null);
  failure = $state<string | null>(null);
  loading = $state(false);
  opened = $state(0);

  private sent = 0;
  private applied = 0;
  private fresh = true;
  private timer: ReturnType<typeof setTimeout> | undefined;

  constructor(
    private send: Send,
    public page = "",
  ) {}

  open(page: string, text: string) {
    clearTimeout(this.timer);
    this.pages.forEach((shown) => URL.revokeObjectURL(shown.url));
    this.page = page;
    this.text = text;
    this.pages = [];
    this.diagnostics = [];
    this.failure = null;
    this.applied = this.sent;
    this.fresh = true;
    this.opened += 1;
    this.loading = false;
    if (page) this.request();
  }

  edit(text: string) {
    this.text = text;
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.request(), debounceMs);
  }

  refresh() {
    clearTimeout(this.timer);
    this.fresh = true;
    if (this.page) this.request();
  }

  request() {
    this.sent += 1;
    this.loading = true;
    this.send(this.page, this.sent, this.text, this.fresh).catch((error: unknown) => this.fail(String(error)));
  }

  fail(message: string) {
    this.loading = false;
    this.downloading = null;
    this.failure = message;
  }

  receive(result: Compiled) {
    if (result.page !== this.page || result.seq <= this.applied) return;
    this.applied = result.seq;
    this.fresh = false;
    this.loading = result.seq !== this.sent;
    this.downloading = null;
    this.failure = null;

    if (result.pageCount !== null) {
      const pages = this.pages.slice(0, result.pageCount);
      this.pages.slice(result.pageCount).forEach((page) => URL.revokeObjectURL(page.url));
      for (const { index, svg, width, height } of result.pages) {
        if (pages[index]) URL.revokeObjectURL(pages[index].url);
        const url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
        pages[index] = { url, width, height };
      }
      this.pages = pages;
    }

    if (result.seq === this.sent) this.diagnostics = result.diagnostics;
  }
}

export const doc = new Document(compile);

export function connect() {
  if (!inDesktopApp()) return doc.fail("it only works in the desktop app.");
  Promise.all([
    onCompiled((result) => doc.receive(result)),
    onDownloading((packageName) => (doc.downloading = packageName)),
    onCompileFailed((message) => doc.fail(message)),
  ]).catch((error: unknown) => doc.fail(String(error)));
}

if (import.meta.vitest) {
  const { test, expect, vi, beforeEach } = import.meta.vitest;

  const rendered = (index: number) => ({ index, svg: "<svg/>", width: 595, height: 842 });
  const problem: Diagnostic = { severity: "error", message: "unknown variable", from: 1, to: 5 };
  const result = (seq: number, patch: Partial<Compiled> = {}): Compiled => ({
    page: "scratch",
    seq,
    pageCount: 1,
    pages: [rendered(0)],
    diagnostics: [],
    ...patch,
  });

  beforeEach(() => {
    vi.useFakeTimers();
  });

  test("typing sends one request with the latest text after the pause", () => {
    const send = vi.fn<Send>(() => Promise.resolve());
    const doc = new Document(send, "scratch");
    doc.edit("a");
    doc.edit("ab");
    vi.advanceTimersByTime(debounceMs - 1);
    expect(send).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(send.mock.calls).toEqual([["scratch", 1, "ab", true]]);
    doc.receive(result(1));
    doc.edit("abc");
    vi.advanceTimersByTime(debounceMs);
    expect(send.mock.lastCall).toEqual(["scratch", 2, "abc", false]);
  });

  test("an older result never replaces a newer one", () => {
    const doc = new Document(() => Promise.resolve(), "scratch");
    doc.request();
    doc.request();
    doc.receive(result(2, { pageCount: 2, pages: [rendered(0), rendered(1)] }));
    expect(doc.loading).toBe(false);
    const newest = doc.pages.map((page) => page.url);
    doc.receive(result(1, { diagnostics: [problem] }));
    expect(doc.pages.map((page) => page.url)).toEqual(newest);
    expect(doc.diagnostics).toEqual([]);
  });

  test("problems from a result that is already out of date are not shown", () => {
    const doc = new Document(() => Promise.resolve(), "scratch");
    doc.request();
    doc.request();
    doc.receive(result(1, { diagnostics: [problem] }));
    expect(doc.loading).toBe(true);
    expect(doc.pages).toHaveLength(1);
    expect(doc.diagnostics).toEqual([]);
    doc.receive(result(2, { pages: [], diagnostics: [problem] }));
    expect(doc.diagnostics).toEqual([problem]);
  });

  test("a failed compile keeps the last good pages and only changed pages are swapped", () => {
    const doc = new Document(() => Promise.resolve(), "scratch");
    doc.request();
    doc.receive(result(1, { pageCount: 2, pages: [rendered(0), rendered(1)] }));
    const [first, second] = doc.pages.map((page) => page.url);
    doc.request();
    doc.receive(result(2, { pageCount: null, pages: [], diagnostics: [problem] }));
    expect(doc.pages.map((page) => page.url)).toEqual([first, second]);
    doc.request();
    doc.receive(result(3, { pageCount: 2, pages: [rendered(1)] }));
    expect(doc.pages[0].url).toBe(first);
    expect(doc.pages[1].url).not.toBe(second);
    doc.request();
    doc.receive(result(4, { pageCount: 1, pages: [] }));
    expect(doc.pages).toHaveLength(1);
  });

  test("opening another page starts clean and ignores what the last page still had coming", () => {
    const send = vi.fn<Send>(() => Promise.resolve());
    const doc = new Document(send, "scratch");
    doc.request();
    doc.receive(result(1));
    doc.request();
    doc.open("other", "text");
    expect(doc.pages).toEqual([]);
    expect(send.mock.lastCall).toEqual(["other", 3, "text", true]);
    doc.receive(result(2, { page: "other" }));
    expect(doc.pages).toEqual([]);
    doc.receive(result(3, { page: "other" }));
    expect(doc.pages).toHaveLength(1);
    doc.open("", "");
    expect(send).toHaveBeenCalledTimes(3);
  });
}
