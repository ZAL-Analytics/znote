<script lang="ts">
  import LayoutTemplate from "@lucide/svelte/icons/layout-template";
  import { Button } from "$lib/components/ui/button";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu";
  import { theme, withTemplate } from "$lib/state/theme.svelte";
  import { workspace } from "$lib/state/workspace.svelte";
</script>

<DropdownMenu.Root>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}
      <Button {...props} variant="ghost" size="sm" title="Use a template for this page">
        <LayoutTemplate /> Template
      </Button>
    {/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Content align="start">
    {#each theme.templates as template (template.id)}
      <DropdownMenu.Item onSelect={() => workspace.rewrite((text) => withTemplate(text, template))}>
        {template.label}
      </DropdownMenu.Item>
    {/each}
  </DropdownMenu.Content>
</DropdownMenu.Root>
