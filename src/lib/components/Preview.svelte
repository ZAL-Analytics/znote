<script lang="ts">
  import { untrack } from "svelte";
  import LoaderCircle from "@lucide/svelte/icons/loader-circle";
  import Minus from "@lucide/svelte/icons/minus";
  import Plus from "@lucide/svelte/icons/plus";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import { Button } from "$lib/components/ui/button";
  import type { PageView } from "$lib/state/document.svelte";

  let {
    pages,
    startZoom,
    loading,
    onrefresh,
  }: { pages: PageView[]; startZoom: number | null; loading: boolean; onrefresh: () => void } = $props();

  const cssPixelsPerPoint = 96 / 72;
  const step = 1.25;

  let zoom = $state(untrack(() => startZoom));

  const zoomLabel = $derived(zoom === null ? "Fit" : `${Math.round(zoom * 100)}%`);

  function scale(factor: number) {
    zoom = Math.min(8, Math.max(0.1, (zoom ?? 1) * factor));
  }
</script>

<div class="flex h-full flex-col">
  <div class="flex h-9 shrink-0 items-center gap-1 border-b px-2 text-xs text-muted-foreground">
    <Button variant="ghost" size="icon-xs" aria-label="Refresh the preview" title="Refresh the preview" onclick={onrefresh}>
      <RefreshCw />
    </Button>
    <span class="flex flex-1 items-center gap-1" role="status">
      {#if loading}<LoaderCircle class="size-3.5 animate-spin" /> Updating…{/if}
    </span>
    <span class="mr-1 tabular-nums" aria-live="polite">{zoomLabel}</span>
    <Button variant="ghost" size="icon-xs" aria-label="Zoom out" title="Zoom out" onclick={() => scale(1 / step)}>
      <Minus />
    </Button>
    <Button variant="ghost" size="icon-xs" aria-label="Zoom in" title="Zoom in" onclick={() => scale(step)}>
      <Plus />
    </Button>
    <Button variant="ghost" size="xs" disabled={zoom === null} onclick={() => (zoom = null)}>Fit width</Button>
  </div>

  <div class="min-h-0 flex-1 overflow-auto bg-muted p-4">
    {#if pages.length === 0}
      <div class="flex h-full items-center justify-center text-sm text-muted-foreground">
        Your pages will show here.
      </div>
    {:else}
      <div class="flex flex-col items-center gap-4 {zoom === null ? 'w-full' : 'w-max min-w-full'}">
        {#each pages as page, index (index)}
          <img
            src={page.url}
            alt="Page {index + 1}"
            class="block max-w-none border shadow-sm"
            style:width={zoom === null ? "100%" : `${page.width * cssPixelsPerPoint * zoom}px`}
            style:aspect-ratio="{page.width} / {page.height}"
          />
        {/each}
      </div>
    {/if}
  </div>
</div>
