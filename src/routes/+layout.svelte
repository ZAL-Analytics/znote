<script lang="ts">
  import "../app.css";
  import { connect } from "$lib/state/document.svelte";
  import { settings } from "$lib/state/settings.svelte";
  import { loadThemes, theme } from "$lib/state/theme.svelte";
  import { start } from "$lib/state/workspace.svelte";

  let { children } = $props();

  const osPrefersDark = matchMedia("(prefers-color-scheme: dark)").matches;

  $effect(() => {
    const dark = settings.loaded ? settings.values.theme === "dark" : osPrefersDark;
    document.documentElement.classList.toggle("dark", dark);
  });

  $effect(() => {
    const root = document.documentElement;
    const style = theme.style;
    for (const [name, value] of Object.entries(style)) root.style.setProperty(name, value);
    return () => Object.keys(style).forEach((name) => root.style.removeProperty(name));
  });

  $effect(() => {
    const faces = theme.fontFaces.map(({ family, source, weight, style }) => new FontFace(family, source, { weight, style }));
    faces.forEach((face) => document.fonts.add(face));
    return () => faces.forEach((face) => document.fonts.delete(face));
  });

  connect();
  start();
  loadThemes();
</script>

{@render children()}
