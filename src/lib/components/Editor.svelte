<script lang="ts">
  import { untrack } from "svelte";
  import { minimalSetup } from "codemirror";
  import { HighlightStyle, LanguageSupport, syntaxHighlighting } from "@codemirror/language";
  import { setDiagnostics } from "@codemirror/lint";
  import { Compartment } from "@codemirror/state";
  import { EditorView, lineNumbers } from "@codemirror/view";
  import { tags } from "@lezer/highlight";
  import { typst_lezer } from "codemirror-lang-typst/lezer";
  import type { Diagnostic } from "$lib/ipc";

  let {
    text,
    diagnostics,
    fontSize,
    wordWrap,
    onchange,
  }: {
    text: string;
    diagnostics: Diagnostic[];
    fontSize: number;
    wordWrap: boolean;
    onchange: (text: string) => void;
  } = $props();

  let host: HTMLDivElement;
  let view: EditorView | undefined;

  const look = new Compartment();

  const colours = EditorView.theme({
    "&": { height: "100%", color: "var(--foreground)", backgroundColor: "var(--background)" },
    "&.cm-focused": { outline: "2px solid var(--ring)", outlineOffset: "-2px" },
    ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "1.6" },
    ".cm-content": { caretColor: "var(--foreground)" },
    ".cm-cursor": { borderLeftColor: "var(--foreground)" },
    ".cm-gutters": {
      color: "var(--muted-foreground)",
      backgroundColor: "var(--background)",
      borderRight: "1px solid var(--border)",
    },
    "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection": {
      backgroundColor: "var(--accent)",
    },
    ".cm-tooltip": {
      color: "var(--popover-foreground)",
      backgroundColor: "var(--popover)",
      border: "1px solid var(--border)",
    },
    ".cm-diagnostic-error": { borderLeftColor: "var(--destructive)" },
    ".cm-diagnostic-warning": { borderLeftColor: "var(--chart-4)" },
    ".cm-lintRange-error": {
      backgroundImage: "none",
      textDecoration: "underline wavy var(--destructive)",
    },
    ".cm-lintRange-warning": {
      backgroundImage: "none",
      textDecoration: "underline wavy var(--chart-4)",
    },
  });

  const highlighting = HighlightStyle.define([
    { tag: tags.comment, color: "var(--muted-foreground)", fontStyle: "italic" },
    { tag: tags.heading, fontWeight: "bold", color: "var(--chart-3)" },
    { tag: tags.strong, fontWeight: "bold" },
    { tag: tags.emphasis, fontStyle: "italic" },
    { tag: tags.link, textDecoration: "underline" },
    { tag: [tags.keyword, tags.bool], color: "var(--chart-1)" },
    { tag: [tags.string, tags.monospace], color: "var(--chart-2)" },
    { tag: tags.number, color: "var(--chart-5)" },
    { tag: [tags.function(tags.variableName), tags.propertyName], color: "var(--chart-3)" },
    { tag: tags.escape, color: "var(--chart-5)" },
    { tag: [tags.punctuation, tags.operator, tags.paren], color: "var(--muted-foreground)" },
    { tag: tags.invalid, color: "var(--destructive)" },
  ]);

  $effect(() => {
    const created = new EditorView({
      parent: host,
      doc: untrack(() => text),
      extensions: [
        minimalSetup,
        lineNumbers(),
        look.of([]),
        new LanguageSupport(typst_lezer().language),
        syntaxHighlighting(highlighting),
        colours,
        EditorView.contentAttributes.of({ "aria-label": "Typst source" }),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) onchange(update.state.doc.toString());
        }),
      ],
    });
    view = created;
    return () => created.destroy();
  });

  $effect(() => {
    view?.dispatch({
      effects: look.reconfigure([
        wordWrap ? EditorView.lineWrapping : [],
        EditorView.theme({ "&": { fontSize: `${fontSize}px` } }),
      ]),
    });
  });

  $effect(() => {
    if (!view) return;
    const length = view.state.doc.length;
    view.dispatch(
      setDiagnostics(
        view.state,
        diagnostics.map(({ severity, message, from, to }) => ({
          severity,
          message,
          from: Math.min(from, length),
          to: Math.min(Math.max(to, from), length),
        })),
      ),
    );
  });
</script>

<div bind:this={host} class="h-full overflow-hidden"></div>
