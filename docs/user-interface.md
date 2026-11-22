# User interface and settings

## Tabs

The schematic sheets, the board, library items, the Gerber viewer and the simulation plot
open in tabs, which can be split side by side by dragging them. Projects are folders
(`<name>.pcbproj/`) of JSON files; **File → Open** (⌘O) opens one, ⌘S saves it.

## Panels

Tool windows sit in narrow icon bars on the window edges, IntelliJ style. Click an icon (or
press its shortcut) to show its panel; click it again to hide it. Each bar has an upper and
a lower group (the left bar's foot holds the bottom panels); a group shows one panel at a
time, and when both groups of a side show one they stack, with a draggable split. Drag an
icon to reorder it or move it to another group; **View → Reset panel layout** puts them
back.

| Bar | Panel | Shortcut |
| --- | --- | --- |
| left | Project: the project tree and its libraries ([Libraries](libraries.md#the-project-panel)) | ⌘1 |
| left, bottom panel | Messages: schematic problems, board problems with the DRC, library issues, the last simulation's problems, one tab each ([Design checks](checks.md)) | ⌘3 |
| right | Properties | ⌘4 |
| right | Tool settings | ⌘5 |
| right | Board setup (with DRC severities) | ⌘6 |
| right | Overview (project or library statistics) | ⌘7 |
| right | Schematic setup: ERC severities and the pin conflict matrix | ⌘8 |
| right, lower | Nets: every net with its pins, sheets and routing state; click selects it, double-click zooms to it | ⌘9 |
| right, lower | Parts: the parts of all libraries the active editor places ([Libraries](libraries.md#the-parts-panel)) | ⌘0 |
| right, lower | Simulation: analyses, Run/Stop, results ([Simulation](simulation.md)) | ⌘2 |
| left, bottom panel | Agent activity: the agent's calls this session, their arguments and results ([Agents](agents.md)) | – |

Properties, Tool settings and Overview follow the editor tab you used last. Messages has a
tab for the schematic, the board, the libraries and the simulation in its title bar; each
tab's badge shows a check when all is fine, else the number of problems (red with errors,
amber with only warnings). While Messages is hidden, its icon counts new problems (a dot
means only problems you have already seen). Which panels are open, and their sizes, are
saved with the settings.

## Settings

**Edit → Settings…** (⌘,) lists the categories on the left; type in the search field (⌘F)
to find a setting by name or keyword, or a shortcut by action or key. Changes apply at
once; each page has a **Reset to defaults**.

- **Appearance:** theme (dark or light), UI scale (80–200 %), length units (mm, mil or
  inch).
- **Layer colours:** the board layers' colours, separately for each theme (with alpha).
- **Keymap:** click `+` next to an action and press the new shortcut, × removes one.
- **Tools:** the drawing tools' style choices (corner styles, widths, sizes, layers, array
  layouts) are remembered across sessions; rotation, the next name and the routing layer
  are not. Reset forgets them.
- **Agent access:** start or stop the MCP server, its port, starting it on launch, and the
  commands and config to connect a client.
- **Libraries:** the most library tabs open at once (1–50, 10 by default), and where
  KiCad's libraries are downloaded to.
- **Simulation:** shared library when found or a separate process only; where ngspice and
  its library are (empty: found automatically); shows what's in use with its version, or
  how to install it.

These, the panel layout, the window's size and position and the recent projects are saved
in `settings.json` in the configuration folder (macOS:
`~/Library/Application Support/pcb-designer`). `PCB_DESIGNER_CONFIG=DIR` uses another
folder; `--keymap FILE.json` loads key bindings and `--mcp-port PORT` the port for one
session.

## Recovery

Unsaved changes are copied to `recovery/` in the configuration folder 5 minutes after the
first change not in the copy (the project itself is only written when you save). The copy
is removed when you save, discard the changes or quit. After a crash the next start offers
to restore it, with the changes still unsaved; **Later** asks again at the next start or
when the project is opened.

## Workspace

Each saved project remembers its workspace: open tabs and splits, the zoom and position of
each sheet, the board and each library tab, hidden layers and the routing layer. It's kept
in the project folder in `.pcb-designer/`, which ignores itself for git.
