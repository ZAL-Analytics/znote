<script lang="ts">
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";
  import { defaultWorkspace, pickFolder } from "$lib/ipc";
  import { workspace } from "$lib/state/workspace.svelte";

  const use = (folder: string) => workspace.attempt(() => workspace.useFolder(folder, true));

  function choose() {
    return workspace.attempt(async () => {
      const folder = await pickFolder();
      if (folder !== null) await workspace.useFolder(folder, true);
    });
  }
</script>

<Dialog.Root open>
  <Dialog.Content showCloseButton={false} escapeKeydownBehavior="ignore" interactOutsideBehavior="ignore">
    <Dialog.Header>
      <Dialog.Title>Welcome to znote</Dialog.Title>
      <Dialog.Description>
        Your pages are kept as ordinary files in one folder. You can change the folder later in Settings.
      </Dialog.Description>
    </Dialog.Header>
    {#await defaultWorkspace() then folder}
      <p class="rounded-md bg-muted px-3 py-2 text-xs break-all">{folder}</p>
      {#if workspace.failure}<p class="text-sm text-destructive" role="alert">{workspace.failure}</p>{/if}
      <Dialog.Footer>
        <Button variant="outline" onclick={choose}>Choose another folder…</Button>
        <Button onclick={() => use(folder)}>Use this folder</Button>
      </Dialog.Footer>
    {:catch}
      {#if workspace.failure}<p class="text-sm text-destructive" role="alert">{workspace.failure}</p>{/if}
      <Dialog.Footer>
        <Button onclick={choose}>Choose a folder…</Button>
      </Dialog.Footer>
    {/await}
  </Dialog.Content>
</Dialog.Root>
