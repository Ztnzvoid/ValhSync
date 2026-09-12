# ValhSync palette

Derived from Valheim's in-game UI: burnt wood, polished bone, worn brass,
northern mist. Used by the documents under `docs/` and by the egui
launcher at M5, so both read as the same product.

| Token       | Hex       | Role                                        | egui usage                    |
|-------------|-----------|---------------------------------------------|-------------------------------|
| `night`     | `#0F0D0B` | Window background                            | `visuals.panel_fill`          |
| `wood`      | `#17140F` | Code blocks, log output, text fields         | `extreme_bg_color`            |
| `panel`     | `#1E1912` | Content surface                              | `window_fill`                 |
| `leather`   | `#262017` | Callouts, inactive widgets                   | `widgets.inactive.bg_fill`    |
| `edge`      | `#4A3C27` | Strong rules, widget strokes                 | `widgets.active.bg_stroke`    |
| `edge_soft` | `#332A1D` | Hairline rules, separators                   | `widgets.noninteractive`      |
| `bone`      | `#E0D5BC` | Body text                                    | `override_text_color`         |
| `bone_dim`  | `#ADA089` | Secondary text, disabled                     | `widgets.noninteractive.fg`   |
| `gold`      | `#C7A455` | Headings, links, primary action              | `selection.bg_fill`, buttons  |
| `gold_lit`  | `#E8CD8B` | Hover, emphasis, `à vérifier` markers        | `widgets.hovered.fg_stroke`   |
| `rune`      | `#7E9AA7` | Section numbers, list markers, informational | info badges                   |
| `blood`     | `#9A3421` | Hard rules, refusals, errors                 | error text, quarantine notice |
| `moss`      | `#7E9155` | Reserved for "up to date" / success only     | "À jour" status               |

Contrast: `bone` on `night` is ~13:1, `gold` on `night` ~7.5:1, `rune` on
`night` ~6.5:1 — all clear of WCAG AA at body size.

Type: a Trajan-like display serif (Cinzel, falling back to Constantia /
Palatino) for headings, an old-style serif for body, a coding mono for paths
and logs. No blackletter: the spec has to stay readable.
