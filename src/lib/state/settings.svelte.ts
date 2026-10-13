import { readSettings, writeSetting } from "$lib/ipc";

export type Settings = {
  workspace: string | null;
  lastPage: string | null;
  fontSize: number;
  wordWrap: boolean;
  autosave: boolean;
  zoom: number | null;
  theme: "light" | "dark";
  uiTheme: string | null;
};

type Persist = (key: string, value: unknown) => Promise<void>;

export const fontSizes = { min: 8, max: 32 };

export const defaults: Settings = {
  workspace: null,
  lastPage: null,
  fontSize: 14,
  wordWrap: true,
  autosave: true,
  zoom: null,
  theme: "light",
  uiTheme: null,
};

export function merged(stored: Record<string, unknown>, osPrefersDark: boolean): Settings {
  const text = (value: unknown) => (typeof value === "string" ? value : null);
  const flag = (value: unknown, fallback: boolean) => (typeof value === "boolean" ? value : fallback);
  const { fontSize, zoom, theme } = stored;
  return {
    workspace: text(stored.workspace),
    lastPage: text(stored.lastPage),
    fontSize:
      typeof fontSize === "number" && fontSize >= fontSizes.min && fontSize <= fontSizes.max
        ? fontSize
        : defaults.fontSize,
    wordWrap: flag(stored.wordWrap, defaults.wordWrap),
    autosave: flag(stored.autosave, defaults.autosave),
    zoom: typeof zoom === "number" && zoom > 0 ? zoom : null,
    theme: theme === "light" || theme === "dark" ? theme : osPrefersDark ? "dark" : "light",
    uiTheme: text(stored.uiTheme),
  };
}

export class SettingsState {
  values = $state<Settings>({ ...defaults });
  loaded = $state(false);
  failure = $state<string | null>(null);

  constructor(private persist: Persist) {}

  async load(read: () => Promise<Record<string, unknown>>, osPrefersDark: boolean) {
    this.values = merged(await read(), osPrefersDark);
    this.loaded = true;
  }

  set<Key extends keyof Settings>(key: Key, value: Settings[Key]) {
    if (this.values[key] === value) return;
    this.values[key] = value;
    this.persist(key, value).then(
      () => (this.failure = null),
      (error: unknown) => (this.failure = String(error)),
    );
  }
}

export const settings = new SettingsState(writeSetting);

export const loadSettings = () =>
  settings.load(readSettings, matchMedia("(prefers-color-scheme: dark)").matches);

if (import.meta.vitest) {
  const { test, expect, vi } = import.meta.vitest;

  test("saved settings win, and anything missing or wrong falls back to the default", () => {
    const stored = { workspace: "C:\\notes", fontSize: 18, wordWrap: false, zoom: 1.5, theme: "light", uiTheme: "acme" };
    expect(merged(stored, true)).toEqual({ ...defaults, ...stored });
    expect(merged({ fontSize: "huge", wordWrap: 1, zoom: -1, theme: "pink", workspace: 7, uiTheme: 3 }, false)).toEqual(defaults);
    expect(merged({ fontSize: 400 }, false).fontSize).toBe(defaults.fontSize);
  });

  test("with no saved theme the OS decides", () => {
    expect(merged({}, true).theme).toBe("dark");
    expect(merged({}, false).theme).toBe("light");
    expect(merged({ theme: "light" }, true).theme).toBe("light");
  });

  test("a changed setting is stored once and an unchanged one is not", () => {
    const persist = vi.fn<Persist>(() => Promise.resolve());
    const state = new SettingsState(persist);
    state.set("fontSize", 20);
    state.set("fontSize", 20);
    expect(state.values.fontSize).toBe(20);
    expect(persist.mock.calls).toEqual([["fontSize", 20]]);
  });
}
