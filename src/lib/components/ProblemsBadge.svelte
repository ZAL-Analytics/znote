<script lang="ts">
  import CircleCheck from "@lucide/svelte/icons/circle-check";
  import CircleX from "@lucide/svelte/icons/circle-x";
  import TriangleAlert from "@lucide/svelte/icons/triangle-alert";
  import { Button } from "$lib/components/ui/button";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu";
  import type { Diagnostic } from "$lib/ipc";

  let { diagnostics }: { diagnostics: Diagnostic[] } = $props();

  const errors = $derived(diagnostics.filter((diagnostic) => diagnostic.severity === "error").length);
  const warnings = $derived(diagnostics.length - errors);
  const label = $derived(
    diagnostics.length === 0 ? "No problems on this page" : `${errors} errors and ${warnings} warnings on this page`,
  );
</script>

<DropdownMenu.Root>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}
      <Button {...props} variant="ghost" size="sm" class="tabular-nums" aria-label={label} title={label} disabled={diagnostics.length === 0}>
        {#if diagnostics.length === 0}
          <CircleCheck />
        {:else}
          <CircleX class={errors > 0 ? "text-destructive" : ""} /> {errors}
          <TriangleAlert /> {warnings}
        {/if}
      </Button>
    {/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Content align="end" class="max-h-80 w-96 overflow-auto">
    {#each diagnostics as diagnostic}
      <DropdownMenu.Item class="items-start whitespace-pre-wrap">
        {#if diagnostic.severity === "error"}<CircleX class="text-destructive" />{:else}<TriangleAlert />{/if}
        {diagnostic.message}
      </DropdownMenu.Item>
    {/each}
  </DropdownMenu.Content>
</DropdownMenu.Root>
