# Draw the composer's caret rather than show the terminal's cursor

The composer's caret was the terminal's own cursor, parked at the end of each frame. Every frame that repaints a cell moves that cursor, and the run loop draws every 32 ms while anything animates (ADR 0009), which is most of the time Shaman is in use. Terminals restart the cursor's blink phase when it is shown or moved, and each does so on its own terms, so no arrangement of Hide, Show and MoveTo blinked the same in two of them: hiding the cursor across each repaint stopped it jumping to the animated cells in Windows Terminal, and left it solid in Ghostty and blinking erratically in Windows Terminal.

Shaman now keeps the terminal's cursor hidden for the whole run and draws the caret itself, as a block over the cell it stands on. It blinks on the presentation clock, lit and dark for 530 ms each, begins afresh lit on every key, paste and use of the mouse so that it holds while the reader types, and stands steady in a muted colour while the terminal reports its window without the keys. Any of those acts takes the keys back, since a report of their return can go missing. The block takes the colours a list gives the row the keys are on, and those of a row they have left, rather than swapping the cell's own: a text selection is drawn swapped, and the caret has to tell apart from one it stands beside or within. Every Theme gives those roles colours of their own, one that otherwise leaves the terminal's showing through included.

The hidden cursor is still parked at the caret after each frame, since that is where a terminal places an input method's composition. The caret is drawn over the finished frame rather than as part of it, as the terminal's cursor was: what a frame draws stays a matter of the Application's state, and whether the caret is lit this instant is a matter of the clock alone.

## Considered Options

- **Keep the terminal's cursor and write less about it.** Rejected: a frame that repaints anything has to move the cursor and move it back, and whether that restarts the blink is the terminal's to decide.
- **A steady block with no blink.** Not taken: it needs no wakeups, but a caret that never blinks is what the change set out to fix in Ghostty.
- **Swap the cell's own colours, as a terminal's block cursor does.** Rejected: that is how a text selection is drawn, so a caret at the end of a selection read as one more selected cell.

## Consequences

- The reader's own cursor shape, colour and blink rate, as their terminal is configured, no longer apply inside Shaman. Only a block is drawn; a bar cannot be drawn over a cell that holds a character.
- An idle TUI showing a focused composer wakes once every 530 ms to blink the caret, a frame that repaints the one cell. Nothing wakes for a caret no frame placed, behind an overlay or in a Subagent's view, nor for one standing steady in a window without the keys. ADR 0009's account of an idle TUI's wakeups gains this one.
- Shaman asks the terminal to report focus changes (mode 1004). A terminal that reports none is taken to hold the keys always, so its caret never goes muted.
- That an input method composes at a hidden cursor is how the terminals we know behave, and has not been checked in each of them.
- Only the composer has a caret. A text field elsewhere that wants one can place it the same way.
