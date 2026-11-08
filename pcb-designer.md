# PCB Designer

An app for creating electronic designs similar to tools like KiCad, EAGLE (doesn't exist anymore, is now part of Autodesk Fusion), Altium Designer, etc.

## Goals
- Pretty, modern looking GUI application for designing electronic schematics and PCB (printed circuit boards) layouts
- The app needs three functional components:
  - The schematic editor for editing the electronic circuit schematic
  - The PCB layout editor for editing the layout of the final PCB
  - The part editor for editing parts (Resistors, connectors, ICs, etc.) to be used in the schematic
- Schematic editor and PCB editor have to be tightly connected in the application
  - Changes made in the schematic should be immediately visible in the PCB editor
  - Changes made in the PCB editor only affect the physical layout, not the logical connections between parts. Hence changes made in the layout should not affect the schematic
- The part editor itself needs two components:
  - Schematic part for defining the schematic symbol of the part
  - PCB layout part for defining the layout (footprint) of the part
  - There must be an option to connect the schematic part and the footprint after designing them so that it can be defined which pins of the schematic symbol correspond to which pins in the footprint.
  - It probably makes sense to have an option to create multiple parts in a single bundle / library
- Schematics, PCB layouts and parts(-libraries) need to be saved to files (something json based probably makes sense)

## Tech stack
- We want to use rust as programming language
- The app should be multi-platform capable running on macOS, Linux and Windows
- The schematic and PCB layout editors need to be fast even for large layouts with thousands of parts. If you know a GUI framework that offers drawing functions fast enough for that, that's fine. But it might make sense to use hardware accelerated rendering here (e.g. by using some rendering framework like bevy, or even using some gaphics API like Vulkan directly)
- No particular preference for the GUI framework used to render GUI elements like buttons, menus, etc. but it must integrate well with whatever technology is chosen for the editors
- Make use of git as you like

## Scope
- For now we concentrate on the core application outlined above
- Things that are out-of-scope for now but probably become relevant in the future
  - Export of the PCB layout to gerber file format for manufacturing
  - Advanced convenience features for the editors like auto-placement of parts and auto-routing
  - Simulation of the circuit
