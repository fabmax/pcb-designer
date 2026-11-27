# The browser version

PCB Designer also runs in a web browser with WebGPU (a recent Chrome, Edge, Safari or
Firefox). It's experimental: the desktop app comes first.

## What works

Schematic, PCB and part editing, libraries, design checks, Gerber, assembly and
schematic PDF export, KiCad import and export, and simulation, as on the desktop.

Simulation runs ngspice inside the browser, a little slower than on the desktop. Not
yet: parts whose models need ngspice's code models (many vendor op amp models, which
use POLY sources, and the digital parts); running them reports an error.

Not in the browser (yet): the MCP server for agents,
downloading KiCad's libraries (upload library folders instead, and choose them in the
KiCad import window), recovery copies after a crash, and Paste from the Edit menu
(Cmd/Ctrl+V works).

To import a KiCad project, upload it (its folder as a `.zip`, or its files into a new
folder), then **File → Import KiCad project…** and pick its `.kicad_pro`.

## Files

The browser has no access to your files. The app keeps its own in the browser's storage
for this site: projects in `/Projects`, settings in `/config`. They stay there across
visits, until the site's data is cleared in the browser.

- **Save** and **Open** work as on the desktop, in a file dialog of the app that shows
  that storage.
- **Upload…** in a file dialog copies files from your computer into the folder shown. A
  `.zip` is unpacked: a downloaded project (a zip with the project folder in it) comes
  back as that folder. Nothing is replaced: a name in use gets a number (`Demo 2`).
- **Download** in a file dialog saves the selected file to your computer, or a folder
  as a `.zip`. To move a project to the desktop app, download its folder and unpack it.
- In the Project panel, **Download** (in the context menu of the project and of outputs)
  does the same; on the Gerber output it downloads the zip for the manufacturer.

The page opens the last project again. `?example=amp` (or `led`) in the address opens
an example instead. Leaving the page with unsaved changes asks first.
