# Design checks (ERC and DRC)

The schematic is checked as you edit (ERC); the board's design rules when you press
**Run DRC**. Findings are listed in the Messages panel (⌘3), errors before warnings, with a
tab each for the schematic, the board, the libraries and the simulation. Errors and
warnings are marked on the canvas, on the **ERC markers** layer of the schematic and the
**DRC markers** layer of the board: hide them in the layer menu to turn the markers off
(prints leave them out). Click a message to show its objects.

- **Severities:** each rule is an error, a warning or ignored: ERC rules and the pin
  conflict matrix (which pin kinds may share a net) in the Schematic setup panel (⌘8),
  DRC rules in Board setup (⌘6). Click a matrix cell to cycle it.
- **Exclusions:** right-click a finding → **Exclude** (with an optional comment). It moves
  into the collapsed *Excluded* list and no longer counts; **Include again** brings it
  back. Exclusions are saved with the project and follow the objects when they move. One
  that matches nothing any more shows as unused; **Remove unused** clears those.
- **Power flags** (⇧P): a power input fed from a connector or another passive pin needs a
  power source. Put a power flag on its net (on a wire or a pin) to say the net is
  powered.
- **Unconnected pins:** mark pins left open on purpose with a no-connect flag (Q).

While routing, the design rules are checked as you draw: see
[PCB → Routing](pcb.md#routing). DRC also flags copper whose net no longer matches the
schematic. The Gerber export refills zones and runs DRC first; violations are reported but
don't stop the export.
