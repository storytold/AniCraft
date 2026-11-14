# Using this guide

This guide comes with AniCraft and matches the version you have. It opens in your web browser and
works without an internet connection. This chapter shows how to find things in it quickly: by
searching, and by asking AniCraft for help on whatever you are working on.

On macOS, read **⌘** wherever this chapter says **Ctrl**.

## Opening the guide

* **Help › User Guide** opens the contents page.
* **Help › Help for This** (**F1**) opens it at the section for what you are working on (see
  [below](#help-for-what-youre-working-on)).
* **Help › Search the User Guide** opens the search page, ready for you to type.

All three are also in the command palette (**Ctrl+K**).

## Searching the guide

Every page has a **Search** field at the top. Type a word or two and press **Enter**; the search
page lists the sections that match, best first. On the search page itself the results update as
you type.

Each result shows the section's heading, the chapter it is in and a short passage with your words
highlighted. Click a result to go straight to that section.

* **Every word must match.** `onion skin` finds the sections about onion skinning; `onion gif`
  finds only the ones that mention both.
* **Words match their longer forms.** `tween` finds "tweens" and "tweening", and `masks` finds
  "mask".
* **Headings count most.** A section whose heading has your words comes before one that only
  mentions them.
* Small words such as "how", "do", "the" and "a" are skipped, so you can type a question:
  `how do I add a camera`.

### With the keyboard

| Key | What it does |
|---|---|
| **/** | Goes to the search field from anywhere in the guide. |
| **↓** and **↑** | Move from the field through the results, and back. |
| **Enter** | Opens the first result (or the result you are on). |
| **Esc** | Clears the search; in the results, goes back to the field. |

Screen readers announce how many results there are as you type.

## Help for what you're working on

Press **F1** in AniCraft and the guide opens at the section for what you are working on. AniCraft
decides what that is in this order:

| What | Opens |
|---|---|
| A dialog that is waiting for an answer, such as Convert to Symbol or a question before a command | That dialog, or the command it is asking about. |
| The command palette | The command or tool you have chosen in it. |
| The window or panel under the pointer | That window or panel: for example **Publish Settings**, the **Library** or the **Timeline**. |
| A tool's button under the pointer, or with keyboard focus | That tool. |
| The stage | The tool in your hand. Press **F1** with the Bone tool and the guide opens at building an armature. |
| The panel with keyboard focus (**Ctrl+F6** moves between them) | That panel. |
| Anything else | The window in front, or else the tool in your hand. |

**F1** works even while you are typing in a field or a dialog is open. The status bar names the
section that opened.

**Help › Help for This** does the same from the menu. Because the pointer is on the menu then, it
goes by the window in front, the focused panel or the tool in your hand.

### From a panel's menu

Every panel's menu (the **☰** button at the end of its tab row, or a right-click on its tab) has
**? Help for** the panel at its end: **? Help for Library**, **? Help for Swatches**, and so on.

### In the command palette

In the command palette (**Ctrl+K**), choose a command with the arrow keys and press **F1** to
read about it before you run it. The palette stays open.

**F1** used to open the keyboard shortcut editor. That is now **Ctrl+Alt+Shift+K**, as in
Animate, or **Help › Keyboard Shortcuts**. See
[Preferences, shortcuts, accessibility and languages](preferences-shortcuts-accessibility.md#keyboard-shortcuts).

## Searching from the command line

`anicraft-cli guide search` searches the same guide from a terminal and prints the same results,
in the same order, as the search page:

```sh
anicraft-cli guide search onion skin
anicraft-cli guide search export gif --limit 3 --json yes
```

See [Working headless](working-headless.md#this-guide-as-a-web-page).

## Printing the guide

Use your browser's **Print** to print a chapter, or to save it as a PDF (choose **Save as PDF**
as the printer). The chapter list, the search field and the previous and next links are left out
of the printout.

## In the web version

In AniCraft in a web browser, the guide is served next to the editor and opens in a new tab.
Search and **Help › Help for This** work there too. **F1** does as well, but some browsers also
open their own help page when you press it; use the menu item if yours does.

## Not there yet

* The guide in other languages. The interface can be translated already; the guide is English
  for now.
* A PDF edition of the whole guide in one file. For now, print each chapter from your browser.
* Help for a single field or menu item. **F1** goes by the window, panel or tool.
