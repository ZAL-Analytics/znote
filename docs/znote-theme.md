# The `.znote-theme` format

A `.znote-theme` file rebrands znote: colours, logo, font and document templates. Users import it
under Settings > Themes > Import. This page is for people who make one.

## Layout

A `.znote-theme` is a zip archive with a different extension:

```
theme.json     required
ui/            logos and fonts for the app itself
typst/         a normal Typst package: typst.toml, lib.typ, assets/, fonts/ ...
```

Both `ui` and `typst` are optional, so one file can be a UI theme, a set of templates, or both.
Zip the three entries themselves, not a folder that contains them.

## `theme.json`

```json
{
  "format": 1,
  "id": "acme",
  "name": "Acme",
  "version": "0.1.0",
  "ui": {
    "logo": "ui/logo.svg",
    "logoDark": "ui/logo-white.svg",
    "font": { "family": "Acme Sans", "files": ["typst/fonts/AcmeSans-Regular.ttf"] },
    "light": { "--primary": "#0F172A", "--accent": "#2563EB", "--background": "#FFFFFF" },
    "dark": { "--primary": "#2563EB", "--background": "#0F172A" }
  },
  "typst": {
    "templates": [
      {
        "label": "Report",
        "kind": "document",
        "snippet": "#import \"@themes/acme:0.1.0\": *\n#show: report.with(title: \"Title\")\n"
      }
    ]
  }
}
```

| Field | Rules |
|---|---|
| `format` | Always `1`. |
| `id` | Lowercase letters, digits and dashes, starting with a letter. Importing a theme replaces the installed theme with the same id. |
| `name` | Shown in Settings. Letters, digits, spaces and `- _ . &`, at most 60 characters. |
| `version` | Three numbers, like `0.1.0`. |
| `ui.logo`, `ui.logoDark` | Shown in the title bar. `svg`, `png`, `jpg` or `jpeg`. The dark logo is used in dark mode and falls back to `logo`. |
| `ui.font` | `family` is the name the app uses; `files` are `ttf` or `otf`. List one file per weight and style; the weight and style are read from each file. |
| `ui.light`, `ui.dark` | Colour settings for light and dark mode. Anything left out keeps the default. |
| `typst.templates` | Entries for the template picker. `kind` is `document`, `slides` or `poster`. |

Paths use `/`, are relative to the archive root and must point to a file in the archive.

### Colour settings

Only these names are accepted:

```
--background  --foreground  --card  --card-foreground  --popover  --popover-foreground
--primary  --primary-foreground  --secondary  --secondary-foreground  --muted  --muted-foreground
--accent  --accent-foreground  --destructive  --border  --input  --ring
--chart-1  --chart-2  --chart-3  --chart-4  --chart-5
--sidebar  --sidebar-foreground  --sidebar-primary  --sidebar-primary-foreground
--sidebar-accent  --sidebar-accent-foreground  --sidebar-border  --sidebar-ring
--radius
```

A colour is a hex value (`#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`) or one of `rgb()`, `rgba()`,
`hsl()`, `hsla()`, `oklch()`, `oklab()` with numbers only, such as `oklch(0.5 0.1 240 / 50%)`.
`--radius` is a number with an optional `px`, `rem`, `em` or `%`. Colour names, `var()`, `url()`
and anything else are refused.

The editor colours its syntax with the chart settings: `--chart-1` keywords, `--chart-2` strings,
`--chart-3` headings and function names, `--chart-4` warnings, `--chart-5` numbers.

## The Typst package

Everything under `typst/` is installed as the package `@themes/<id>:<version>`, so a page imports
it with `#import "@themes/acme:0.1.0": *`. Fonts anywhere in the theme are available to Typst by
their family name.

A template's `snippet` is the Typst that the picker puts at the top of the page, between two
marker lines:

```typ
// znote:template acme/Report
#import "@themes/acme:0.1.0": *
#show: report.with(title: "Title")
// znote:end
```

Picking another template replaces that block and leaves the rest of the page alone.

## Limits

- The file is at most 50 MB, 200 MB unpacked, 2000 entries.
- Every entry must stay inside the theme folder; names with `..`, `\` or `:` are refused.
- A theme that breaks any rule on this page is not imported, and the app says which rule.
