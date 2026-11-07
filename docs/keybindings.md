# Key bindings

Everything works with the keyboard alone. The mouse is optional.

## Keyboard

| Key | Action |
|---|---|
| `↑` `↓` / `k` `j` | Move the cursor in the focused list, or scroll the viewer when it has focus |
| `PgUp` / `PgDn` | Move or scroll by 10 rows |
| `Enter` | Show the row under the cursor in the viewer right away (the viewer also follows the cursor by itself once it rests for a moment) |
| `Tab` / `Shift+Tab` | Cycle focus: Pods → Images → Services → Storage → Viewer |
| `1`–`4` | Jump to a list (in the narrow layout this also switches the visible list) |
| `←` `→` | Previous / next viewer tab (wraps) |
| `c` | Next container (pod logs) |
| `f` | Toggle follow mode (pod logs). Scrolling up also leaves follow mode |
| `z` | Zoom the viewer to full screen |
| `/` | List focused: filter the list by name. Log viewer focused: search the logs (matches the raw line, ignoring ASCII case). `Enter` keeps it, `Esc` clears it |
| `p` | Logs: switch to the previous container instance (after restarts) and back |
| `J` | Logs: show raw JSON instead of formatted lines |
| `w` | Logs: wrap long lines onto several rows (on by default) or cut them at the edge |
| `↑` `↓` then `Enter` | Viewer focused, with related resources shown: choose one and jump to it |
| `x` | Switch context (from kubeconfig; type to filter, `Enter` switches) |
| `n` | Switch namespace (type a name if listing namespaces is not allowed). With `--oci`: pick a compose project, or `(all projects)` |
| `r` | Refresh now (lists refresh every 10s anyway) |
| `?` | Toggle help |
| `Esc` | Close help, or leave zoom |
| `q` / `Ctrl+C` | Quit |

## Mouse

| Action | Effect |
|---|---|
| Click a list row | Focus the list and show the row in the viewer |
| Click a list pane | Focus it |
| Click a list tab (narrow layout) | Switch to that list |
| Click a viewer tab | Switch tabs |
| Click the viewer | Focus the viewer |
| Wheel over a list | Move its cursor by 3 rows (focus stays where it was) |
| Wheel over the viewer | Scroll by 3 rows |

Mouse reporting depends on the terminal emulator. If clicks do nothing, check that mouse reporting is enabled in your terminal's settings.

## Layouts

- **Wide**, at least 120×30: all four lists stacked on the left, with the viewer on the right.
- **Narrow**, at least 60×16: list tabs at the top, one list, and the viewer below it.
- **Smaller than 60×16**: a "terminal too small" message.
