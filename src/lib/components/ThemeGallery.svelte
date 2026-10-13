<script lang="ts">
  import Trash2 from "@lucide/svelte/icons/trash-2";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import { themeFileUrl } from "$lib/ipc";
  import { settings } from "$lib/state/settings.svelte";
  import { theme } from "$lib/state/theme.svelte";

  const swatches = ["--background", "--primary", "--secondary", "--ring"];

  const cards = $derived([
    { id: null, name: "Default", detail: "The neutral look that comes with znote.", logo: null, colours: [] },
    ...theme.installed.map((installed) => {
      const tokens = theme.dark ? installed.dark : installed.light;
      return {
        id: installed.id,
        name: installed.name,
        detail: `Version ${installed.version} · ${installed.templates.length} templates`,
        logo: (theme.dark ? installed.logoDark : null) ?? installed.logo,
        colours: swatches.map((name) => tokens[name]).filter((colour) => colour !== undefined),
      };
    }),
  ]);
</script>

<section class="h-full overflow-auto p-6" aria-labelledby="themes-title">
  <div class="mb-1 flex items-center justify-between">
    <h2 id="themes-title" class="text-lg font-medium">Themes</h2>
    <Button variant="outline" size="sm" onclick={() => theme.import()}>Import a theme…</Button>
  </div>
  <p class="mb-4 text-sm text-muted-foreground">A theme changes how znote looks and adds its own page templates.</p>
  {#if theme.failure}<p class="mb-4 text-sm text-destructive" role="alert">{theme.failure}</p>{/if}

  <ul class="grid grid-cols-[repeat(auto-fill,minmax(16rem,1fr))] gap-4">
    {#each cards as { id, name, detail, logo, colours } (id)}
      {@const chosen = (theme.active?.id ?? null) === id}
      <li>
        <Card.Root class="h-full {chosen ? 'ring-2 ring-ring' : ''}">
          <Card.Header>
            <Card.Title>{name}</Card.Title>
            <Card.Description>{detail}</Card.Description>
          </Card.Header>
          <Card.Content class="flex flex-1 items-center justify-between gap-2">
            {#if logo}<img src={themeFileUrl(logo)} alt="" class="h-6 max-w-32" />{/if}
            <div class="flex gap-1">
              {#each colours as colour (colour)}
                <span class="size-5 rounded-full border" style:background-color={colour}></span>
              {/each}
            </div>
          </Card.Content>
          <Card.Footer class="justify-between">
            <Button
              variant={chosen ? "secondary" : "default"}
              size="sm"
              disabled={chosen}
              onclick={() => settings.set("uiTheme", id)}
            >
              {chosen ? "In use" : "Use this theme"}
            </Button>
            {#if id !== null}
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Remove the theme {name}"
                title="Remove"
                onclick={() => theme.remove(id)}
              >
                <Trash2 />
              </Button>
            {/if}
          </Card.Footer>
        </Card.Root>
      </li>
    {/each}
  </ul>
</section>
