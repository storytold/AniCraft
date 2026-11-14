# Command reference

Everything you can do in AniCraft is a command: the menus, the shortcuts, the command palette (**Ctrl+K**, macOS **⌘K**), `anicraft-cli` and AI assistants over MCP all run the same 349 commands, listed here by menu. The id is what scripts and [working headless](../working-headless.md) use; the parameters are what a command takes as JSON. Commands that change the document are one undo step each. Your own shortcuts (**F1**) replace the defaults shown here; see the [shortcut reference](shortcuts.md).

This page is generated from the command registry, so it always matches the version of AniCraft it came with.

## File

- **New** `file.new` · **Ctrl+N** (macOS **⌘N**)
  Parameters: `{width?, height?, fps?}`
- **New from Sample** `file.new_sample`
  Parameters: `{which?: demo (default) | interactive (a menu with a Play button, for Test Movie)}`
- **New from Template…** `file.new_from_template`
  Parameters: `{template? (a built-in template's id, from file.templates) | path? (a template file: any .anicraft opens as a new, untitled document)}`
- **Open…** `file.open` · **Ctrl+O** (macOS **⌘O**)
  Parameters: `{path, unsaved? (opens it as unsaved work: a restored copy), untitled? (with no file name, so Save asks for one)}`
- **Save** `file.save` · **Ctrl+S** (macOS **⌘S**)
  Parameters: `{path?, confirm?}`
- **Save As…** `file.save_as` · **Ctrl+Shift+S** (macOS **⇧⌘S**)
  Parameters: `{path, confirm?}`
- **Save as Template…** `file.save_as_template`
  Parameters: `{folder (the user templates folder), name, category? (default “My Templates”), confirm? (to replace a template of that name)} → {path, thumbnail}: the document as it is, kept as <folder>/<category>/<name>.anicraft with a picture of its first frame; the open document is unchanged`
- **Export Frame as PNG…** `file.export_png` · File › Export
  Parameters: `{path, frame? (in the current scene), scale?}`
- **Import to Stage…** `file.import` · File › Import · **Ctrl+R** (macOS **⌘R**) · changes the document
  Parameters: `{path (.svg, .png, .jpg; or a .wav, .ogg or .mp3 sound: onto a new layer at the playhead), x?, y? (top-left on the stage; default: centred), cx?, cy? (centred on that point instead), layer? (new (default) | active: a one-layer drawing goes into the active layer's keyframe at the playhead)}`
- **Export Frame as SVG…** `file.export_svg` · File › Export
  Parameters: `{path, frame? (in the current scene), confirm?}`
- **Export Video…** `file.export_video` · File › Export
  Parameters: `{path (.mp4 or .webm), from?, to? (movie frames), scale?, audio? (default true: the movie's sounds, Opus in WebM, AAC in MP4), ffmpeg? (path; else ANICRAFT_FFMPEG, else ffmpeg on the PATH), confirm?}`
- **Export Sprite Sheet…** `file.export_sprite_sheet` · File › Export
  Parameters: `{path (.png; the frame map goes beside it as .json), from?, to? (movie frames), scale?, columns?, padding?, background?, confirm?}`
- **Export PNG Sequence…** `file.export_png_sequence` · File › Export
  Parameters: `{dir, from?, to? (movie frames: every scene in order), scale?}`
- **Export Animated GIF…** `file.export_gif` · File › Export
  Parameters: `{path, from?, to? (movie frames: every scene in order), scale?, loop?, confirm?}`
- **Export Animated PNG…** `file.export_apng` · File › Export
  Parameters: `{path (.png or .apng), from?, to? (movie frames: every scene in order), scale?, loop? (true (default): forever; false: once; or how many times it plays), background? (default false: transparent), confirm?}`
- **Export Animated WebP…** `file.export_webp` · File › Export
  Parameters: `{path (.webp; lossless), from?, to? (movie frames: every scene in order), scale?, loop? (true (default): forever; false: once; or how many times it plays), background? (default false: transparent), confirm?}`
- **Export Lottie Animation…** `file.export_lottie` · File › Export
  Parameters: `{path, from?, to? (movie frames: every scene in order), background? (default false: transparent), confirm?}`
- **Export HTML Canvas…** `file.export_html` · File › Export
  Parameters: `{path (.html: one self-contained page), title? (default: the file name), sound? (default true: embed the movie's sounds), fit? (default true: scale to the browser window), confirm?}`
- **Publish Settings…** `publish.settings_get` · **Ctrl+Shift+F12** (macOS **⇧⌘F12**)
  Parameters: `{} (the document's publish profile, and where each format would go)`
- **Publish** `file.publish` · **Shift+F12** (macOS **⇧F12**)
  Parameters: `{formats? ([format, …]: only these; default: the ones Publish Settings has on), confirm? (to replace files already there)}`
- **Recover Autosaved Documents…** `file.recovery_list`
  Parameters: `{dir (the recovery folder)}`
- **Import to Library…** `file.import_to_library` · File › Import · changes the document
  Parameters: `{path (.wav, .ogg or .mp3 sound; .png or .jpg image; .svg drawing: a graphic symbol named after the file holding its shapes), name?}`
- **Export Audio…** `file.export_audio` · File › Export
  Parameters: `{path (.wav), from?, to? (movie frames), rate? (Hz, default 44100), bits? (16, 24 or 32 = float; default 16), confirm?}`

## Edit

- **Undo** `edit.undo` · **Ctrl+Z** (macOS **⌘Z**)
  Parameters: `{}`
- **Redo** `edit.redo` · **Ctrl+Y** (macOS **⌘Y**)
  Parameters: `{}`
- **Cut Frames** `timeline.cut_frames` · Edit › Timeline · **Ctrl+Alt+X** (macOS **⌥⌘X**) · changes the document
  Parameters: `{layer? | layers?: [..], frame?, count?}`
- **Copy Frames** `timeline.copy_frames` · Edit › Timeline · **Ctrl+Alt+C** (macOS **⌥⌘C**)
  Parameters: `{layer? | layers?: [..], frame?, count?}`
- **Paste Frames** `timeline.paste_frames` · Edit › Timeline · **Ctrl+Alt+V** (macOS **⌥⌘V**) · changes the document
  Parameters: `{layer?, frame?} (top layer and first frame; default: active layer, playhead)`
- **Select All** `edit.select_all` · **Ctrl+A** (macOS **⌘A**)
  Parameters: `{}`
- **Deselect All** `edit.deselect` · **Ctrl+Shift+A** (macOS **⇧⌘A**)
  Parameters: `{}`
- **Clear** `edit.delete` · **Delete** (macOS **⌫**) · changes the document
  Parameters: `{} (on every layer with selected items)`
- **Duplicate** `edit.duplicate` · **Ctrl+D** (macOS **⌘D**) · changes the document
  Parameters: `{dx?, dy?} (each copy on its original's layer)`
- **Cut** `edit.cut` · **Ctrl+X** (macOS **⌘X**) · changes the document
  Parameters: `{}`
- **Copy** `edit.copy` · **Ctrl+C** (macOS **⌘C**)
  Parameters: `{} (the selection on every layer, each layer's artwork kept apart)`
- **Paste in Center** `edit.paste` · **Ctrl+V** (macOS **⌘V**) · changes the document
  Parameters: `{x?, y?} (centre of the pasted artwork; default: offset 10 px). Artwork copied from one layer lands on the active layer; from several, back on those layers when they are all here, else on the active layer and the ones below it, creating missing ones`
- **Paste in Place** `edit.paste_in_place` · **Ctrl+Shift+V** (macOS **⇧⌘V**) · changes the document
  Parameters: `{} (layers as edit.paste)`
