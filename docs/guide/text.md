# Text

Titles, captions, speech bubbles, scores and name boxes: this chapter covers the Text tool, the
fonts it uses, the three kinds of text, and how text looks when you export.

A note on keys: AniCraft uses **Ctrl** on Windows and Linux and **⌘** (Command) on macOS, and
**Alt** is **⌥** (Option) on macOS. So **Ctrl+B** is **⌘B** on a Mac.

## Typing text

1. Pick the **Text** tool (**T**).
2. Click on the stage where the text should start, and type. **Enter** starts a new line.
3. Press **Esc**, or click somewhere else, to finish.

To change text later, click it with the Text tool, or double-click it with the **Selection**
tool (**V**). Each edit is one undo step.

Text is artwork like any other: you can move, scale, rotate and fade it, tween it, put it in a
symbol, and use it as a mask.

## Text properties

Select some text to set its look in **Properties**. With the Text tool in your hand and nothing
selected, the same rows set what new text gets. New text keeps the last style you used, as in
Animate.

| Row | What it sets |
|---|---|
| **Font** | The typeface: any font on your computer. |
| **Size** | The size in pixels, with the text colour beside it. |
| **Style** | **B** for bold and **I** for italic, then **Left**, **Centre** or **Right** alignment. |
| **Spacing** | **Letters** (letter spacing) and **Lines** (line spacing), in pixels. |
| **Width** | Turn on **Wrap** to wrap lines at a fixed width, and set the width. With Wrap off, each paragraph stays on one line. |

Bold and italic use the matching face of the font (for example, its Bold or Italic file). If a
font has no bold or italic face, AniCraft doesn't fake one.

A style applies to the whole text block. Different styles for single words in one block aren't
there yet; use a separate block for each style.

## Fonts on your computer

The **Font** menu lists the fonts installed on your computer, plus Inter, which comes with
AniCraft. There is nothing to set up: install a font in your system as usual and it shows up the
next time AniCraft starts.

### Your text looks the same everywhere

AniCraft saves the shapes of the letters with the document. So a file you send to someone looks
exactly the same on their computer, even if they don't have the font, and so do your exports.

If you edit text whose font isn't on this computer, AniCraft lays it out again with the default
font and tells you, so nothing changes behind your back.

### Missing characters

If a font doesn't have a character you type (an emoji, or a letter from another script), the
character shows as an empty box. AniCraft doesn't borrow it from another font yet, so pick a font
that covers your language.

## Static, dynamic and input text

As in Animate, text comes in three types. Choose one in **Properties › Text type**.

| Type | What it's for |
|---|---|
| **Static text** | Text that's part of the artwork: titles, captions, labels. This is what the Text tool makes. |
| **Dynamic text** | Text a script changes while the movie plays, such as a score or a timer. |
| **Input text** | A box viewers type into, such as a name or an answer. |

### Field settings

Dynamic and input text are **fields**, with a few more rows in Properties:

* **Instance**: the name scripts use for the field, for example `score_txt`. Letters, digits and
  `_` only.
* **Lines**: **Single line**, **Multiline**, **Multiline no wrap**, or (input text only)
  **Password**, which hides what the viewer types.
* **Box**: the field's width (**W**) and height (**H**) in pixels.
* **Show border**: draws a line round the field and fills its box.
* **Selectable** (dynamic text): viewers can select the text while the movie plays.
* **Max characters** (input text): the most characters a viewer may type. **0** means no limit.

### Setting and reading fields from a script

A frame script can set a dynamic field's text, add to it, and read what a viewer typed into an
input field. For example:

```actionscript
score_txt.text = "Score: " + score;
```

See [Interactivity](interactivity.md) for how to add scripts.

### Trying it out

Choose **Control › Test Movie** (**Ctrl+Enter**). Click an input field and type. You can move
the caret with the arrow keys, **Home** and **End**, select with **Shift** or the pointer, delete
with **Backspace** and **Delete**, start a new line with **Enter** in a multiline field, and press
**Tab** to go to the next field. Select All, Copy, Paste and your system's input methods work too.
Screen readers find input fields as text boxes.

Not there yet for fields: formatted (HTML) text, different styles inside one field, scripts that
change a field's colour or format, listening for each change as the viewer types, Cut,
double-clicking to select a word, a blinking caret and scrolling a field with the mouse wheel.

## Turning text into shapes

To reshape the letters themselves, select the text and choose **Modify › Break Apart**
(**Ctrl+B**). The text becomes an ordinary shape that you can edit with the Subselection tool,
recolour and morph. It is no longer editable as text, so keep a copy if you might want to change
the words.

## Text in exports

Because the letter shapes are saved with the document, text comes out exactly as you see it on
the stage in every picture export: PNG, PNG sequences, GIF, animated PNG and WebP, video, sprite
sheets, SVG and Lottie. Fields show their authored text, box and border there.

In the **HTML canvas** export, the movie plays in a web browser:

* static text, and the authored text of fields, is drawn exactly as in AniCraft;
* input fields become real text boxes over the movie, so the browser's input methods, on-screen
  keyboards on phones and tablets, and the clipboard all work;
* text that a script or the viewer changes is drawn with the fonts in the viewer's browser. If
  the viewer doesn't have your font, it uses a similar one, and the export report tells you this
  can happen.

AniCraft doesn't embed font files in the web page. Most font licences don't allow that, and
AniCraft can't check the licence for you. This only affects text that changes while the movie
plays.

See [Exporting and Publish](exporting-and-publish.md) for every format.

## Text in Animate files

When you open an Animate document, its text comes across as text you can edit, with its font,
size, colour, bold and italic, alignment, spacing, position and wrapping. Dynamic and input
fields come across as fields, with their names, boxes, borders and line types. A block whose
lines use different styles becomes one block per style, stacked as before. Fonts that aren't on
your computer use the default font, and the status bar names them. See
[Opening Animate files](opening-animate-files.md).

Not there yet: vertical text and text along a path.

## Next steps

* Make text react to the viewer: [Interactivity](interactivity.md).
* Animate a title: [Tweens](tweens.md).
