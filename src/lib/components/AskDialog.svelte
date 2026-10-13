<script lang="ts" module>
  export type Ask = {
    title: string;
    text?: string;
    value?: string;
    action: string;
    destructive?: boolean;
    options?: string[];
    run: (value: string, option: number) => Promise<void>;
  };
</script>

<script lang="ts">
  import { untrack } from "svelte";
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Input } from "$lib/components/ui/input";
  import * as NativeSelect from "$lib/components/ui/native-select";
  import { messageOf } from "$lib/utils";

  let { ask, onclose }: { ask: Ask; onclose: () => void } = $props();

  let value = $state(untrack(() => ask.value ?? ""));
  let option = $state(0);
  let failure = $state<string | null>(null);
  let busy = $state(false);

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    try {
      await ask.run(value, option);
      onclose();
    } catch (error) {
      failure = messageOf(error);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog.Root
  open
  onOpenChange={(open) => {
    if (!open) onclose();
  }}
>
  <Dialog.Content>
    <form class="grid gap-4" onsubmit={submit}>
      <Dialog.Header>
        <Dialog.Title>{ask.title}</Dialog.Title>
        {#if ask.text}<Dialog.Description>{ask.text}</Dialog.Description>{/if}
      </Dialog.Header>
      {#if ask.value !== undefined}
        <Input bind:value aria-label="Name" aria-invalid={failure !== null} autocomplete="off" />
      {/if}
      {#if ask.options}
        <NativeSelect.Root
          aria-label="Template"
          value={String(option)}
          onchange={(event) => (option = Number(event.currentTarget.value))}
        >
          {#each ask.options as label, index (index)}
            <NativeSelect.Option value={String(index)}>{label}</NativeSelect.Option>
          {/each}
        </NativeSelect.Root>
      {/if}
      {#if failure}<p class="text-sm text-destructive" role="alert">{failure}</p>{/if}
      <Dialog.Footer>
        <Button type="button" variant="outline" onclick={onclose}>Cancel</Button>
        <Button type="submit" variant={ask.destructive ? "destructive" : "default"} disabled={busy}>
          {ask.action}
        </Button>
      </Dialog.Footer>
    </form>
  </Dialog.Content>
</Dialog.Root>