- **Edit Symbols** `symbol.edit` · **Ctrl+E** (macOS **⌘E**)
  Parameters: ``{id?: instance element id, symbol?: library symbol id} opens a symbol's timeline for editing: the instance `id` (default: the selected instance) in place, or a Library symbol at the stage's centre. A group (ADR 0073) or a drawing object (ADR 0080) opens in place the same way: inside, its raw shapes merge only with each other, and a drawing object closes as one object again (a group of its pieces when an edit left several). Every command then works inside it. With nothing to open while a symbol is open, goes back to the document (Animate's Ctrl+E)``
- **Edit Document** `symbol.exit`
  Parameters: `{all?: bool} closes the open symbol (default: one level; all: back to the scene), selecting the instance it was opened from`
- **Find and Replace** `find.search` · **Ctrl+F** (macOS **⌘F**)
  Parameters: `{kind: text | font | color | symbol | sound | bitmap, text? (kind text: what to find), match_case?, whole_word?, regex? (a pattern; linear-time engine, size-limited), font? (family), color? (#rrggbb; alpha is ignored), symbol? | sound? | bitmap?: library id, in?: [text, labels, layers, instances, scripts] (kind text) | [fills, strokes, text] (kind color) (default: all), scope?: scene (the current scene and the symbols it uses; default) | document, offset?, limit? (default 500, at most 10000)} → {hits: [{kind, field, scene | symbol, layer, layer_name, frame (keyframe start; null on a layer name), element?, start, end (characters, text only), found, context}], total, truncated}`

## View

- **Snap to Objects** `draw.snap_to_objects` · View › Snapping · **Ctrl+Shift+/** (macOS **⇧⌘/**)
  Parameters: ``{on?: bool} (default: toggle) dragged points and selections, and the points the Line, Rectangle, Oval, PolyStar and Pen tools draw, snap to points, lines and fill edges (while it is off, the `snap` params of element.translate, raw.move_point and draw.snap only snap to guides, the grid and pixels, as View › Guides, Grid and Snapping say)``
- **Rulers** `view.rulers` · **Ctrl+Alt+Shift+R** (macOS **⌥⇧⌘R**)
  Parameters: `{on?: bool} (default: toggle) rulers along the stage's top and left edges, in stage pixels from its top-left corner; drag from one to make a guide`
- **Show Grid** `view.grid_show` · View › Grid · **Ctrl+'** (macOS **⌘'**)
  Parameters: `{on?: bool} (default: toggle) the grid on the stage (never in pictures or exports)`
- **Snap to Grid** `view.grid_snap` · View › Grid · **Ctrl+Shift+'** (macOS **⇧⌘'**)
  Parameters: `{on?: bool} (default: toggle) points being drawn, dragged or moved snap to the grid's lines (shown or not)`
- **Edit Grid…** `view.grid_set` · View › Grid · **Ctrl+Alt+G** (macOS **⌥⌘G**) · changes the document
  Parameters: `{spacing?, spacing_x?, spacing_y? (stage pixels, 1..=4096), color?: "#rrggbb[aa]", over_objects?: bool, show?: bool, snap?: bool, accuracy?: "close"|"normal"|"distant"|"always"} sets the grid; spacing and colour are saved with the document (one undo step), the rest is the editor's; returns everything as view.aids does`
- **Show Guides** `view.guides_show` · View › Guides · **Ctrl+;** (macOS **⌘;**)
  Parameters: `{on?: bool} (default: toggle) this scene's guides on the stage (never in pictures or exports)`
- **Lock Guides** `view.guides_lock` · View › Guides · **Ctrl+Alt+;** (macOS **⌥⌘;**)
  Parameters: `{on?: bool} (default: toggle) locked guides can't be dragged, moved or deleted one by one (new ones can still be added, and Clear Guides clears them)`
- **Snap to Guides** `view.guides_snap` · View › Guides · **Ctrl+Shift+;** (macOS **⇧⌘;**)
  Parameters: `{on?: bool} (default: toggle) points being drawn, dragged or moved snap to the guides`
- **Edit Guides…** `view.guides_set` · View › Guides · **Ctrl+Alt+Shift+G** (macOS **⌥⇧⌘G**) · changes the document
  Parameters: `{color?: "#rrggbb[aa]", show?: bool, lock?: bool, snap?: bool, accuracy?: "close"|"normal"|"distant"} sets the guides; the colour is saved with the document (one undo step), the rest is the editor's; returns everything as view.aids does`
- **Add Guide…** `view.guide_add` · View › Guides · changes the document
  Parameters: ``{axis: "horizontal"|"vertical", at} adds a guide to this scene at `at` stage pixels (y for a horizontal guide, x for a vertical one); returns {index, guides}``
- **Clear Guides** `view.guides_clear` · View › Guides · changes the document
  Parameters: `{} removes every guide from this scene (undoable)`
- **Snap to Pixels** `view.snap_pixels` · View › Snapping
  Parameters: `{on?: bool} (default: toggle) points being drawn, dragged or moved go to whole stage pixels where nothing else snaps them`

## Insert

- **Frame** `timeline.insert_frame` · Insert › Timeline · **F5** · changes the document
  Parameters: `{layer? | layers?: [..] | all_layers?: true (every layer with frames at or after the frame, as F5 with no frame selection; the active layer when none has), frame?, count?, advance?: true (the playhead moves onto the last frame inserted, as the key does in the editor)}`
- **Remove Frame** `timeline.remove_frame` · Insert › Timeline · **Shift+F5** (macOS **⇧F5**) · changes the document
  Parameters: `{layer? | layers?: [..], frame?, count?}`
- **Keyframe** `timeline.insert_keyframe` · Insert › Timeline · **F6** · changes the document
  Parameters: `{layer? | layers?: [..], frame?, advance?: true (on a keyframe the new key goes on the next frame, and the playhead moves onto it, as the key does in the editor)}`
- **Blank Keyframe** `timeline.insert_blank_keyframe` · Insert › Timeline · **F7** · changes the document
  Parameters: `{layer? | layers?: [..], frame?, advance?: true (on a keyframe the blank key goes on the next frame instead of clearing it, and the playhead moves onto it)}`
- **Clear Keyframe** `timeline.clear_keyframe` · Insert › Timeline · **Shift+F6** (macOS **⇧F6**) · changes the document
  Parameters: `{layer? | layers?: [..], frame?, count?}`
- **Create Motion Tween** `timeline.create_motion_tween` · changes the document
  Parameters: `{layer?, frame?} (several objects on the key become one graphic symbol; a one-frame span grows to a second)`
- **Create Classic Tween** `timeline.create_tween` · changes the document
  Parameters: `{layer?, frame?, easing? (as timeline.set_easing takes it)}`
- **Create Shape Tween** `timeline.create_shape_tween` · changes the document
  Parameters: `{layer?, frame?, easing? (as timeline.set_easing takes it)}`
- **Remove Tween** `timeline.remove_tween` · changes the document
  Parameters: `{layer?, frame?} (a motion tween leaves its object as it is on the first frame)`
- **New Layer** `layer.new` · Insert › Timeline · changes the document
  Parameters: `{name?}`
- **New Folder** `layer.new_folder` · Insert › Timeline · changes the document
  Parameters: `{name?}`
- **Add Classic Motion Guide** `layer.add_motion_guide` · Insert › Timeline · changes the document
  Parameters: `{layer?}`
- **Add Camera** `camera.add` · Insert › Timeline · changes the document
  Parameters: `{} (a camera layer on top of the scene, at rest: it sees the stage as it is; nothing happens if there is one)`
- **Scene** `scene.new` · changes the document
  Parameters: `{name?}`

## Modify

- **Document Settings** `doc.set_stage` · changes the document
  Parameters: `{width?, height?, fps?, background?: "#rrggbb"}`
- **Envelope** `modify.envelope` · Modify › Transform
  Parameters: `{} opens an envelope around the selected shapes and drawing objects on the active layer (ADR 0080): a mesh of 4 × 4 Bézier control points over their bounds, at rest → {points: [[x, y] × 16] (row by row from the top-left), bounds: [x, y, w, h], shapes}. Then modify.envelope_move bends them. Symbols, text, groups and bitmaps can't be bent: break them apart first`
- **Delete Layer** `layer.delete` · Modify › Timeline · changes the document
  Parameters: `{layer? (a folder goes with its contents)}`
- **Move Layer Up** `layer.move_up` · Modify › Timeline · changes the document
  Parameters: `{layer?}`
- **Move Layer Down** `layer.move_down` · Modify › Timeline · changes the document
  Parameters: `{layer?}`
- **Mask** `layer.toggle_mask` · Modify › Timeline · changes the document
  Parameters: `{layer?}`
- **Layer Properties** `layer.set_properties` · Modify › Timeline · changes the document
  Parameters: `{layer?, name?, type?: normal|mask|masked|guide|guided, mask?: coverage|alpha|luminance (a mask layer's mask type), color?, outline?: bool (view as outlines), height?: 100|200|300, visible?, locked?, onion?} sets several at once, as one undo step; answers with the layer's properties`
- **Remove Camera** `camera.remove` · Modify › Timeline · changes the document
  Parameters: `{} (removes the camera layer and its moves; asks first when the camera is animated)`
- **Add Warp Mesh** `warp.add` · Modify › Asset Warp · changes the document
  Parameters: `{element? (a shape or bitmap; default the one selected), resolution? (cells along the longer side, 2…48; default 12)} → {element, created, resolution, columns, rows}. Covers the artwork with a warp mesh on every keyframe of its layer that holds it (it becomes a drawing object); on artwork that has one, changes the mesh's resolution`
- **Remove Warp** `warp.remove` · Modify › Asset Warp · changes the document
  Parameters: `{element?} takes the warp mesh and its pins off the artwork on every keyframe of its layer: it shows as drawn again`
- **Split Motion** `motion.split` · Modify › Timeline · changes the document
  Parameters: `{layer?, frame?}: the motion tween becomes two at the frame, each moving as before → {first, second}`
- **Join Motion** `motion.join` · Modify › Timeline · changes the document
  Parameters: `{layer?, frame?, to? (the last frame of the spans to join; default: the span after the one at the frame)}: adjacent motion tweens of one object become one, keeping every frame as it was → {start, frames, joined}`
- **Add Shape Hint** `tween.add_hint` · Modify › Shape · **Ctrl+Shift+H** (macOS **⇧⌘H**) · changes the document
  Parameters: `{layer?, frame? (in the shape tween, or the keyframe it ends on)} → the new hint's letter; it starts at the shapes' centre, unplaced`
- **Remove All Hints** `tween.clear_hints` · Modify › Shape · changes the document
  Parameters: `{layer?, frame?}`
- **Duplicate Scene** `scene.duplicate` · Modify › Scene · changes the document
  Parameters: `{scene?}`
- **Rename Scene** `scene.rename` · Modify › Scene · changes the document
  Parameters: `{name, scene?}`
- **Delete Scene** `scene.delete` · Modify › Scene · changes the document
  Parameters: `{scene?}`
- **Move Scene Earlier** `scene.move_up` · Modify › Scene · changes the document
  Parameters: `{scene?}`
- **Move Scene Later** `scene.move_down` · Modify › Scene · changes the document
  Parameters: `{scene?}`
- **Bring to Front** `arrange.bring_to_front` · Modify › Arrange · **Ctrl+Shift+Up** (macOS **⇧⌘↑**) · changes the document
  Parameters: `{} (within each layer)`
- **Bring Forward** `arrange.bring_forward` · Modify › Arrange · **Ctrl+Up** (macOS **⌘↑**) · changes the document
  Parameters: `{} (within each layer)`
- **Send Backward** `arrange.send_backward` · Modify › Arrange · **Ctrl+Down** (macOS **⌘↓**) · changes the document
  Parameters: `{} (within each layer)`
- **Send to Back** `arrange.send_to_back` · Modify › Arrange · **Ctrl+Shift+Down** (macOS **⇧⌘↓**) · changes the document
  Parameters: `{} (within each layer)`
- **Convert to Symbol…** `modify.convert_to_symbol` · **F8** · changes the document
  Parameters: `{name? (default: the next free “Symbol n”; a name the folder holds gets a number), kind?: graphic|movie_clip|button (default: graphic), folder?: Library folder id (default: the top level)} → {symbol, instance, name, layer, folder} (a selection on several layers becomes one symbol with those layers inside; the instance lands on the topmost)`
- **Break Apart** `modify.break_apart` · **Ctrl+B** (macOS **⌘B**) · changes the document
  Parameters: `{} the selected symbol instances become their contents, text becomes shapes, and drawing objects become raw shapes that merge with the raw shapes they touch`
- **Group** `modify.group` · **Ctrl+G** (macOS **⌘G**) · changes the document
  Parameters: `{} puts the selected elements on each layer into one group there (ADR 0073), where the topmost of them was, and selects it → {group, members} (one layer) or {layers: [..]}. Members keep their places exactly: the group's transform starts as the identity. Raw shapes inside merge only with each other; double-click the group (symbol.edit) to edit it in place`
- **Ungroup** `modify.ungroup` · **Ctrl+Shift+G** (macOS **⇧⌘G**) · changes the document
  Parameters: `{} puts each selected group's members back on its layer where it was, each placed and coloured as the group showed it, and selects them; raw shapes among them merge with the drawing there → {members}`
- **Union** `modify.union` · Modify › Combine Objects · changes the document
  Parameters: `{} combine with op union`
- **Intersect** `modify.intersect` · Modify › Combine Objects · changes the document
  Parameters: `{} combine with op intersect`
- **Punch** `modify.punch` · Modify › Combine Objects · changes the document
  Parameters: `{} combine with op punch`
- **Crop** `modify.crop` · Modify › Combine Objects · changes the document
  Parameters: `{} combine with op crop`
- **Rotate 90° CW** `modify.rotate_cw` · Modify › Transform · **Ctrl+Shift+9** (macOS **⇧⌘9**) · changes the document
  Parameters: `{}`
- **Rotate 90° CCW** `modify.rotate_ccw` · Modify › Transform · **Ctrl+Shift+7** (macOS **⇧⌘7**) · changes the document
  Parameters: `{}`
- **Flip Horizontal** `modify.flip_h` · Modify › Transform · changes the document
  Parameters: `{}`
- **Flip Vertical** `modify.flip_v` · Modify › Transform · changes the document
  Parameters: `{}`
- **Remove Transform** `modify.remove_transform` · Modify › Transform · **Ctrl+Shift+Z** (macOS **⇧⌘Z**) · changes the document
  Parameters: `{}`
- **Left** `align.left` · Modify › Align · **Ctrl+Alt+1** (macOS **⌥⌘1**) · changes the document
  Parameters: `{to_stage?}`
- **Horizontal Center** `align.hcenter` · Modify › Align · **Ctrl+Alt+2** (macOS **⌥⌘2**) · changes the document
  Parameters: `{to_stage?}`
- **Right** `align.right` · Modify › Align · **Ctrl+Alt+3** (macOS **⌥⌘3**) · changes the document
  Parameters: `{to_stage?}`
- **Top** `align.top` · Modify › Align · **Ctrl+Alt+4** (macOS **⌥⌘4**) · changes the document
  Parameters: `{to_stage?}`
- **Vertical Center** `align.vcenter` · Modify › Align · **Ctrl+Alt+5** (macOS **⌥⌘5**) · changes the document
  Parameters: `{to_stage?}`
- **Bottom** `align.bottom` · Modify › Align · **Ctrl+Alt+6** (macOS **⌥⌘6**) · changes the document
  Parameters: `{to_stage?}`
- **Distribute Horizontal Centers** `align.distribute_h` · Modify › Align · changes the document
  Parameters: `{}`
- **Distribute Vertical Centers** `align.distribute_v` · Modify › Align · changes the document
  Parameters: `{}`
- **Attach Text to Path** `text.attach_path` · Modify › Text · changes the document
  Parameters: `{path?: element id of a shape (default: the other selected element, a shape), x?, y? (pick the shape at this stage point instead), offset? (px along the path; default by the text's alignment: its start, middle or end), flip?: bool} sets the selected static text along a copy of the shape's outline (its first contour); the shape stays as it is (ADR 0070)`
- **Release Text from Path** `text.release_path` · Modify › Text · changes the document
  Parameters: `{} the selected text on a path goes back to lines of text (ADR 0070)`

## Commands

- **Install Plug-in…** `plugin.install`
  Parameters: `{path: a .wasm plug-in, confirm? (when it replaces an installed plug-in's file)} validates and installs it; with a plug-ins folder it is copied there`
- **Manage Plug-ins…** `plugin.list`
  Parameters: `{} lists the installed plug-ins (the app shows them in a window)`
- **Reload Plug-ins** `plugin.reload`
  Parameters: `{folder? (makes it the plug-ins folder first)} re-reads every .wasm in the plug-ins folder`

## Control

- **Next Frame** `timeline.next_frame` · **.**
  Parameters: `{}`
- **Previous Frame** `timeline.prev_frame` · **,**
  Parameters: `{}`
- **Go to First Frame** `timeline.first_frame` · **Home**
  Parameters: `{}`
- **Go to Last Frame** `timeline.last_frame` · **End**
  Parameters: `{}`
- **Previous Scene** `scene.prev` · **Shift+PageUp** (macOS **⇧PageUp**)
  Parameters: `{}`
- **Next Scene** `scene.next` · **Shift+PageDown** (macOS **⇧PageDown**)
  Parameters: `{}`
- **Test Movie** `control.test_movie` · **Ctrl+Enter** (macOS **⌘↩**)
  Parameters: `{frames?: steps to play (default 10 s of frames, at most 10,000), events?: [{tick, type: move|down|up|click|leave, x, y} or {tick, type: key_down|key_up|key, key: code|letter|LEFT…}], seed?: Math.random()'s seed (default 0)} plays the movie with its actions and frame scripts from the first frame; returns the output, the sounds it started ({tick, sound, name, sync, gain}; buttons' as the pointer changes their state) and where it ended (the app opens the Test Movie window)`

## Not in a menu

These commands have no menu item: the panels, the stage and the timeline use them, and so can scripts and assistants.

- **Document Info** `doc.info`
  Parameters: `{}`
- **Delete Template** `file.delete_template`
  Parameters: `{folder, name, category?, confirm}: removes a user template and its picture (cannot be undone)`
- **List Templates** `file.templates`
  Parameters: `{folder? (the user templates folder)} → {builtin: [{id, name, category, description, width, height, fps, frames}], user: [{name, category, path, thumbnail}]}`
- **Export with Plug-in…** `file.export_plugin`
  Parameters: `{format: an exporter plug-in's id (see file.export_formats), path, from?, to? (movie frames: every scene in order), options?: {…} (the plug-in's; missing ones take their defaults), confirm?}`
- **Export Formats** `file.export_formats`
  Parameters: `{} → every export format: the built-in ones (with their command) and the exporter plug-ins (with their extension and options)`
- **Envelope** `modify.envelope_move` · changes the document
  Parameters: `{points: [[x, y] × 16] (the whole mesh) | index: 0..15, x, y (one control point; a corner or side handle moves the inner points with it, as a Coons patch), tolerance? (stage units, default 0.05)} bends the open envelope's shapes from where they were when it opened → {points, shapes}. One undo step`
- **Close Envelope** `modify.envelope_close` · changes the document
  Parameters: `{} closes the open envelope: bent raw shapes merge with the drawing they now touch (as after a move). Nothing to close is no error → {closed}`
- **Publish Settings** `publish.settings_set` · changes the document
  Parameters: `{profile? (a whole profile, as publish.settings_get returns it), format? (html | gif | apng | webp | video | lottie | sprite_sheet | png_sequence | png | svg | audio), enabled?, output? (relative to the document's folder; {name} is the document's name), and that format's options: title, sound, fit (html); from, to, scale, loop (gif); from, to, scale, loop (true, false or how many times it plays; 0: forever), background (apng, webp); from, to, scale, audio (video); from, to, background (lottie); from, to, scale, columns, padding, background (sprite_sheet); from, to, scale (png_sequence); frame, scale (png); frame (svg); from, to, rate, bits (audio). from/to/frame are movie frames, null for the movie's ends}`
- **Open Recent** `file.open_recent`
  Parameters: `{index? (in file.recent_list order, from 0) | path?, confirm? (needed over unsaved changes)}: opens it as file.open does; a file that is no longer there stays on the list, marked missing`
- **Recent Files** `file.recent_list`
  Parameters: ``{} → {files: [{index, path, name, pinned, missing, thumb}]}: pinned favourites first, then the last 10 files opened or saved, most recent first (`thumb` names the start screen's picture)``
- **Pin to Recent Files** `prefs.recent_pin`
  Parameters: `{index? | path?, pinned? (default: toggle)}: a pinned file stays at the top of Open Recent`
- **Remove from Recent Files** `prefs.recent_remove`
  Parameters: `{index? | path?}`
- **Clear Recent Files** `prefs.recent_clear`
  Parameters: `{pinned? (default false: pinned favourites stay)} → {removed}`
- **Import Dropped Files** `file.drop` · changes the document
  Parameters: ``{paths: [..] (or path; SVG, PNG, JPEG, WAV, OGG, MP3; at most 64), to: stage | library | timeline, x?, y? (stage and timeline: the point in the edited space to centre on, default the stage centre), folder? (library: a folder id), layer, frame (timeline: a layer id and a frame, from 0)} → {elements, items, layers}: on the stage each drawing, image and sound goes onto the active layer's keyframe at the playhead, centred on the drop point (a drawing with several layers or masks gets its own layers, as Import to Stage); in the Library images and sounds are added, and drawings become graphic symbols, into `folder`; on the timeline the playhead goes to the frame, a sound goes onto the layer's keyframe there (one sound per drop) and drawings and images onto the layer in a keyframe starting at that frame (made if needed, keeping what the frame showed; a drawing with several layers or masks goes in as a graphic symbol). One undo step``
- **Restore Autosaved Document** `file.recover`
  Parameters: `{dir, session (an id from file.recovery_list), confirm? (needed over unsaved changes)}`
- **Discard Autosaved Document** `file.discard_recovery`
  Parameters: `{dir, session, confirm (always: it cannot be undone)}`
- **Autosave Now** `file.autosave`
  Parameters: `{dir, session? (default headless-<process id>)}`
- **Move Keyframes** `timeline.move_keyframes` · changes the document
  Parameters: `{layer? | layers?: [..], from, to?, by, layer_by? (rows down; negative = up)}`
- **Move Spans** `timeline.move_spans` · changes the document
  Parameters: `{layer? | layers?: [..], from, to?, by, layer_by? (rows down; negative = up), copy?: true (Alt-drag: the frames stay where they were too)}: the spans starting in from..=to move whole, keeping their lengths and tweens (as dragging a tween span in Animate), replacing what they land on; the frames they leave are empty`
- **Set Tween Easing** `timeline.set_easing` · changes the document
  Parameters: `{easing: a preset name (Linear, QuadIn … ElasticInOut) | {classic: -100..100} | {bezier: [x1, y1, x2, y2]} | {custom: {points: [..]} | [[x, y], ..]} | {saved: name} | {plugin: id, ease: id} (an ease plug-in's; the tween keeps its sampled curve), group?: all|position|rotation|scale|color|filters (classic tweens: that property group's own ease), layer?, frame?}`
- **Orient to Path** `timeline.set_orient` · changes the document
  Parameters: `{orient: bool, layer?, frame?}`
- **Frame Label** `timeline.set_label` · changes the document
  Parameters: `{label, layer?, frame?}`
- **Go to Frame** `timeline.goto`
  Parameters: `{frame}`
- **Rename Layer** `layer.rename` · changes the document
  Parameters: `{name, layer?}`
- **Show/Hide Layer** `layer.toggle_visible` · changes the document
  Parameters: `{layer?}`
- **Lock/Unlock Layer** `layer.toggle_lock` · changes the document
  Parameters: `{layer?}`
- **Outline Layer** `layer.toggle_outline` · changes the document
  Parameters: `{layer?}`
- **Select Layer** `layer.select`
  Parameters: `{layer: id} or {index}, select?: bool (also select all of its artwork at the playhead, as a click on the layer's name does; nothing on a locked or hidden layer)`
- **Layer Type** `layer.set_type` · changes the document
  Parameters: `{type: normal|mask|masked|guide|guided, layer?}`
- **Onion Skin Layer** `layer.set_onion` · changes the document
  Parameters: `{onion?: bool (default: the other way), solo?: bool, layer?} whether onion skin draws the layer's frames around the playhead. solo: only this layer is onion-skinned (every other layer off); solo again on the only layer on turns them all back on (Alt-click)`
- **Layer Colour** `layer.set_color` · changes the document
  Parameters: `{color: "#rrggbb", layer?} the layer's colour: its outlines in outline mode and its bar on the timeline`
- **Layer Height** `layer.set_height` · changes the document
  Parameters: `{height: 100 | 200 | 300 (percent of a normal row), layer?} the layer's row height on the timeline`
- **Mask Type** `layer.set_mask` · changes the document
  Parameters: `{mask: coverage|alpha|luminance, layer?} how a mask layer reveals the layers it masks: coverage (its fills' area, as in Animate), alpha (its artwork's opacity: gradients and faded shapes make soft edges) or luminance (its brightness: white shows, black hides; SVG's mask) (ADR 0070)`
- **Get Layer Properties** `layer.properties`
  Parameters: `{layer?} the layer's name, type, mask type (mask layers), colour, outline, height (percent), visible, locked and onion`
- **Camera** `camera.set` · changes the document
  Parameters: `{x?, y? (camera centre, stage px), dx?, dy? (pan by), zoom? (%, 5–2000), zoom_by? (factor), rotation? (°), rotate_by? (°), reset?: bool, frame?, and the colour effect as element.set_color_effect takes it: style?: none|brightness|tint|advanced, brightness? (%), tint? (#rrggbb), tint_amount? (%), red_multiplier? … blue_offset? (the camera has no opacity: no alpha values)}`
- **Camera Info** `camera.info`
  Parameters: `{frame?} → the camera at the frame (x, y, zoom %, rotation °, tint) and its frame on the stage`
- **Layer Depth** `layer.set_depth` · changes the document
  Parameters: ``{depth (px behind the stage, −500…10000; through the camera a deeper layer moves and scales less, and draws behind nearer ones), layer?, key?: bool (default: whether the layer's depth is keyed), frame? (default: the playhead)} with `key` sets a depth key at the frame (the first key away from frame 0 also keys frame 0 with the depth it had), so the depth tweens linearly between keys; `key: false` makes the depth one value again (ADR 0082)``
- **Remove Depth Key** `layer.remove_depth_key` · changes the document
  Parameters: `{layer?, frame? (default: the playhead)} removes the layer's depth key at the frame; removing the last one leaves the depth it had there`
- **Attach Layer to Camera** `layer.set_attached` · changes the document
  Parameters: `{attached?: bool (default: toggles), layer?} an attached layer stays where it is on screen while the camera moves (titles, HUDs); its depth plays no part (ADR 0082)`
- **Layer Depths** `layer.depths`
  Parameters: `{frame?} → {camera: bool, layers: [{layer, name, depth, keyed, key_here, attached}]} the layers that can have a depth, nearest first (ties in timeline order), at the frame`
- **Add Bone** `bone.add` · changes the document
  Parameters: ``{from (instance id: the bone starts on it), to (instance id: the bone moves it), head?: [x, y] (the joint on a root instance, stage px, default its registration point; from an instance that hangs from a bone the joint is always that bone's end, so chains stay joined), tail?: [x, y] (default `to`'s registration point), name?} → {bone, layer}. Both instances go to an armature layer (made for the first bone)``
- **Add Bone in Shape** `bone.add_in_shape` · changes the document
  Parameters: `{shape? (shape id: the first bone, or another bone from the shape), parent? (bone id: continue the chain from its end), head?: [x, y] (the joint, stage px; default the parent bone's end), tail: [x, y] (stage px), name?} → {bone, layer, created, points}. The first bone moves the shape to an armature layer of its own, as an object (merge drawing leaves it alone); the shape's points are bound to the nearest bones`
- **Bind Points** `bone.bind` · changes the document
  Parameters: `{bone, points?: [point index…] (default every point; see bone.info with points), mode?: "only" (the points follow just this bone; default) | "add" (they share it with the bones they follow) | "remove" | "auto" (back to automatic binding)} → {bone, changed, bound (how many points the bone moves)}. A shape armature's binding: which points of the shape follow which bone`
- **Paint Weights** `bone.paint_weights` · changes the document
  Parameters: `{bone, at: [[x, y]…] (dab centres, stage px, at most 4096), radius? (px, 1–2000, default 20), strength? (0–100 at the centre, default 25), mode?: "add" (default) | "subtract" | "smooth", frame?} → {bone, changed, bound}. The Bind tool's brush: raises, lowers or evens out how much the bone moves the shape's points within the radius of each dab (a smooth falloff to the rim); each point's weights stay normalised (its other bones share what is left) and the points become hand-bound. A drag sends many in one gesture: one undo step`
- **Set Point Weight** `bone.set_weight` · changes the document
  Parameters: `{bone, points: [point index…] (see bone.info with points), weight (0–100: the bone's share of each point)} sets exactly how much the bone moves the points; their other bones share the rest in proportion (a point that followed only this bone gives the rest to its nearest other bones). Hand-bound`
- **Remove Bone** `bone.remove` · changes the document
  Parameters: `{bone} removes it and the bones below it; the instances they moved go to a layer of their own as they look at the playhead (with the last bone, the whole armature does); asks first when the armature has more than one pose`
- **Joint Limits** `bone.set_limits` · changes the document
  Parameters: `{bone, constrain?: bool (false: turns freely), min?, max? (°, −360…360, from where it was drawn)} limits how far the joint turns, in every pose`
- **Joint Translation** `bone.set_translation` · changes the document
  Parameters: `{bone, axis: "x" | "y" (along the axes of what the bone starts on, as it is turned), enabled?: bool (false: the joint doesn't slide that way), constrain?: bool (false: slides freely), min?, max? (px, ±10,000, from where it was drawn)} lets the joint slide, in every pose; posing (bone.solve, the Selection tool) slides it only where turning can't reach`
- **Bone Spring** `bone.set_spring` · changes the document
  Parameters: `{bone, strength? (0–100: 0 is no spring; stiffer swings faster and lags less), damping? (0–100: how quickly the swing dies down; 100 absorbs it all)} secondary motion: the bone lags behind and swings past its pose as the bones it hangs from move, then settles. Evaluated, never stored: the keyed poses don't change`
- **Enable Springs** `bone.enable_springs` · changes the document
  Parameters: `{enabled: bool, layer? (the armature layer) | bone?} turns the armature's springs on or off (off: it plays its keyed poses exactly, as while posing)`
- **Pin Bone** `bone.pin` · changes the document
  Parameters: `{bone, pinned: bool} a pinned bone (and the bones it hangs from) stays put while dragging bones below it`
- **Pose Bone** `bone.pose` · changes the document
  Parameters: `{bone?, angle? (°, the joint angle: 0 as drawn), rotate_by? (°), slide_x?, slide_y? (px: how far the joint slides, within its translation limits), dx?, dy? (moves the whole armature), layer? (the armature layer, when no bone is given), frame?} keys the pose at the frame`
- **Drag Bone** `bone.solve` · changes the document
  Parameters: `{bone, x, y (stage px: where the bone's end goes), local?: [x, y] (the point dragged, in the bone's instance's space; default its end), frame?} → {reached, error (px), chain}; inverse kinematics turns the bone and the bones above it (limits respected, pinned bones kept)`
- **Armatures** `bone.info`
  Parameters: `{frame?, layer?, points?: bool} → every armature on the scene at the frame: its bones with joint and end (stage px, as keyed), joint angle and limits (°), slide and translation limits (px), spring (0–100) and its swing at the frame (°, what the spring adds to the angle), pinned; whether springs are on; a shape armature's shape, how many points each bone moves, and with points the shape's points (stage px) and the bones each follows`
- **Add Warp Pin** `warp.pin_add` · changes the document
  Parameters: `{element?, x, y (stage px, on the artwork)} → {element, pin, created}. Pins the artwork there (adding the mesh first if it has none), on every keyframe that bends the same mesh; nothing moves`
- **Move Warp Pin** `warp.pin_move` · changes the document
  Parameters: `{element?, pin, x?, y? (stage px: where the pin goes), dx?, dy? (stage px: by how much), frame?} → {element, pin, at}. Bends the artwork at the frame (keying it inside a tween); pins tween between keyframes`
- **Remove Warp Pin** `warp.pin_remove` · changes the document
  Parameters: `{element?, pin} removes the pin from every keyframe that bends the same mesh; the artwork follows the pins that are left`
- **Warp Mesh** `warp.info`
  Parameters: `{element?, frame?} → the artwork's warp at the frame: resolution, the mesh's columns, rows, vertices and triangles, whether it is bent, and its pins (id, where they show in stage px, and where they were placed on the artwork as drawn, in its own space)`
- **Property Keyframe** `motion.set_key` · changes the document
  Parameters: `{property: x|y|scale_x|scale_y|rotation|skew|alpha|brightness|tint (amount)|tint_red|tint_green|tint_blue (0..255)|red_multiplier|green_multiplier|blue_multiplier|alpha_multiplier (%)|red_offset|green_offset|blue_offset|alpha_offset (-255..255)|filter.<slot>.<blur_x|blur_y|strength (%)|angle|distance|brightness|contrast|saturation|hue> (a number of the object's filter at that place, 0 the first; ADR 0092), value (px, %, °), frame?, layer?} (colour properties of several styles compose into Advanced, ADR 0056)`
- **Remove Property Keyframe** `motion.remove_key` · changes the document
  Parameters: `{property? (default: every property), frame?, layer?}`
- **Property Ease** `motion.set_ease` · changes the document
  Parameters: `{easing (as timeline.set_easing takes it, saved eases too), property? (default: every property), frame? (inside the segment), layer?}`
- **Motion Tween** `motion.info`
  Parameters: `{layer?, frame?} → the span's properties, keys, eases and values at the frame, its motion path, its anchors pinned to frames (pins) and where the object is`
- **Edit Motion Path** `motion.set_path` · changes the document
  Parameters: `{d: SVG path data in stage space (its first subpath; ends are the start and end positions), layer?, frame?}`
- **Remove Motion Path** `motion.clear_path` · changes the document
  Parameters: `{layer?, frame?}: back to the straight path the X/Y curves trace`
- **Orient to Path** `motion.orient` · changes the document
  Parameters: `{orient: bool, layer?, frame?}: turn the object as its motion path turns`
- **Add Motion Path Point** `motion.add_point` · changes the document
  Parameters: `{x, y, slop? (stage units, default 4) | segment, t (0..1), layer?, frame?}: an anchor on the motion path, its shape unchanged → {anchor}`
- **Delete Motion Path Points** `motion.delete_points` · changes the document
  Parameters: `{anchors: [index, ..] (as motion.info lists them; the ends stay), layer?, frame?}: the path refits through the rest`
- **Convert Motion Path Point** `motion.convert_point` · changes the document
  Parameters: `{anchor, smooth: bool, layer?, frame?}: smooth (handles in line) or corner (handles on the anchor)`
- **Move Motion Path** `motion.move_path` · changes the document
  Parameters: `{dx, dy, layer?, frame?}: the whole path with its object, keeping the timing`
- **Pin Motion Path Point** `motion.pin_point` · changes the document
  Parameters: `{anchor? (default: a new anchor where the object is), frame? (the frame to pin it to, between the first and last position keys), layer?}: the object passes the anchor exactly on that frame → {anchor, frame, pins}`
- **Unpin Motion Path Point** `motion.unpin_point` · changes the document
  Parameters: `{anchor? | frame?, layer?}: the anchor shapes the path only again; the X/Y keys of its frame go → {anchor, frame, pins}`
- **Move Property Keyframes** `motion.move_keys` · changes the document
  Parameters: `{keys: [{property, frame}, ..] (frames as motion.info lists them), by (frames; negative = earlier), layer?, frame? (in the span; default: the first key's)}: values and eases travel with their keys; no key moves outside the span or onto or past a key of its property that isn't moving (the move stops there); a pinned path point's X and Y keys move together → {by, keys}`
- **Span Length** `timeline.resize_span` · changes the document
  Parameters: `{frames (the new length), layer?, frame?, scale_keys?: false (keep a motion tween's keys where they are; default: they stretch in proportion, as dragging the span's end in Animate)}: later frames move with the end, as with F5 / Shift+F5 → {start, frames, scaled}`
- **Ease** `ease.info`
  Parameters: `{target: layer?, frame?, group? | property? | saved?} → the ease (as easing params take it), its label, and its curve as points (exact unless the preset is fitted)`
- **Set Ease** `ease.set` · changes the document
  Parameters: `{easing: a preset name | {classic: n} | {bezier: [..]} | {custom: ..} | {saved: name} | {plugin: id, ease: id} (the tween keeps the sampled curve) | {sampled: ..}, layer?, frame?, group? | property? | saved?}`
- **Add Ease Point** `ease.add_point` · changes the document
  Parameters: `{t: time 0..1 on the curve (its shape stays), layer?, frame?, group? | property? | saved?} → {index}`
- **Move Ease Point** `ease.move_point` · changes the document
  Parameters: `{index, x (time, kept between its neighbours), y (value, may overshoot), layer?, frame?, group? | property? | saved?} (the ends stay at (0, 0) and (1, 1))`
- **Move Ease Handle** `ease.move_handle` · changes the document
  Parameters: `{index, handle: in|out, x, y (absolute; x stays between the point and its neighbour), layer?, frame?, group? | property? | saved?} (a smooth point's other handle turns with it)`
- **Delete Ease Point** `ease.delete_point` · changes the document
  Parameters: `{index (not an end), layer?, frame?, group? | property? | saved?}`
- **Convert Ease Point** `ease.convert_point` · changes the document
  Parameters: `{index, smooth: bool (handles in line) or corner (handles on the point), layer?, frame?, group? | property? | saved?}`
- **Ease Properties Separately** `ease.separate` · changes the document
  Parameters: `{separate: bool, layer?, frame?}: a classic tween with one ease per property group (each starting from the tween's), or one ease for all (the groups' own go)`
- **Save Ease** `ease.save` · changes the document
  Parameters: `{name, easing? (default: the target's ease), replace?: bool, layer?, frame?, group? | property?} → saved in the document, offered with the presets`
- **Delete Saved Ease** `ease.delete` · changes the document
  Parameters: `{name} (tweens using it keep their curves)`
- **Rename Saved Ease** `ease.rename` · changes the document
  Parameters: `{name, to}`
- **Eases** `ease.list`
  Parameters: `{} → the presets (name, label, family, direction), the eases saved in the document, and the installed ease plug-ins' eases ({plugin, ease} as easing params take them)`
- **Copy Ease** `ease.copy`
  Parameters: `{layer?, frame?, group? | property? | saved?}: the ease, for ease.paste`
- **Paste Ease** `ease.paste` · changes the document
  Parameters: `{layer?, frame?, group? | property? | saved?}: the copied ease onto this target`
- **Move Shape Hint** `tween.move_hint` · changes the document
  Parameters: `{hint: a..z, x, y, side?: start|end (default: the side the frame shows), layer?, frame?} (snaps to the nearest shape edge on that keyframe)`
- **Remove Shape Hint** `tween.remove_hint` · changes the document
  Parameters: `{hint: a..z, layer?, frame?} (later letters move down one)`
- **Shape Hints** `tween.hints`
  Parameters: `{layer?, frame?} → the shape tween's hints: letter, start and end points, and whether each is on an edge`
- **Go to Scene** `scene.select`
  Parameters: `{scene: index}`
- **Artwork on Multiple Frames** `frames.artwork`
  Parameters: `{first, last} lists every element Edit Multiple Frames shows in frames first..=last (0-based): [layer, frame (its keyframe's first frame), id] with stage bounds; the playhead's elements as they look there`
- **Move on Multiple Frames** `frames.edit_multiple` · changes the document
  Parameters: `{first, last, dx, dy, items?: [[layer, frame, id], ..], merge?: bool} moves artwork on every keyframe within frames first..=last (0-based) on visible, unlocked layers, as Edit Multiple Frames does: the items given (a keyframe's first frame and an element id), or all of them; a motion tween's path and X/Y keys move with its object; raw shapes merge again where they land unless merge is false; one undo step; answers the items after the move`
- **Transform on Multiple Frames** `frames.transform` · changes the document
  Parameters: `{first, last, items?, merge?, a, b, c, d, e, f} (an affine in stage space) | {rotate? (degrees), scale_x?, scale_y?, skew_x?, skew_y? (degrees), pivot?: [x, y] (default: the centre of every item's bounds together)} transforms the items on each keyframe about one shared pivot, as Free Transform does with Edit Multiple Frames on; one undo step`
- **Align on Multiple Frames** `frames.align` · changes the document
  Parameters: `{first, last, edge: left|hcenter|right|top|vcenter|bottom, items?, to_stage? (default: one item aligns to the stage), merge?} lines the items up with the edge of their bounds together (or the stage's), each on its own keyframe`
- **Clear on Multiple Frames** `frames.delete` · changes the document
  Parameters: `{first, last, items?} deletes the items (default: everything shown) from their keyframes`
- **Copy on Multiple Frames** `frames.copy`
  Parameters: `{first, last, items?} copies the items (default: everything shown), remembering each one's keyframe: Paste on Multiple Frames puts them back keyframe by keyframe, Paste puts them all on the current frame`
- **Cut on Multiple Frames** `frames.cut` · changes the document
  Parameters: `{first, last, items?} frames.copy, then frames.delete`
- **Paste in Place on Multiple Frames** `frames.paste` · changes the document
  Parameters: ``{first, last, layer?} pastes in place onto the keyframes within the markers: artwork copied from several keyframes onto the keyframe covering the frame each came from (on the layers it came from when it came from several, else on `layer` or the active layer); artwork copied on one frame onto every keyframe of the layer; raw shapes merge where they land``
- **Merge on Multiple Frames** `frames.merge` · changes the document
  Parameters: `{first, last, items} the raw shapes among the items merge again with what they touch on their keyframes (the end of a drag made with merge: false); answers the items after the merge`
- **Rectangle** `draw.rect` · changes the document
  Parameters: `{x, y, w, h, radius?, fill?, stroke?, stroke_width?, profile?, cap?, join?, style?, dash?, object?}`
- **Oval** `draw.ellipse` · changes the document
  Parameters: `{x, y, w, h, fill?, stroke?, stroke_width?, profile?, cap?, join?, style?, dash?, object?}`
- **PolyStar** `draw.star` · changes the document
  Parameters: `{cx, cy, outer (the corners' radius), shape?: star|polygon (default star; the PolyStar Tool Settings, ADR 0080), sides? (or points; 3..64, default 5), point_size? (0..1, default 0.5: how deep a star's points are, 1 is the polygon through its tips), inner? (an exact inner radius instead of point_size), fill?, stroke?, stroke_width?, profile?, cap?, join?, style?, dash? (the line style), object?}`
- **Line** `draw.line` · changes the document
  Parameters: `{x0, y0, x1, y1, stroke?, stroke_width?, profile? (a width profile name or [[t, left, right], ..]), cap?, join?, miter?, style?, dash?, dash_offset?, dot_spacing? (the line style, as stroke.set_style; default: the tool's), object? (true: a drawing object; default: Object Drawing, off: it merges with the raw shapes it touches)}`
- **Object Drawing** `draw.object_drawing` · **J**
  Parameters: `{on?: bool} (default: toggle)`
- **Select Elements** `element.select`
  Parameters: ``{ids: [..], add?: bool, toggle?: bool, layer?} selects elements on any visible, unlocked layer at the playhead (ids are unique; `layer` looks only there); `toggle` takes each one that is selected out of the selection and adds the others (Shift+Space on a stage element); the active layer becomes the first one's unless it has some of them``
- **Select at Point** `element.pick`
  Parameters: ``{x, y, add?: bool, view?: stage|camera} the topmost element there on a visible, unlocked layer; `add` (Shift) adds it to the selection, or lets go of it, on any layer; with `view: camera` the point is on the movie's picture as the camera shows it (each layer through its depth)``
- **Move** `element.translate` · changes the document
  Parameters: ``{dx, dy, snap?, from?: [x, y]} moves the selection, on every layer; with `snap` (a distance) one of its corners or ends, or its box's corners or centre (the nearest to `from`), then snaps within that distance by the snap engine's priority (ADR 0076): a point, guide, line or fill edge of the rest, the grid, a whole pixel``
- **Transform** `element.set_transform` · changes the document
  Parameters: `{x?, y?, sx?, sy?, rotation? (degrees)}`
- **Fill Colour** `element.set_fill` · changes the document
  Parameters: `{color: "#rrggbb" | null}`
- **Stroke** `element.set_stroke` · changes the document
  Parameters: `{color?: "#rrggbb" | null, width?, profile?: uniform|taper_both|taper_start|taper_end|bulge|pinch | [[t, left, right], ..], cap?, join?, miter?, style?, dash?, dash_offset?, dot_spacing? (as stroke.set_style)}`
- **Alpha** `element.set_alpha` · changes the document
  Parameters: `{alpha: 0..1} sets the alpha multiplier and keeps the rest of the colour effect (None becomes Alpha; Brightness and Tint become Advanced below 1)`
- **Colour Effect** `element.set_color_effect` · changes the document
  Parameters: `{style?: none|brightness|tint|advanced|alpha (default: the style the values belong to, else each element's own), alpha? (%, 0..100), brightness? (%, -100..100), tint? (#rrggbb), tint_amount? (%, 0..100), red_multiplier?, green_multiplier?, blue_multiplier?, alpha_multiplier? (%, -100..100), red_offset?, green_offset?, blue_offset?, alpha_offset? (-255..255)} on every selected element (ADR 0056); values not given keep each element's own. A new style starts from the look it can keep: Advanced is exactly the effect before, Alpha keeps the opacity, Brightness and Tint start at 0 %. The camera has no opacity (no Alpha style, alpha multiplier or offset)`
- **Blending** `element.set_blend` · changes the document
  Parameters: `{mode: normal|layer|darken|multiply|lighten|screen|overlay|hardlight|add|subtract|difference|invert|alpha|erase} on the selected movie clip and button instances`
- **Filters** `element.set_filters` · changes the document
  Parameters: `{filters: [{type: blur|drop_shadow|glow|bevel|gradient_glow|gradient_bevel|adjust_color, …values, enabled?: bool (default true; false switches it off, keeping its settings)}]} replaces the selected movie clip and button instances' filters (in order; [] removes them). Bevel: shadow_color, highlight_color, side: inner|outer|full, knockout; gradient filters: stops: [{offset 0..1, color "#rrggbbaa"}] (Gradient Glow's start and Gradient Bevel's middle stay transparent)`
- **Looping** `element.set_loop` · changes the document
  Parameters: `{loop?: loop|play_once|single_frame, first_frame?}`
- **Symbol Type** `symbol.set_kind` · changes the document
  Parameters: `{kind: graphic|movie_clip|button, symbol?: id (default: the symbols of every selected instance)}`
- **9-Slice Scaling** `symbol.set_scale_grid` · changes the document
  Parameters: `{symbol?: id (default: the symbol open for editing, else the selected instance's), on?: bool, left?, right?, top?, bottom? (guide positions in the symbol's space)} turns a symbol's guides for 9-slice scaling on or off, or moves them (ADR 0055). Turning them on brings back the guides it had when they were turned off (ADR 0057), else puts them a quarter of the way in from each edge of the symbol's artwork; moving one turns them on. Scaled instances then keep the corners outside the guides unscaled, stretch the edges one way and the centre both ways → {symbol, scale_grid: {left, right, top, bottom} | null}`
- **Instance Name** `element.rename` · changes the document
  Parameters: `{name}`
- **Place on Stage** `library.place` · changes the document
  Parameters: `{symbol | bitmap | sound: id, x?, y?} or {items: [{symbol | bitmap | sound: id}, ..], x?, y?} → {element, elements, sound?}: symbols and bitmaps centred on x, y (default: the stage centre; inside an open symbol, in its space) in the active layer's keyframe at the playhead, selected; a sound goes onto that keyframe, its span grown to fit (one sound per keyframe). One undo step`
- **Move Guide** `view.guide_move` · changes the document
  Parameters: ``{index, at} moves this scene's guide `index` (as view.aids lists them) to `at`; refused while guides are locked``
- **Delete Guide** `view.guide_delete` · changes the document
  Parameters: ``{index} removes this scene's guide `index`; refused while guides are locked``
- **Rulers, Grid and Guides** `view.aids`
  Parameters: `{} whether the rulers, grid and guides show and snap, the grid's and guides' settings, and this scene's guides [{axis, at}, ..]`
- **Combine Objects** `modify.combine` · changes the document
  Parameters: `{op: union|intersect|punch|crop} combines the selected shapes on the active layer, bottom to top as they are stacked (ADR 0073). union: one drawing object of everything (a group of the merged pieces when they have more than one colour or keep lines); intersect: the area every one covers, in the topmost's fill and stroke; punch: the topmost's area cut out of each of the others; crop: each of the others cut to the topmost's area. Punch and crop remove the topmost. A result of one fill (or one line) is a drawing object, one of several is a group of raw shapes → {op, created}`
- **List Library** `library.items`
  Parameters: ``{} every folder (id, name, parent, path) and item (symbol | bitmap | sound id, kind, name, folder, path, uses, unused) in Library order; `uses` counts instances, bitmap fills and keyframe sounds once per keyframe``
- **New Folder** `library.new_folder` · changes the document
  Parameters: `{name? (default: the next free “Folder n”), parent?: folder id} → {folder, name}; names are unique within a folder (case aside) and can't hold “/”`
- **Rename** `library.rename` · changes the document
  Parameters: `{symbol | bitmap | sound | folder: id, name} renames a Library item or folder; the name must be free in its folder (case aside) and can't hold “/”`
- **Move to Folder** `library.move` · changes the document
  Parameters: `{items: [{symbol | bitmap | sound | folder: id}, ..], to: folder id | null (the top level)} moves items and folders (with what they hold) into a folder; a folder can't go inside itself, and a name the folder already has stops the move`
- **Duplicate Symbol** `library.duplicate` · changes the document
  Parameters: `{symbol, name? (default: “<name> copy”)} → {symbol, name}: a copy in the same folder, with its own layers and elements and no linkage class`
- **Delete** `library.delete` · changes the document
  Parameters: `{items: [{symbol | bitmap | sound | folder: id}, ..]} deletes items and folders with everything in them; instances of deleted symbols, shapes filled with deleted bitmaps and keyframes' deleted sounds go too (asks first when anything deleted is in use or a folder isn't empty)`
- **Select Unused Items** `library.unused`
  Parameters: `{} → {items: [..]} the items nothing in the movie reaches: not in a scene, nor inside a symbol that is; symbols with a linkage class count as used`
- **Delete Unused Items** `library.delete_unused` · changes the document
  Parameters: ``{} deletes every item `library.unused` lists (asks first)``
- **Allow Smoothing** `library.set_smoothing` · changes the document
  Parameters: `{bitmap, on?: bool (default: toggle)} whether the bitmap is filtered when scaled or turned (off: hard-edged pixels)`
- **Swap Bitmap** `element.swap_bitmap` · changes the document
  Parameters: `{bitmap} the selected bitmaps and bitmap-filled shapes show another bitmap: a placed bitmap keeps its transform and top-left corner and takes the new bitmap's size; a bitmap fill keeps its shape and fill transform`
- **Swap Symbol** `element.swap_symbol` · changes the document
  Parameters: `{symbol} the selected instances show another symbol, keeping their transform, colour effect, blending, filters, looping, actions and instance name`
- **Drop** `draw.drop` · changes the document
  Parameters: `{} the selected raw shapes cut and join the raw shapes they touch (after a move), on each layer`
- **Select Connected** `element.select_connected`
  Parameters: `{x, y, add?, slop?} a raw stroke and the strokes joined to it, or a raw fill and the strokes on and in it`
- **Transform** `modify.transform` · changes the document
  Parameters: `{a, b, c, d, e, f} affine applied in stage space (free transform), to the selection on every layer`
- **Pen Path** `draw.path` · changes the document
  Parameters: `{d: SVG path data (stage space), fill?, stroke?, stroke_width?, object?}`
- **Pencil Stroke** `draw.freehand` · changes the document
  Parameters: `{points: [[x, y], ..], mode?: "smooth"|"straighten"|"ink", tolerance? (stage units), closed?, stroke?, stroke_width?, object?}`
- **Move Points** `path.move_points` · changes the document
  Parameters: `{id, points: [[cmd, slot], ..], dx, dy, smooth?: bool} (a handle moved alone at a smooth anchor turns the other one with it, keeping its length, unless smooth is false: Alt breaks the pair)`
- **Add Anchor Point** `path.add_point` · changes the document
  Parameters: `{id, x, y, slop?}`
- **Delete Anchor Points** `path.delete_points` · changes the document
  Parameters: `{id, cmds: [..]}`
- **Convert Anchor Point** `path.convert_point` · changes the document
  Parameters: `{id, cmd, smooth: bool}`
- **Bend Segment** `path.bend_segment` · changes the document
  Parameters: ``{x, y, dx, dy, id?, cmd?, t?, slop?, merge?: bool} the segment at (x, y) (of shape `id`, else the one there) bends so that point moves by (dx, dy), its ends staying; a raw shape's copies of the edge bend with it and merge again (unless merge is false)``
- **Move Point** `raw.move_point` · changes the document
  Parameters: ``{x, y, to_x, to_y, with?: [[x, y], ..], points?: [[id, cmd], ..], snap?, slop?, merge?: bool} moves the corner or end at (x, y), with every raw shape that shares it, to (to_x, to_y), snapped within `snap` to points, lines and fill edges (while Snap to Objects is on); the anchors at `with` move by as much; returns the points to pass on while a drag goes on``
- **Select Parts** `raw.select_rect` · changes the document
  Parameters: `{x0, y0, x1, y1, add?: bool} the marquee, on every visible, unlocked layer: the parts of raw fills and strokes under the rectangle (cut at its edges, per layer) and the objects wholly inside it`
- **Merge Shapes** `raw.merge` · changes the document
  Parameters: `{ids?: [..]} the raw shapes (default: the selection) merge again with the raw shapes they touch, in place`
- **Selection Tool Target** `raw.hit`
  Parameters: `{x, y, slop?, point_slop?} what a Selection-tool drag from (x, y) takes: a point, a segment to bend, a fill, or an object`
- **Subselect** `raw.subselect`
  Parameters: `{x, y, add?: bool, slop?} the Subselection tool's click: the raw stroke or fill at (x, y) and every raw shape connected to it are selected, to show their anchors (an object there is selected alone); returns the anchors and, on an edge, its ends`
- **Subselect Rectangle** `raw.subselect_rect`
  Parameters: `{x0, y0, x1, y1, add?: bool} the Subselection tool's marquee, on the active layer only: the shapes it touches are selected whole (raw ones with every raw shape connected to them); returns the raw anchors inside it`
- **Raw Points** `raw.points`
  Parameters: `{ids?: [..]} the anchors of raw shapes (default: the selection), once per position, and the handles of their curved segments`
- **Move Handle** `raw.move_handle` · changes the document
  Parameters: ``{id, cmd, slot: 1|2, dx, dy, smooth?: bool, merge?: bool} moves a handle (control point `slot` of segment `cmd` of shape `id`, as `raw.points` lists it) by (dx, dy): the curve changes in every raw shape that has that edge, and they merge again (unless merge is false); at a smooth anchor the other handle turns to stay opposite, keeping its length (unless smooth is false: Alt breaks the pair)``
- **Add Anchor Point** `raw.add_point` · changes the document
  Parameters: `{x, y, slop?} adds an anchor on the raw line or fill edge at (x, y): every raw shape with that edge gets it, and the curve keeps its shape; returns where it is`
- **Delete Anchor Points** `raw.delete_points` · changes the document
  Parameters: `{points: [[x, y], ..]} deletes the raw anchors at those positions (as raw.points lists them): every shape that has one is joined across it, so the drawing stays connected; a free end shortens its line; a shape left with nothing is removed; a point where three or more edges meet is kept`
- **Convert Anchor Points** `raw.convert_points` · changes the document
  Parameters: `{points: [[x, y], ..], smooth: bool} makes the raw anchors at those positions smooth (their handles in one line) or corners (handles drawn in), in every shape that has them`
- **Snap Point** `draw.snap`
  Parameters: ``{x, y, snap, moving?: bool} where a point drawn at (x, y) snaps within `snap` stage units, by the snap engine's priority (ADR 0076): an anchor (Snap to Objects), a guide (Snap to Guides), a line or fill edge, the grid (Snap to Grid), a whole pixel (Snap to Pixels); `moving`: the selection is what moves (a Free Transform handle), so it isn't a target; returns {x, y, snapped: {x, y, to: point|guide|line|grid|pixel} | null}``
- **Replace** `find.replace` · changes the document
  Parameters: ``{the find.search query, with: text (kind text; $1 and ${name} insert groups when regex) | family (font) | #rrggbb (color; each place keeps its alpha) | library id (symbol, sound, bitmap), at?: one hit as find.search returned it (Replace); without `at`, every hit (Replace All, one undo step; asks first above 100 places)} → {replaced, skipped, notes}``
- **Go to Found Item** `find.goto`
  Parameters: ``{layer?: id (its scene or symbol is found), scene?: index, symbol?: id, path?: [{layer, frame, element}] (instances to open in place from `scene`, or from `symbol` opened alone), frame?, element?} goes there: closes open symbols, switches scene or opens the symbol (in place through an instance that uses it when there is one), picks the layer and frame and selects the element``
- **Movie Explorer** `explorer.tree`
  Parameters: `{scope?: document (default) | scene, show?: {text?, symbols?, scripts?, media?, frames? (layers and keyframes)} (each default true), definitions?: bool (the Library's symbols; default true), expanded?: [row keys] (default: the top rows), all?: bool (everything open), filter?: text (rows whose names hold it, and the rows above them), offset?, limit? (default 1000, at most 10000)} → {rows: [{key, depth, kind, type?, name, extra, frame?, children, open, go: find.goto params}], total, truncated}`
- **Brush Stroke** `draw.brush` · changes the document
  Parameters: `{points: [[x, y], [x, y, pressure 0..1 (clamped)] or [x, y, pressure, altitude, azimuth] (the pen's tilt in degrees: altitude 90 upright to 0 flat, azimuth the way it leans, clockwise from +x)], ..] (one point paints a dab), preset? (a brush from brush.preset_list, by key or name: its settings are the defaults for the rest), size? (stage units, default 10), shape?: round|square, angle? (degrees), roundness? (5..100 %, default 100), smoothing? (0..100, default 50), curve?: linear|soft|firm|[[pressure, output], ..] (how pressure maps, both 0..1), pressure_size? (default true: pressure varies the size from 10 to 100 %), pressure_opacity? (default false: the stroke's average pressure sets its opacity), taper_start?, taper_end? (0..50 % of the stroke's length), spacing? (0 continuous, or 5..1000 % of the size between dabs), jitter? (0..50 % of the size: a ragged edge), texture? (0..100 %: a paper grain of holes through the paint), scatter? (0..200 % of the size: dabs thrown either side of the line, varied in size), tilt_angle? (the pen's tilt turns the tip: its angle plus the azimuth), tilt_width? (the pen's tilt widens the tip, up to twice as wide flat; also the Paint Brush's width), as_stroke? (default false; true: the Paint Brush, a variable-width stroke in the stroke colour, editable with the Width tool), mode?: normal|fills|behind|selection|inside (Paint Normal covers fills and lines, Paint Fills leaves lines, Paint Behind paints only empty areas, Paint Selection only inside the selected fills, Paint Inside only inside the fill the stroke starts on or the empty area closed off around it; ADR 0080), color? (default: the fill colour, or the stroke colour as a stroke), unit? (stage units per screen point, default 1: smoothing and accuracy follow it), object?}`
- **Brush Presets** `brush.preset_list`
  Parameters: `{} the built-in brushes and yours, with their settings`
- **Save Brush** `brush.preset_save`
  Parameters: `{name, from? (a preset to start from; default: the plain brush), size?, shape?, angle?, roundness?, smoothing?, curve?, pressure_size?, pressure_opacity?, taper_start?, taper_end?, spacing?, jitter?, texture?, scatter?, tilt_angle?, tilt_width? (as draw.brush)} saves a brush of your own (replacing yours of that name; kept in your preferences)`
- **Rename Brush** `brush.preset_rename`
  Parameters: `{name, to}`
- **Delete Brush** `brush.preset_delete`
  Parameters: `{name} deletes a brush of your own (asks first: it can't be undone)`
- **Export Brushes** `brush.preset_export`
  Parameters: `{path, names? (default: all yours)} writes your brushes to a brush set file (JSON)`
- **Import Brushes** `brush.preset_import`
  Parameters: `{path} adds the brushes in a brush set file to yours (a name you have already gets a number)`
- **Move Filter** `filter.move` · changes the document
  Parameters: ``{from, to} moves the filter at index `from` (0 is the first, applied first) to index `to` in the selected movie clip and button instances' lists; the order changes how they draw``
- **Enable Filter** `filter.enable` · changes the document
  Parameters: `{enabled: bool, index? (default: every filter)} switches a filter on or off; a filter switched off keeps its settings and draws nothing`
- **Remove Filter** `filter.remove` · changes the document
  Parameters: `{index? (default: every filter)} removes a filter from the selected movie clip and button instances`
- **Copy Filters** `filter.copy`
  Parameters: `{index? (default: every filter)} copies the selected instance's filters (one, or all), for Paste Filters`
- **Paste Filters** `filter.paste` · changes the document
  Parameters: `{replace? (default false: added after the instances' own)} pastes the copied filters onto the selected movie clip and button instances`
- **Filter Presets** `filter.preset_list`
  Parameters: `{} your filter presets, each a name and its filters`
- **Save Filter Preset** `filter.preset_save`
  Parameters: `{name, filters? (a list as element.set_filters takes; default: the selected instance's)} saves a filter preset of your own (kept in your preferences; a name you have already is replaced, after asking)`
- **Apply Filter Preset** `filter.preset_apply` · changes the document
  Parameters: `{name} gives the selected movie clip and button instances the preset's filters, replacing theirs (as Animate does)`
- **Rename Filter Preset** `filter.preset_rename`
  Parameters: `{name, to}`
- **Delete Filter Preset** `filter.preset_delete`
  Parameters: `{name} deletes a filter preset (asks first: it can't be undone; artwork that used it keeps its filters)`
- **Export Filter Presets** `filter.preset_export`
  Parameters: `{path, names? (default: all)} writes your filter presets to a small JSON file`
- **Import Filter Presets** `filter.preset_import`
  Parameters: `{path} adds the presets in a filter preset file to yours (a name you have already gets a number)`
- **Eraser** `draw.erase` · changes the document
  Parameters: `{points: [[x, y] or [x, y, pressure], ..] (one point erases a dab), size? (stage units, default 10), shape?: round|square, angle?, roundness?, mode?: normal|fills|lines|selected_fills|inside (Erase Normal takes fills and lines, Erase Fills leaves lines, Erase Lines leaves fills, Erase Selected Fills only the selected fills, Erase Inside only the fill the stroke starts on), unit? (stage units per screen point)} erases from the raw shapes on the active layer; drawing objects, symbols and text are kept`
- **Faucet** `draw.faucet` · changes the document
  Parameters: `{x, y, slop?} deletes the whole raw fill or stroke under the point on the active layer`
- **Erase Everything** `draw.erase_all` · changes the document
  Parameters: `{} clears every visible, unlocked layer at the playhead (asks first; undoable)`
- **Paint Bucket** `paint.bucket` · changes the document
  Parameters: ``{x, y, color?, gap? (stage units: how wide an opening still counts as closed), object?: bool} fills the closed area under the point, or recolours the fill there; unless `object` (default: the Object Drawing setting), the fill is raw and merges with the drawing (an area closed by raw lines and fills is filled exactly up to them)``
- **Ink Bottle** `paint.ink` · changes the document
  Parameters: `{x, y, color?, width?, profile?, cap?, join?, miter?, style?, dash?, dash_offset?, dot_spacing?} (the tool's line style unless given; see stroke.set_style)`
- **Eyedropper** `paint.pick`
  Parameters: `{x, y, other?: bool (into the other swatch only: a fill's colour into the stroke, a line's into the fill), slop?} picks the style under the point on any visible layer into the tool settings: a fill's paint (solid, gradient or bitmap, kept where it is on the stage) into the fill swatch, a line's colour, width and profile into the stroke settings`
- **Apply Style** `paint.apply_style` · changes the document
  Parameters: `{fill?: bool, stroke?: bool} (default both) gives the selected shapes the fill swatch's paint (shapes with a fill) and the stroke settings (shapes with a line, and drawing objects without one)`
- **Fill** `paint.set_fill` · changes the document
  Parameters: ``{type?: solid|linear|radial|bitmap (default: the swatch's), color? (solid), stops?: [{offset: 0..1, color: "#rrggbb" | "#rrggbbaa"}, ..] (2 to 15), spread?: extend|reflect|repeat, linear_rgb?: bool, focal?: -0.98..0.98 (radial), bitmap?: id, selection?: bool (default true)} sets the fill swatch (what new shapes, the Paint Bucket and the Brush pour) and, unless `selection` is false, the fills of the selected shapes. A shape keeping its kind of fill keeps where its gradient or bitmap lies; one changing kind gets it fitted to its bounds. A new gradient starts white to black → {fill: {type, ..}, changed}``
- **Gradient Transform** `paint.transform_fill` · changes the document
  Parameters: `{element?: id (default: the selected shapes with a gradient or bitmap fill), frame?: [a, b, c, d, e, f] (the fill's frame in the space being edited: the box -1..1 onto a linear gradient's span, a radial gradient's ring or one copy of a bitmap), or changes made in this order: scale_x?, scale_y?, scale? (along the frame's own axes), skew_x?, skew_y? (degrees, along its axes), rotate? (degrees, about its centre), move?: [dx, dy], center?: [x, y]; focal?: -0.98..0.98 (radial)} moves, turns, scales or skews where a gradient or bitmap fill lies, as the Gradient Transform tool (F) does → {changed, frames: [[a, b, c, d, e, f], ..]}`
- **Lock Fill** `paint.lock_fill`
  Parameters: `{on?: bool (default: toggle)} Lock Fill: a gradient or bitmap from the fill swatch is poured where it lies on the stage, running on across shapes; off, each shape gets it fitted to its bounds → {on}`
- **Swatches** `swatch.list`
  Parameters: `{} the document's swatches, in the Swatches panel's order → {swatches: [{index, name, type, color | stops, ..}], count}`
- **Add Swatch** `swatch.add` · changes the document
  Parameters: `{from?: fill|stroke (default fill: the fill swatch's colour or gradient; stroke: the stroke colour), or the swatch itself: type?: solid|linear|radial, color? ("#rrggbb" | "#rrggbbaa", solid), stops?: [{offset: 0..1, color}, ..] (2 to 15), spread?: extend|reflect|repeat, linear_rgb?: bool, focal?: -0.98..0.98 (radial); name?, at? (index; default: the end)} → {index, count}`
- **Delete Swatch** `swatch.remove` · changes the document
  Parameters: `{index} → {count}`
- **Move Swatch** `swatch.move` · changes the document
  Parameters: ``{from, to} (indices: the swatch at `from` ends up at `to`) → {index}``
- **Duplicate Swatch** `swatch.duplicate` · changes the document
  Parameters: `{index} puts a copy right after it → {index, count}`
- **Rename Swatch** `swatch.rename` · changes the document
  Parameters: `{index, name} (empty: no name, the hex shows on hover)`
- **Apply Swatch** `swatch.apply` · changes the document
  Parameters: ``{index, target?: fill|stroke (default fill), selection?: bool (default true)} sets the fill swatch (as paint.set_fill: a gradient is fitted to each shape) or the stroke colour, and unless `selection` is false the selected shapes' fills or strokes. A gradient swatch can't be a stroke (strokes are one colour) → {target, changed}``
- **Clear Swatches** `swatch.clear` · changes the document
  Parameters: `{} removes every swatch but black and white (Animate's Clear Colors; undoable)`
- **Default Swatches** `swatch.reset` · changes the document
  Parameters: `{} replaces the swatches with a new document's set (undoable)`
- **Import Swatches** `swatch.import` · changes the document
  Parameters: `{path: a swatch file (.json, from swatch.export) or a GIMP palette (.gpl), replace?: bool (default false: added at the end)} → {imported, count}`
- **Export Swatches** `swatch.export`
  Parameters: `{path} writes the document's swatches to a swatch file (JSON) → {written, swatches}`
- **New Text** `text.create` · changes the document
  Parameters: `{text (may be empty for an input field), type?: static|dynamic|input, x?, y? (top-left; default: centred), font?, size?, bold?, italic?, color?, align?, letter_spacing?, line_spacing?, width?}`
- **Edit Text** `text.edit` · changes the document
  Parameters: `{text?, font?, size? (px), bold?, italic?, color?, align?: left|center|right, letter_spacing?, line_spacing?, width? (px, wraps; null: one line per paragraph)} on the selected text`
- **Fonts** `text.fonts`
  Parameters: `{}`
- **Text on Path** `text.path` · changes the document
  Parameters: `{offset? (px along the path), flip?: bool (along the path backwards, on its other side), release?: bool (back to lines of text where it starts)} on the selected text on a path (ADR 0070)`
- **Text Type** `text.field` · changes the document
  Parameters: `{type?: static|dynamic|input, lines?: single|multiline|multiline_no_wrap|password (input only), width?, height? (the field's box, px), selectable?, border?, background?, border_color?, background_color?, max_chars? (input; 0: no limit), variable? (kept, not run)} on the selected text; dynamic and input fields are named with instance.set_name and scripted in Test Movie and the HTML export (ADR 0066)`
- **Width Profile** `stroke.set_profile` · changes the document
  Parameters: `{profile: uniform|taper_both|taper_start|taper_end|bulge|pinch | [[t, left, right], ..], id? (default: the selected shapes)}`
- **Width** `stroke.set_width_point` · changes the document
  Parameters: `{id, index? (an existing point; else one is added), t? 0..1 | x, y (stage point on the stroke), left?, right?, width? (both sides; factors of the nominal width)}`
- **Delete Width Point** `stroke.delete_width_point` · changes the document
  Parameters: `{id, index}`
- **Width at Point** `stroke.width_at`
  Parameters: `{x, y, id? (default: the topmost stroke there on the active layer), slop?}`
- **Stroke Style** `stroke.set_style` · changes the document
  Parameters: `{cap?: none|round|square, join?: miter|round|bevel, miter? (limit, 1..=100; a corner whose point is longer than this many widths is bevelled), style?: solid|dashed|dotted, dash? ([on, off, ..] lengths in the shape's units; implies dashed), dash_offset?, dot_spacing? (between dots, edge to edge; implies dotted), id? (default: the selected shapes)}`
- **Run Plug-in** `plugin.run` · changes the document
  Parameters: `{id, params?: {…} (see plugin.list; missing ones take their defaults)} one undo step`
- **Remove Plug-in** `plugin.remove`
  Parameters: `{id, confirm? (when it deletes the plug-in's file from the plug-ins folder)}`
- **Place Sound** `sound.place` · changes the document
  Parameters: `{sound, layer?, frame? (a blank keyframe is made there if the frame is empty), sync?: event|start|stop|stream, repeat?, loop?, effect?, envelope?, extend? (lengthen the layer's last span to the sound's length)}`
- **Sound Settings** `sound.set` · changes the document
  Parameters: `{layer?, frame?, sound?: id | null (null takes the sound off), sync?, repeat?, loop?, effect?: none|left_channel|right_channel|fade_left_to_right|fade_right_to_left|fade_in|fade_out|custom (presets shape over the time the sound is heard; custom starts from the effect before), envelope?: [{at (seconds from the sound's start), left (0–1), right (0–1)}…] (1–16 points; makes the effect custom)}`
- **Mute/Unmute Layer** `layer.toggle_mute` · changes the document
  Parameters: `{layer?} (its sounds, and those in the symbols on it, go silent in playback and every export)`
- **Layer Volume** `layer.set_volume` · changes the document
  Parameters: `{volume? (0–1: the layer's sounds and those in the symbols on it), muted?, layer?}`
- **Remove Sound** `sound.remove` · changes the document
  Parameters: `{layer?, frame?}`
- **Delete Sound** `sound.delete` · changes the document
  Parameters: `{sound} (from the library and every keyframe that uses it)`
- **Rename Sound** `sound.rename` · changes the document
  Parameters: `{sound, name}`
- **Set Actions** `actions.set` · changes the document
  Parameters: `{actions: [{op: stop|play|goto_and_play|goto_and_stop|next_frame|prev_frame|next_scene|prev_scene|trace|set_property, target?, frame?, scene?, message?, property?, value?}] | plugin: id, entry?, symbol?: id (a movie clip's timeline), layer?, frame?} sets what the keyframe at the frame runs; or {element, event: click|press|release|over|out, actions} sets an instance's event handler. An empty list clears it`
- **Clear Actions** `actions.clear` · changes the document
  Parameters: `{symbol?, layer?, frame?} clears the keyframe's actions; {element, event?} clears an instance's handlers (one event, or all)`
- **List Actions** `actions.list`
  Parameters: `{} every keyframe with actions or a frame script and every instance with event handlers, in every scene and symbol`
- **Instance Name** `instance.set_name` · changes the document
  Parameters: `{element?: id (default: the selected instance), name: letters, digits, _ or $, not starting with a digit; "" clears} names a movie clip or button instance, or a dynamic or input text field, for actions and scripts`
- **Set Frame Script** `script.set` · changes the document
  Parameters: ``{text?: the script, verbatim ("" clears; at most 1 MiB), language?: actionscript3|actionscript1or2|other (default: the script's, else actionscript3), symbol?: id, layer?, frame?} keeps a script on the keyframe at the frame (it replaces actions there); ActionScript 3.0 in the subset runs in Test Movie and the HTML export, the rest is kept and not run (`runs`, `problems` say which)``
- **Clear Frame Script** `script.clear` · changes the document
  Parameters: `{symbol?, layer?, frame?} removes the keyframe's kept script (actions are left alone: actions.clear)`
- **List Frame Scripts** `script.list`
  Parameters: `{text?: bool (default true)} every kept script in every scene and symbol, with its text, whether it runs and its problems, plus symbol linkage classes and the document class`
- **Check Frame Script** `script.check`
  Parameters: ``{text: the script, language?: actionscript3|actionscript1or2|other (default actionscript3)} whether it would run (the ActionScript 3.0 subset, ADR 0013), its problems with line and column, what is ignored, the names it takes from other frames of its timeline (`uses`), and what it compiles to (`steps`, `program`); changes nothing``
- **Set Script Class** `script.set_class` · changes the document
  Parameters: `{symbol?: id (default: the document), class: dotted class name ("" clears), base?: the class it extends} sets a symbol's linkage class, or the document class; kept as text, not run`
