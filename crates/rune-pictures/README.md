# rune-pictures

Throwaway, for [How pictures are shown in a Transcript](https://github.com/jake-tucker/shaman/issues/800). Not
shipped, not a default member.

Three looks at one fake Session of pictures, in the window the earlier prototypes chose (form `d` of
`crates/rune-composer`, whose code this crate began from and still carries), in Rune's default light and dark themes.
`src/pictures.rs` holds all of it.

The fake Session holds a picture from each place one could come from, which no look disagrees about: an Attachment
on a Prompt (one alone, and four together), a path an agent wrote in a Message (wide, tall and small), a URL it
wrote, a path that names no file, and a picture a Tool Call returned.

| | a, a viewer over the window | b, the sheet a diff opens in | c, in place |
|---|---|---|---|
| An Attachment | a thumbnail, 220 pt alone and 116 pt among others | a line, `Image 1 · PNG · 1280×800 · 233 KiB`, a small picture at its head | a thumbnail, as a |
| A Message's picture | Rune's Markdown image as it comes: its own size up to the column's width, however tall | a band at most 240 pt tall, cut from the picture's head, over a line that names it | at the column's width, cut at 480 pt until pressed, over a line: **Copy**, **Save as…**, **Open outside** |
| A press | Rune's `Preview` over the whole window: pinch or ⌘ wheel zooms, a drag pans, ← → step through the Session's pictures, Esc fits and then shuts | the sheet over the window: the Session's pictures down its side, this one on Rune's `Stage`, **Copy** and **Save as…** in its head | a cut picture shows all of itself, and an Attachment grows to its Prompt's width; nothing opens over the window |
| A picture still on its way | a frame where it will stand, a spinner in it | one row: its name and "Reading…" | Rune's own: the broken-image glyph and the alt text |
| A picture that cannot be read | the frame, saying why, with **Try again** | one row, saying why, with **Try again** | the broken-image glyph and the alt text; why is a tooltip |
| A URL's picture | a frame naming the host, with **Load picture** | one row naming the host, with **Load picture** | a link, as the TUI draws it; nothing fetches it |

Where `transcript.imagePreviews` is off, every look names each picture in a line and draws none.

## What Rune does, tried here

- **A picture is an `Image` from an `ImageCache`**, decoded off the main thread from bytes the app hands it. Rune
  fetches nothing: on a desktop a URL other than `file:` fails, so the bytes are always the app's to get.
- **An element with one side set takes the other from the picture**, and has no height until the picture decodes,
  so a row whose picture's size is not known beforehand jumps when it arrives.
- **Markdown takes a resolver and a frame** (`Markdown::images`, `Markdown::image_frame`): look a's Message
  pictures go through both, and the press that opens the viewer is put on in the frame. An image with no picture is
  Rune's broken-image glyph and its alt text, which is look c's stand-in untouched.
- **`Preview` and `Stage` both work headless** and need only the picture's size and an element that draws it. The
  preview has no place for a caption, so a's is a pill of the app's own over it. `Stage`'s three controls drew as
  dashes here, with no icons given.

## Running it

```
cargo run -p rune-pictures -- a        # or b, c
```

The strip under the window switches looks (⌥← ⌥→) and states (⌥↑ ⌥↓). Its buttons make every picture slow, stop the
Server answering, and turn previews off. The line under them says what is on show.

Every state is a scene, and `ci/snap.sh <out-dir>` takes every picture and writes `index.html`, which sets the looks
side by side state by state. `results/` holds the pictures as they stood when the ticket was worked.

## What is fake

Everything but Rune. No Server holds a picture: all of them are compiled in, and "the Server stops answering" is a
flag. **Copy**, **Save as…**, **Open outside** and **Try again** do nothing. The composer, the Sidebar and the Aside
are the composer prototype's, and a Prompt sent from here is drawn after the fake Transcript. Nothing was run on
Windows or Linux.
