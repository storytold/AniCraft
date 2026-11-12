// AniCraft HTML canvas player: the core (ADR 0028). SPDX-License-Identifier: MIT OR Apache-2.0
//
// Plays a movie exported by File > Export > HTML Canvas: the instance tree, movie clip
// playheads, frame actions, button events, and frame scripts in the ActionScript subset
// (compiled by AniCraft to a small script IR: variables, expressions, conditions, loops,
// functions and listeners, ADR 0013). It is a line-by-line port of the Rust player
// (crates/doc/src/player.rs, the reference) over the exported data, and the tests run it in a
// JavaScript engine against that player. Nothing here touches the page: canvas.js draws and
// listens to the pointer; this file only computes.
//
// The movie data (see crates/render/src/html.rs): every timeline's frames are baked (tweens,
// easing, motion paths and shape morphs already evaluated), so this file never interpolates.
// A frame is a list of layers, bottom first; a layer holds items: shapes (a path, a paint, a
// stroke), symbol instances (a symbol, its element id, its name and event handlers) and text
// fields (ADR 0066: a box, the authored text and its outline, and the font changed text is drawn
// with), which are instances too: scripts name them, set their text and read it back.
"use strict";
var AniCraft = (function () {
  // Budgets, as in the Rust player: a go-to loop ends in a message, not a hang.
  const MAX_ACTIONS_PER_STEP = 10000;
  const MAX_ENTERS_PER_STEP = 1000;
  const MAX_NODES = 10000;
  const MAX_OUTPUT_PER_STEP = 1000;
  const MAX_EVENTS_PER_STEP = 256;
  const MAX_SOUNDS_PER_STEP = 256;
  const MAX_SYMBOL_DEPTH = 32;
  const MAX_LISTENERS_PER_INSTANCE = 16;
  const ROOT = 0, GRAPHIC = 1, CLIP = 2, BUTTON = 3, TEXT = 4;
  const KINDS = { graphic: GRAPHIC, movieclip: CLIP, button: BUTTON };
  const I = [1, 0, 0, 1, 0, 0];
  const NO_EFFECT = [1, 1, 1, 1, 0, 0, 0, 0];
  const EMPTY = { l: [] };

  // ----- geometry ------------------------------------------------------------------------------

  // `p · q`: apply q first, then p (Affine::mul).
  function mul(p, q) {
    return [p[0] * q[0] + p[2] * q[1], p[1] * q[0] + p[3] * q[1], p[0] * q[2] + p[2] * q[3], p[1] * q[2] + p[3] * q[3], p[0] * q[4] + p[2] * q[5] + p[4], p[1] * q[4] + p[3] * q[5] + p[5]];
  }
  // `m`⁻¹ applied to (x, y) (Affine::inverse), or null when `m` can't be undone.
  function inverseApply(m, x, y) {
    const det = m[0] * m[3] - m[1] * m[2];
    if (!Number.isFinite(det) || Math.abs(det) < 1e-12) return null;
    const ia = m[3] / det, ib = -m[1] / det, ic = -m[2] / det, id = m[0] / det;
    const e = -(ia * m[4] + ic * m[5]), f = -(ib * m[4] + id * m[5]);
    return [ia * x + ic * y + e, ib * x + id * y + f];
  }
  function finite(m) {
    return m.every(Number.isFinite);
  }
  function meanScale(m) {
    return Math.sqrt(Math.abs(m[0] * m[3] - m[1] * m[2]));
  }
  // Affine::decompose and Decomposed::compose (T · R · Shear · S).
  function decompose(m) {
    const [a, b, c, d] = m;
    const sx = Math.sqrt(a * a + b * b), r = Math.atan2(b, a), det = a * d - b * c;
    let sy, k;
    if (sx > 1e-12) {
      sy = det / sx;
      k = Math.abs(det) > 1e-12 ? (a * c + b * d) / det : 0;
    } else {
      sy = Math.sqrt(c * c + d * d);
      k = 0;
    }
    return { tx: m[4], ty: m[5], sx: sx, sy: sy, r: r, k: k };
  }
  function compose(q) {
    const c = Math.cos(q.r), s = Math.sin(q.r);
    return mul(mul(mul([1, 0, 0, 1, q.tx, q.ty], [c, s, -s, c, 0, 0]), [1, 0, q.k, 1, 0, 0]), [q.sx, 0, 0, q.sy, 0, 0]);
  }

  // ----- colour effects (ADR 0056): [r, g, b, a multipliers, r, g, b, a offsets] --------------------

  // ColorTransform::concat: `o` applied after `i`, exactly.
  function concat(o, i) {
    const out = new Array(8);
    for (let k = 0; k < 4; k++) {
      out[k] = o[k] * i[k];
      out[k + 4] = o[k] * i[k + 4] + o[k + 4];
    }
    return out;
  }
  // ColorTransform::apply on straight [r, g, b, a], clamped.
  function applyEffect(fx, c) {
    const out = new Array(4);
    for (let k = 0; k < 4; k++) {
      const v = c[k] * fx[k] + fx[k + 4];
      out[k] = Number.isFinite(v) ? Math.min(Math.max(v, 0), 1) : 0;
    }
    return out;
  }
  // Whether it does more than multiply alpha (a bitmap then needs its pixels recoloured).
  function recolours(fx) {
    return fx[0] !== 1 || fx[1] !== 1 || fx[2] !== 1 || fx[4] !== 0 || fx[5] !== 0 || fx[6] !== 0 || fx[7] !== 0;
  }

  // ----- dictionaries ------------------------------------------------------------------------------

  // String-keyed tables on plain objects (no Map: some engines' Map iterators misbehave).
  function Dict(pairs) {
    this.o = Object.create(null);
    for (const [k, v] of pairs || []) this.o[k] = v;
  }
  Dict.prototype.get = function (k) {
    return this.o[k];
  };
  Dict.prototype.has = function (k) {
    return k in this.o;
  };
  Dict.prototype.set = function (k, v) {
    this.o[k] = v;
  };
  Dict.prototype.delete = function (k) {
    delete this.o[k];
  };
  Dict.prototype.keys = function () {
    return Object.keys(this.o);
  };
  Dict.prototype.values = function () {
    return Object.keys(this.o).map((k) => this.o[k]);
  };
  Dict.prototype.entries = function () {
    return Object.keys(this.o).map((k) => [k, this.o[k]]);
  };

  // ----- instance paths --------------------------------------------------------------------------

  // An instance's place: element ids from the main timeline down ([] is the main timeline).
  function keyOf(path) {
    return path.join(",");
  }
  // The order of Rust's BTreeMap<Vec<ElementId>, _>: element by element, then shorter first.
  function comparePaths(a, b) {
    const n = Math.min(a.length, b.length);
    for (let i = 0; i < n; i++) {
      if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
    }
    return a.length - b.length;
  }
  function samePath(a, b) {
    return a !== null && b !== null && comparePaths(a, b) === 0;
  }

  // ----- the movie --------------------------------------------------------------------------------

  // Unpacks the exported data: frame tables expanded, symbol kinds as numbers.
  function load(movie) {
    for (const tl of movie.timelines) {
      const frames = [];
      for (const [content, count] of tl.f) {
        for (let i = 0; i < count && frames.length < tl.n; i++) frames.push(content);
      }
      tl.frames = frames;
      tl.labels = new Dict(tl.lb || []);
    }
    for (const s of movie.symbols) s.kind = KINDS[s.k] || GRAPHIC;
    return movie;
  }
  function content(M, tl, frame) {
    const c = tl.frames[frame];
    return c === undefined ? EMPTY : M.contents[c] || EMPTY;
  }

  // ----- the player state ------------------------------------------------------------------------

  function newState(seed) {
    return { scene: 0, root: { frame: 0, playing: true }, clips: new Dict(), props: new Dict(), listeners: new Dict(), stage: [], vars: new Dict(), rng: seededRng(seed || 0), mouse: { x: 0, y: 0 }, seq: 0, pointer: null, down: false, hover: null, pressed: null, tick: 0, fields: new Dict(), focus: null, shift: false, selecting: false };
  }
  // The label at or before `frame` (ActionScript's `currentLabel`), or null.
  function currentLabel(tl, frame) {
    let best = null, at = -1;
    for (const [l, f] of tl.lb || []) {
      if (f <= frame && f > at) {
        best = l;
        at = f;
      }
    }
    return best;
  }
  // Each clip entry: { path, frame, playing }; each props entry: { path, ...overrides }; each
  // listeners entry: [{ ev, owner, k, h, path }] in the order added.
  function clipFrame(st, key, len) {
    const c = st.clips.get(key);
    return Math.min(c ? c.frame : 0, Math.max(len, 1) - 1);
  }
  // Where the pointer leaves a button (player.rs `ButtonState`): Up (0), Over (1), Down (2), or
  // pressed on it and dragged off (3, shown Up).
  function buttonState(hover, pressed, down, path) {
    const held = down && samePath(pressed, path);
    return samePath(hover, path) ? (held ? 2 : 1) : held ? 3 : 0;
  }
  // The frame whose sound plays as a button goes from state `a` to `b` (player.rs
  // `button_sound_frame`, ADR 0057): Over's going over it, Up's leaving, Down's pressing, Hit's
  // letting go over it; -1 for none.
  function buttonSoundFrame(a, b) {
    if (a === 0 && b === 1) return 1;
    if (a === 1 && b === 0) return 0;
    if (a === 1 && b === 2) return 2;
    if (a === 2 && b === 1) return 3;
    return -1;
  }
  // Up (0), Over (1) under the pointer, Down (2) while pressed on it.
  function buttonFrame(st, path, len) {
    const s = buttonState(st.hover, st.pressed, st.down, path);
    const f = s === 1 || s === 2 ? s : 0;
    return Math.min(f, Math.max(len, 1) - 1);
  }

  // Property ranges (anicraft_actions::Property::range).
  const RANGES = { x: [-1e6, 1e6], y: [-1e6, 1e6], alpha: [0, 1], visible: [0, 1], rotation: [-36000, 36000], scale_x: [-1000, 1000], scale_y: [-1000, 1000] };
  function setProp(ov, p, value) {
    const r = RANGES[p];
    if (!r || !Number.isFinite(value)) return;
    const v = Math.min(Math.max(value, r[0]), r[1]);
    if (p === "visible") ov.visible = v !== 0;
    else ov[p] = Math.fround(v);
  }
  // Overrides::apply: the element's transform and alpha with the set properties in place.
  function override(ov, m, fx) {
    m = m || I;
    if (ov.x !== undefined || ov.y !== undefined || ov.rotation !== undefined || ov.scale_x !== undefined || ov.scale_y !== undefined) {
      const d = decompose(m);
      if (ov.x !== undefined) d.tx = ov.x;
      if (ov.y !== undefined) d.ty = ov.y;
      if (ov.rotation !== undefined) d.r = (ov.rotation * Math.PI) / 180;
      if (ov.scale_x !== undefined) d.sx = ov.scale_x;
      if (ov.scale_y !== undefined) d.sy = ov.scale_y;
      const c = compose(d);
      if (finite(c)) m = c;
    }
    if (ov.alpha !== undefined) {
      fx = (fx || NO_EFFECT).slice();
      fx[3] = ov.alpha;
    }
    return { m: m, c: fx || NO_EFFECT };
  }

  // ----- the instance tree (player.rs `tree`, `walk`) --------------------------------------------

  function tree(M, st) {
    const tl = M.scenes[st.scene] ? M.scenes[st.scene].tl : 0;
    const out = [{ path: [], key: "", parent: -1, owner: 0, name: "", sym: -1, kind: ROOT, frame: st.root.frame, frames: M.timelines[tl].n, xf: I, local: I, alpha: 1, hittable: true, events: null, gain: 1 }];
    walk(M, st, tl, st.root.frame, 0, 0, I, 1, true, out);
    return out;
  }
  function walk(M, st, tli, frame, parent, owner, xf, depth, hittable, out) {
    if (depth > MAX_SYMBOL_DEPTH) return;
    const base = out[parent].path;
    for (const L of content(M, M.timelines[tli], frame).l) {
      const hit = hittable && L.mk === undefined;
      // Through a camera, a scene's layer is placed by the camera and its depth.
      const lxf = L.v || xf;
      // The volume the sounds inside get: the layers holding them, from the main timeline down.
      const gain = out[parent].gain * (L.sg === undefined ? 1 : L.sg);
      for (const e of L.e) {
        if (out.length >= MAX_NODES) return;
        // Dynamic and input text fields are instances scripts can name (static text isn't).
        if (e.tf) {
          const path = base.concat([e.id]);
          const key = keyOf(path);
          const ov = st.props.get(key);
          const shown = !ov || ov.visible !== false;
          const m = ov ? override(ov, e.m, e.c).m : e.m || I;
          out.push({ path: path, key: key, parent: parent, owner: owner, name: e.n || "", sym: -1, kind: TEXT, frame: 0, frames: 0, xf: mul(lxf, m), local: e.m || I, alpha: e.c ? e.c[3] : 1, hittable: hit && shown, events: null, gain: gain, tf: e.tf });
          continue;
        }
        if (e.y === undefined) continue;
        const sym = M.symbols[e.y];
        if (!sym) continue;
        const path = base.concat([e.id]);
        const key = keyOf(path);
        const ov = st.props.get(key);
        const shown = !ov || ov.visible !== false;
        const m = ov ? override(ov, e.m, e.c).m : e.m || I;
        const len = M.timelines[sym.tl].n;
        const kind = sym.kind;
        const local = kind === GRAPHIC ? e.g || 0 : kind === CLIP ? clipFrame(st, key, len) : buttonFrame(st, path, len);
        const idx = out.length;
        const nxf = mul(lxf, m);
        out.push({ path: path, key: key, parent: parent, owner: owner, name: e.n || "", sym: e.y, kind: kind, frame: local, frames: len, xf: nxf, local: e.m || I, alpha: e.c ? e.c[3] : 1, hittable: hit && shown, events: e.ev || null, gain: gain });
        // A graphic's contents belong to the timeline the graphic is on.
        walk(M, st, sym.tl, local, idx, kind === GRAPHIC ? owner : idx, nxf, depth + 1, hit && shown, out);
      }
    }
  }

  // ----- evaluation (eval.rs `eval_timeline`) ----------------------------------------------------

  // What the player shows at `st`, as a display list like the Rust one: items in drawing
  // order, with the clip they are drawn in (`clip`), the clip their fill forms (`masks`) and
  // the group they fade with; `clips` holds each clip's parent, `groups` each group. `v`, when
  // given, hears every item and where clips and groups open and close (canvas.js draws so).
  function evaluate(M, st, v) {
    const dl = { items: [], clips: [], groups: [] };
    const scene = M.scenes[st.scene];
    const tli = scene ? scene.tl : 0;
    // A scene seen through its camera: the camera's tint colours everything.
    const cx = top(I, 0);
    cx.c = content(M, M.timelines[tli], st.root.frame).cc || NO_EFFECT;
    evalTimeline(M, tli, st.root.frame, cx, dl, { st: st, path: [] }, v || null);
    return dl;
  }
  function top(xf, depth) {
    return { xf: xf, c: NO_EFFECT, depth: depth, clip: null, masks: null, group: null };
  }
  function evalTimeline(M, tli, frame, cx, dl, play, v) {
    if (cx.depth > MAX_SYMBOL_DEPTH) return;
    const tl = M.timelines[tli];
    // One clip per visible mask layer, nested in the enclosing clip; inside mask artwork
    // every layer is artwork.
    const slots = [];
    if (cx.masks === null) {
      for (let k = 0; k < (tl.nm || 0); k++) {
        slots.push(dl.clips.length);
        dl.clips.push(cx.clip);
      }
    }
    let open = null;
    for (const L of content(M, tl, frame).l) {
      let clip = cx.clip, masks = cx.masks;
      const isMask = cx.masks === null && L.mk !== undefined;
      if (isMask) masks = slots[L.mk];
      else if (cx.masks === null && L.c !== undefined) clip = slots[L.c];
      if (v && cx.masks === null && !isMask) {
        // Masked layers draw into their clip; one that never meets its mask stays hidden.
        const want = clip !== cx.clip ? clip : null;
        if (open !== null && open !== want) {
          v.clipEnd(open, false);
          open = null;
        }
        if (want !== null && open === null) {
          v.clipBegin(want);
          open = want;
        }
      }
      if (v && isMask) v.maskBegin(masks);
      const lcx = L.v ? Object.assign({}, cx, { xf: L.v }) : cx;
      for (const e of L.e) evalElement(M, e, lcx, clip, masks, dl, play, v);
      if (v && isMask) {
        v.maskEnd(masks, open === masks);
        if (open === masks) open = null;
      }
    }
    if (v && open !== null) v.clipEnd(open, false);
  }
  function evalElement(M, e, cx, clip, masks, dl, play, v) {
    let m = e.m || I, fx = e.c || NO_EFFECT, child = null;
    // In the player a symbol instance (and a text field) has its own path, and maybe
    // properties a script set.
    if ((e.y !== undefined || e.tf) && play) {
      child = { st: play.st, path: play.path.concat([e.id]) };
      const ov = play.st.props.get(keyOf(child.path));
      if (ov) {
        if (ov.visible === false) return;
        const o = override(ov, m, fx);
        m = o.m;
        fx = o.c;
      }
    }
    const xf = mul(cx.xf, m);
    if (e.tf) return evalField(e.tf, e.n || "", xf, concat(cx.c, fx), cx.group, clip, masks, dl, child, v);
    if (e.y === undefined) {
      const it = { p: e.p, m: xf, f: e.f === undefined ? -1 : e.f, s: e.s || null, k: e.k || null, eo: !!e.eo, c: concat(cx.c, fx), clip: clip, masks: masks, group: cx.group };
      dl.items.push(it);
      if (v) v.item(it);
      return;
    }
    const sym = M.symbols[e.y];
    if (!sym) return;
    const len = M.timelines[sym.tl].n;
    let local;
    if (sym.kind === GRAPHIC) local = e.g || 0;
    else if (child) local = sym.kind === CLIP ? clipFrame(child.st, keyOf(child.path), len) : buttonFrame(child.st, child.path, len);
    // Outside the player (a button's hit area): movie clips where the timeline has them,
    // buttons on Up.
    else local = sym.kind === CLIP ? e.af || 0 : 0;
    // Alpha alone fades the instance as one; an alpha offset stays on each item.
    const fade = fx[7] === 0 ? Math.min(Math.max(fx[3], 0), 1) : 1;
    const blend = e.bl || "normal";
    let color = concat(cx.c, fx), group = cx.group;
    if (masks === null && (fade < 1 - 1e-4 || blend !== "normal" || e.fx)) {
      group = dl.groups.length;
      dl.groups.push({ parent: cx.group, alpha: fade, blend: blend, clip: clip });
      color = concat(cx.c, fx[7] === 0 ? [fx[0], fx[1], fx[2], 1, fx[4], fx[5], fx[6], 0] : fx);
    }
    if (v && group !== cx.group) v.groupBegin(group, dl.groups[group]);
    const inner = { xf: xf, c: color, depth: cx.depth + 1, clip: clip, masks: masks, group: group };
    evalTimeline(M, sym.tl, local, inner, dl, child, v);
    if (v && group !== cx.group) v.groupEnd(group, dl.groups[group]);
  }

  // A dynamic or input field's items (eval.rs `field_items`): its background, the box as a clip
  // around its text (the authored outline until the text changes; the page draws changed text
  // with the browser's fonts), the caret where the player draws one, and its border. `v.field`,
  // when the visitor has it, hears the field as one (with its state) in place of its items.
  function evalField(tf, name, xf, color, group, clip, masks, dl, play, v) {
    const mk = (p, f, s, k, cl, ms) => ({ p: p, m: xf, f: f, s: s, k: k, eo: false, c: color, clip: cl, masks: ms, group: group });
    // Mask artwork: a field's text masks; its box and border don't.
    if (masks !== null) {
      const it = mk(tf.p, tf.f, null, null, clip, masks);
      dl.items.push(it);
      if (v) v.item(it);
      return;
    }
    const key = play ? keyOf(play.path) : null;
    const fs = play ? play.st.fields.get(key) : undefined;
    const focused = !!play && play.st.focus !== null && samePath(play.st.focus, play.path);
    const text = fs && fs.text !== null ? fs.text : null;
    const items = [];
    if (tf.bg !== undefined) items.push(mk(tf.r, tf.bg, null, null, clip, null));
    const inner = dl.clips.length;
    dl.clips.push(clip);
    items.push(mk(tf.r, tf.kp, null, null, clip, inner));
    // Changed text has no outline here (it isn't laid out): the page draws it.
    if (text === null) items.push(mk(tf.p, tf.f, null, null, inner, null));
    if (focused && fs && tf.k === "input" && fs.caret === fs.anchor) items.push(mk(tf.cr, tf.f, null, null, inner, null));
    if (tf.bd) items.push(mk(tf.r, -1, tf.bd, tf.bk || null, clip, null));
    for (const it of items) dl.items.push(it);
    if (!v) return;
    if (v.field) v.field({ tf: tf, name: name, m: xf, c: color, clip: clip, group: group, key: key, text: text, focused: focused, items: items });
    else for (const it of items) v.item(it);
  }

  // ----- hit testing (player.rs `hit_in`) --------------------------------------------------------

  // `host.inFill(path, xf, x, y, evenOdd)` and `host.nearStroke(path, xf, x, y, reach)` answer
  // for one path; canvas.js asks the canvas, the tests a polygon test.
  function hitIn(M, st, nodes, host, x, y) {
    for (let i = nodes.length - 1; i >= 0; i--) {
      const n = nodes[i];
      const interactive = n.kind === BUTTON || (n.kind === CLIP && (n.events || st.listeners.has(n.key)));
      if (!n.hittable || !interactive) continue;
      const sym = M.symbols[n.sym];
      if (!sym) continue;
      let inside = false;
      const probe = {
        item: function (it) {
          if (inside || it.masks !== null || !finite(it.m)) return;
          const fill = it.f >= 0 && host.inFill(it.p, it.m, x, y, it.eo);
          inside = fill || (it.s !== null && host.nearStroke(it.p, it.m, x, y, it.s[4] * 0.5 * meanScale(it.m)));
        },
        clipBegin: function () {},
        clipEnd: function () {},
        maskBegin: function () {},
        maskEnd: function () {},
        groupBegin: function () {},
        groupEnd: function () {},
      };
      const dl = { items: [], clips: [], groups: [] };
      // The Hit frame (the fourth) when a button has one, else its Up frame.
      if (n.kind === BUTTON) evalTimeline(M, sym.tl, n.frames > 3 ? 3 : 0, top(n.xf, 1), dl, null, probe);
      else evalTimeline(M, sym.tl, n.frame, top(n.xf, 1), dl, { st: st, path: n.path }, probe);
      if (inside) return n.path;
    }
    return null;
  }

  // ----- frame scripts in the ActionScript subset (ADR 0013 tiers 2 and 2.5) -----------------------
  //
  // A line-by-line port of crates/actions/src/as3/run.rs (the evaluator) and the script side of
  // player.rs (`Script`, `run_code`, `listen`, `property`). Values are JavaScript's own
  // primitives, whose operators are ECMAScript's, as the Rust evaluator implements them.

  const FUEL_PER_RUN = 100000;
  const MAX_CALL_DEPTH = 16;
  const MAX_TEXT = 16384;
  const MAX_TRACE_CHARS = 1000;
  const MAX_FUEL_PER_STEP = 1000000;
  const MAX_STAGE_LISTENERS = 64;
  const MAX_FRAME = 100000;
  const MAX_LABEL_CHARS = 256;
  const PROP_NAMES = { x: "x", y: "y", alpha: "alpha", visible: "visible", rotation: "rotation", scale_x: "scaleX", scale_y: "scaleY" };
  const PROP_LABELS = { x: "x", y: "y", alpha: "alpha", visible: "visible", rotation: "rotation", scale_x: "scale x", scale_y: "scale y" };
  const READ_NAMES = { current_frame: "currentFrame", total_frames: "totalFrames", current_label: "currentLabel", mouse_x: "mouseX", mouse_y: "mouseY", name: "name", text: "text", length: "length" };
  // Longest text a field holds (code points, as lib.rs `MAX_FIELD_CHARS`).
  const MAX_FIELD_CHARS = 16384;
  // How messages name a kind of instance (player.rs `kind_name`).
  function kindName(k) {
    return ["the main timeline", "a graphic", "a movie clip", "a button", "a text field"][k] || "an instance";
  }

  // Marsaglia's xorshift128, seeded as player.rs `Rng::seeded`.
  function seededRng(seed) {
    const lo = seed % 4294967296 >>> 0, hi = Math.floor(seed / 4294967296) >>> 0;
    const s = [(lo ^ 0x9e3779b9) >>> 0, (hi ^ 0x243f6a88) >>> 0, 0xb7e15162, 0x3c6ef372];
    if (s[0] === 0 && s[1] === 0 && s[2] === 0 && s[3] === 0) s[0] = 1;
    return s;
  }
  function nextU32(s) {
    const x = s[0], w = s[3];
    const t = (x ^ (x << 11)) >>> 0;
    const w2 = (w ^ (w >>> 19) ^ (t ^ (t >>> 8))) >>> 0;
    s[0] = s[1];
    s[1] = s[2];
    s[2] = w;
    s[3] = w2;
    return w2;
  }
  function nextF64(s) {
    const a = nextU32(s) >>> 5, b = nextU32(s) >>> 6;
    return (a * 67108864 + b) / 9007199254740992;
  }

  // A literal of the IR's JSON: "u", "null", "nan", {b}, {n}, {s}, {inf}.
  function lit(v) {
    if (typeof v === "string") return v === "null" ? null : v === "nan" ? Number.NaN : undefined;
    if ("b" in v) return v.b;
    if ("n" in v) return v.n;
    if ("s" in v) return v.s;
    if ("inf" in v) return v.inf ? -Infinity : Infinity;
    return undefined;
  }
  function defaultOf(t) {
    switch (t) {
      case "number":
        return Number.NaN;
      case "int":
      case "uint":
        return 0;
      case "boolean":
        return false;
      case "string":
        return null;
      default:
        return undefined;
    }
  }
  function coerce(t, v) {
    switch (t) {
      case "number":
        return +v;
      case "int":
        return +v >> 0;
      case "uint":
        return +v >>> 0;
      case "boolean":
        return !!v;
      case "string":
        return v === null || v === undefined ? null : String(v);
      default:
        return v;
    }
  }
  // Math.round as the Rust evaluator computes it.
  function round(x) {
    if (!Number.isFinite(x)) return x;
    const f = Math.floor(x);
    return x - f >= 0.5 ? f + 1 : f;
  }
  function minMax(vals, max) {
    let out = max ? -Infinity : Infinity;
    for (const v of vals) {
      if (Number.isNaN(v)) return Number.NaN;
      if ((max && v > out) || (!max && v < out)) out = v;
    }
    return out;
  }
  function traceLine(s) {
    if (s.length <= MAX_TRACE_CHARS) return s;
    let n = MAX_TRACE_CHARS;
    const u = s.charCodeAt(n - 1);
    if (u >= 0xd800 && u < 0xdc00) n -= 1;
    return s.slice(0, n) + "…";
  }
  // Rust's `char::is_whitespace` and `is_control`, for labels.
  function rustSpace(c) {
    const u = c.codePointAt(0);
    return (u >= 9 && u <= 13) || u === 32 || u === 0x85 || u === 0xa0 || u === 0x1680 || (u >= 0x2000 && u <= 0x200a) || u === 0x2028 || u === 0x2029 || u === 0x202f || u === 0x205f || u === 0x3000;
  }
  function rustControl(c) {
    const u = c.codePointAt(0);
    return u < 0x20 || (u >= 0x7f && u <= 0x9f);
  }
  function labelOk(s) {
    const cs = Array.from(s);
    return cs.length >= 1 && cs.length <= MAX_LABEL_CHARS && !cs.some(rustControl) && cs.some((c) => !rustSpace(c));
  }
  function shown(v) {
    return typeof v === "string" ? '"' + Array.from(v).slice(0, 40).join("") + '"' : String(v);
  }
  // A number as Rust's `Display` writes it (no exponent), for action descriptions.
  function rustNum(v) {
    if (v === 0 || !Number.isFinite(v)) return String(v);
    const a = Math.abs(v);
    if (a >= 1e-6 && a < 1e21) return String(v);
    const [m, e] = v.toExponential().split("e");
    const neg = m[0] === "-";
    const digits = m.replace("-", "").replace(".", "");
    const exp = Number.parseInt(e, 10);
    let out;
    if (exp < 0) out = "0." + "0".repeat(-exp - 1) + digits;
    else out = digits + "0".repeat(Math.max(exp - (digits.length - 1), 0));
    return (neg ? "-" : "") + out;
  }
  // A name path as the exported actions' `t` steps (Target::segments).
  function segsOf(path) {
    const s = (path || "").trim();
    const out = [];
    if (s === "") return out;
    s.split(".").forEach((part, i) => {
      const p = part.trim();
      if (p === "this" && i === 0) return;
      if ((p === "root" || p === "_root") && i === 0) out.push("^");
      else if (p === "parent" || p === "_parent") out.push("..");
      else out.push(p);
    });
    return out;
  }
  function placeText(at) {
    const base = { stage: "stage", target: "e.target", current: "e.currentTarget" }[at.base] || "";
    const path = (at.path || "").trim();
    const p = path === "" || path === "this" ? "this" : path;
    if (base === "") return p;
    return p === "this" ? base : base + "." + path;
  }
  // An action the evaluator builds, in the exported form, with Action::describe's text.
  function makeAction(m, path, frame, scene) {
    const ops = { stop: "stop", play: "play", goto_and_play: "goto_and_play", goto_and_stop: "goto_and_stop", next_frame: "next_frame", prev_frame: "prev_frame", next_scene: "next_scene", prev_scene: "prev_scene" };
    const t = (path || "").trim();
    const on = t === "" || t === "this" ? "" : " (" + t + ")";
    const a = { op: ops[m], t: segsOf(path) };
    const fr = typeof frame === "number" ? "frame " + (frame + 1) : "“" + frame + "”";
    const sc = scene === null ? "" : " in “" + scene + "”";
    switch (m) {
      case "stop":
        a.d = "Stop" + on;
        break;
      case "play":
        a.d = "Play" + on;
        break;
      case "goto_and_play":
      case "goto_and_stop":
        a.f = frame;
        if (scene !== null) a.s = scene;
        a.d = "Go to " + fr + sc + (m === "goto_and_play" ? " and play" : " and stop") + on;
        break;
      case "next_frame":
        a.d = "Next frame" + on;
        break;
      case "prev_frame":
        a.d = "Previous frame" + on;
        break;
      case "next_scene":
        a.d = "Next scene";
        break;
      case "prev_scene":
        a.d = "Previous scene";
        break;
    }
    return a;
  }
  // The shortest decimal that reads back as the same f32 (player.rs `short`).
  function short(v) {
    const f = Math.fround(v);
    if (!Number.isFinite(f) || f === 0) return f;
    for (let p = 1; p <= 9; p++) {
      const s = Number(f.toPrecision(p));
      if (Math.fround(s) === f) return s;
    }
    return f;
  }

  function Stop(kind, line, message) {
    this.kind = kind;
    this.line = line;
    this.message = message;
  }
  Stop.prototype.describe = function () {
    if (this.kind === "fuel") return "it ran more than " + FUEL_PER_RUN + " steps (an endless loop?)";
    if (this.kind === "halt") return "the frame's budget ran out";
    return this.line > 0 ? "line " + this.line + ": " + this.message : this.message;
  };

  const NORMAL = { k: 0 }, BREAK = { k: 1 }, CONTINUE = { k: 2 };

  // The evaluator over a world `w` (see Script below).
  function Vm(w, fuel) {
    this.w = w;
    this.fuel = fuel;
    this.depth = 0;
    this.line = 0;
  }
  Vm.prototype.tick = function (n) {
    if (this.fuel < n) {
      this.fuel = 0;
      throw new Stop("fuel");
    }
    this.fuel -= n;
  };
  Vm.prototype.err = function (message) {
    throw new Stop("error", this.line, message);
  };
  // A world call's failure: a message stops the run there; null means the world halted.
  Vm.prototype.lift = function (line, r) {
    if (r === undefined) return;
    if (r === null) throw new Stop("halt");
    throw new Stop("error", line, r);
  };
  Vm.prototype.wrap = function (r) {
    if (r.err !== undefined) throw new Stop("error", this.line, r.err);
    return r.ok;
  };
  Vm.prototype.block = function (list, fr) {
    for (const s of list) {
      const f = this.stmt(s, fr);
      if (f.k !== 0) return f;
    }
    return NORMAL;
  };
  Vm.prototype.stmt = function (st, fr) {
    this.tick(1);
    this.line = st.l;
    switch (st.s) {
      case "do":
        this.expr(st.x, fr);
        return NORMAL;
      case "if":
        return this.block(this.expr(st.c, fr) ? st.a : st.b, fr);
      case "loop": {
        let test = st.first;
        for (;;) {
          if (test && st.c !== undefined && st.c !== null) {
            this.line = st.l;
            if (!this.expr(st.c, fr)) break;
          }
          test = true;
          this.tick(1);
          const f = this.block(st.body, fr);
          if (f.k === 1) break;
          if (f.k === 3) return f;
          if (st.next !== undefined && st.next !== null) {
            this.line = st.l;
            this.expr(st.next, fr);
          }
        }
        return NORMAL;
      }
      case "break":
        return BREAK;
      case "continue":
        return CONTINUE;
      case "return":
        return { k: 3, v: st.v !== undefined && st.v !== null ? this.expr(st.v, fr) : undefined };
      case "act": {
        const fv = st.args.length > 0 ? this.expr(st.args[0], fr) : undefined;
        const has = st.args.length > 0;
        const sv = st.args.length > 1 ? this.expr(st.args[1], fr) : undefined;
        const hasScene = st.args.length > 1;
        this.line = st.l;
        let frame = 0;
        if (has) {
          if (typeof fv === "string" && labelOk(fv)) frame = fv;
          else if (typeof fv === "number" && Number.isInteger(fv) && fv >= 1 && fv <= MAX_FRAME + 1) frame = fv - 1;
          else this.err("`" + methodName(st.m) + "` needs a frame number from 1 or a label, not " + shown(fv));
        }
        let scene = null;
        if (hasScene && sv !== null && sv !== undefined) {
          if (typeof sv === "string" && labelOk(sv)) scene = sv;
          else this.err("the scene is its name, not " + shown(sv));
        }
        this.lift(st.l, this.w.act(st.at.base || "this", makeAction(st.m, st.at.path, frame, scene)));
        return NORMAL;
      }
      case "trace": {
        const parts = [];
        for (const a of st.args) parts.push(String(this.expr(a, fr)));
        this.w.trace(traceLine(parts.join(" ")));
        return NORMAL;
      }
      case "listen": {
        let func;
        if (st.f.named !== undefined) {
          func = this.w.fn(st.f.named);
          if (func === undefined) this.err("there's no function `" + st.f.named + "` on this timeline");
        } else {
          func = fr.inner[st.f.inline];
          if (func === undefined) this.err("the listener's function is missing");
        }
        this.lift(st.l, this.w.listen(st.at, st.ev, func, st.add));
        return NORMAL;
      }
    }
    return NORMAL;
  };
  function methodName(m) {
    return { stop: "stop", play: "play", goto_and_play: "gotoAndPlay", goto_and_stop: "gotoAndStop", next_frame: "nextFrame", prev_frame: "prevFrame", next_scene: "nextScene", prev_scene: "prevScene" }[m] || m;
  }
  Vm.prototype.text = function (s) {
    const n = s.length;
    if (n > MAX_TEXT) this.err("this text would be longer than " + MAX_TEXT + " characters");
    this.tick(Math.floor(n / 64));
    return s;
  };
  Vm.prototype.binary = function (op, a, b) {
    switch (op) {
      case "add":
        if (typeof a === "string" || typeof b === "string") return this.text(String(a) + String(b));
        return +a + +b;
      case "sub":
        return +a - +b;
      case "mul":
        return +a * +b;
      case "div":
        return +a / +b;
      case "mod":
        return +a % +b;
      case "lt":
        return a < b;
      case "gt":
        return a > b;
      case "le":
        return a <= b;
      case "ge":
        return a >= b;
      case "eq":
        return a == b;
      case "ne":
        return a != b;
      case "strict_eq":
        return a === b;
      case "strict_ne":
        return a !== b;
    }
    return undefined;
  };
  Vm.prototype.load = function (to, fr) {
    if (to.k === "local") return fr.locals[to.i];
    if (to.k === "var") return this.wrap(this.w.read(to.n));
    if (to.k === "text") return this.wrap(this.w.get(to.at, "text"));
    return this.wrap(this.w.get(to.at, to.p));
  };
  Vm.prototype.store = function (to, v, fr) {
    if (to.k === "local") {
      const c = coerce(fr.types[to.i] || "any", v);
      if (to.i < fr.locals.length) fr.locals[to.i] = c;
      return c;
    }
    if (to.k === "var") {
      this.wrap(this.w.write(to.n, v));
      return this.wrap(this.w.read(to.n));
    }
    if (to.k === "text") {
      if (v === null || v === undefined) this.err("`text` is set to text, not " + String(v));
      this.lift(this.line, this.w.setText(to.at, String(v)));
      return v;
    }
    const n = +v;
    if (!Number.isFinite(n)) this.err("`" + PROP_NAMES[to.p] + "` is set to a number, not " + shown(v));
    this.lift(this.line, this.w.set(to.at, to.p, n));
    return v;
  };
  Vm.prototype.expr = function (e, fr) {
    this.tick(1);
    switch (e.e) {
      case "lit":
        return lit(e.v);
      case "local":
        return fr.locals[e.i];
      case "var":
        return this.wrap(this.w.read(e.n));
      case "get":
        return this.wrap(this.w.get(e.at, e.p));
      case "key":
        return this.wrap(this.w.keyCode());
      case "event":
        return this.err("the event can only be passed to a function's event parameter");
      case "un": {
        const v = this.expr(e.x, fr);
        return e.op === "neg" ? -v : e.op === "plus" ? +v : !v;
      }
      case "bin": {
        const a = this.expr(e.a, fr);
        const b = this.expr(e.b, fr);
        return this.binary(e.op, a, b);
      }
      case "and": {
        const a = this.expr(e.a, fr);
        return a ? this.expr(e.b, fr) : a;
      }
      case "or": {
        const a = this.expr(e.a, fr);
        return a ? a : this.expr(e.b, fr);
      }
      case "cond":
        return this.expr(e.c, fr) ? this.expr(e.a, fr) : this.expr(e.b, fr);
      case "math": {
        const v = [];
        for (const a of e.args) v.push(+this.expr(a, fr));
        const x = v.length > 0 ? v[0] : Number.NaN;
        switch (e.f) {
          case "min":
            return minMax(v, false);
          case "max":
            return minMax(v, true);
          case "abs":
            return Math.abs(x);
          case "floor":
            return Math.floor(x);
          case "ceil":
            return Math.ceil(x);
          case "round":
            return round(x);
          case "random":
            return this.w.random();
        }
        return Number.NaN;
      }
      case "call": {
        const f = this.w.fn(e.f);
        if (f === undefined) this.err("there's no function `" + e.f + "` on this timeline");
        const line = this.line;
        const v = this.call(f, e.args, fr);
        this.line = line;
        return v;
      }
      case "same":
        return this.wrap(this.w.same(e.a, e.b)) !== e.ne;
      case "set": {
        let nv;
        if (e.op === null || e.op === undefined) nv = this.expr(e.v, fr);
        else {
          const old = this.load(e.to, fr);
          const rhs = this.expr(e.v, fr);
          nv = this.binary(e.op, old, rhs);
        }
        return this.store(e.to, nv, fr);
      }
      case "step": {
        const old = +this.load(e.to, fr);
        const stored = this.store(e.to, old + e.d, fr);
        return e.post ? old : stored;
      }
      case "cast":
        return coerce(e.t, this.expr(e.x, fr));
    }
    return undefined;
  };
  Vm.prototype.call = function (f, args, caller) {
    if (this.depth >= MAX_CALL_DEPTH) this.err("functions call each other more than " + MAX_CALL_DEPTH + " deep");
    const params = f.params;
    const required = params.filter((p) => p.d === undefined).length;
    if (args.length < required || args.length > params.length) {
      const want = required === params.length ? String(required) : required + " to " + params.length;
      this.err("`" + f.name + "` takes " + want + " argument" + (params.length === 1 ? "" : "s") + ", not " + args.length);
    }
    const locals = f.locals.map(defaultOf);
    for (let i = 0; i < params.length; i++) {
      const p = params[i];
      let v;
      if (i < args.length) {
        const a = args[i];
        if (a.e === "event") {
          if (p.ev || p.t === "any") v = null;
          else this.err("the event can only be passed to a function's event parameter");
        } else v = this.expr(a, caller);
      } else v = p.d !== undefined ? lit(p.d) : undefined;
      if (i < locals.length) locals[i] = coerce(p.t, v);
    }
    this.depth += 1;
    this.line = f.line;
    const fr = { locals: locals, types: f.locals, inner: f.inner || [] };
    let flow;
    try {
      flow = this.block(f.body, fr);
    } finally {
      this.depth -= 1;
    }
    const v = flow.k === 3 ? flow.v : undefined;
    return f.ret !== undefined && f.ret !== null ? coerce(f.ret, v) : undefined;
  };
  // Runs `go(vm)`; [fuel used, null or the Stop].
  function runVm(w, fuel, go) {
    const vm = new Vm(w, fuel);
    try {
      go(vm);
      return [fuel - vm.fuel, null];
    } catch (e) {
      if (e instanceof Stop) return [fuel - vm.fuel, e];
      throw e;
    }
  }
  function runScriptCode(w, code, fuel) {
    return runVm(w, fuel, (vm) => vm.block(code.b, { locals: [], types: [], inner: code.i || [] }));
  }
  function runListenerCode(w, f, fuel) {
    return runVm(w, fuel, (vm) => {
      vm.line = f.line;
      vm.call(f, [{ e: "event" }], { locals: [], types: [], inner: [] });
    });
  }

  // ----- a running script's view of the movie (player.rs `Script`) ---------------------------

  // A script running on timeline `owner` (an instance path), handling event `ev` (or null).
  function Script(run, owner, tl, ev) {
    this.r = run;
    this.owner = owner;
    this.tl = tl;
    this.ev = ev;
  }
  Script.prototype.from = function () {
    this.r.refresh();
    const i = this.r.indexOf(this.owner);
    return i < 0 ? { err: "the script's timeline has left the stage" } : { ok: i };
  };
  Script.prototype.act = function (base, a) {
    if (!this.r.budget()) return null;
    const p = this.r.basePath(this.owner, this.ev, base);
    if (p.err !== undefined) return p.err;
    this.r.exec(p.ok, a, true);
    return this.r.halted ? null : undefined;
  };
  Script.prototype.get = function (at, p) {
    const f = this.from();
    if (f.err !== undefined) return f;
    const i = this.r.place(f.ok, this.ev, at);
    if (i.err !== undefined) return i;
    return this.r.property(i.ok, p);
  };
  Script.prototype.set = function (at, p, v) {
    if (at.base === "stage") return "the stage's properties can't be set here";
    const r = RANGES[p];
    v = Math.min(Math.max(v, r[0]), r[1]) + 0;
    const t = (at.path || "").trim();
    const on = t === "" || t === "this" ? "" : " (" + t + ")";
    const a = { op: "set_property", t: segsOf(at.path), p: p, v: v, pl: PROP_LABELS[p], d: "Set " + PROP_LABELS[p] + " to " + rustNum(v) + on };
    return this.act(at.base || "this", a);
  };
  // `field.text = …` (player.rs `set_text`).
  Script.prototype.setText = function (at, text) {
    if (!this.r.budget()) return null;
    const f = this.from();
    if (f.err !== undefined) return f.err;
    const i = this.r.place(f.ok, this.ev, at);
    if (i.err !== undefined) return i.err;
    const n = i.ok === null ? undefined : this.r.nodes[i.ok];
    if (!n) return "the stage isn't a text field";
    if (n.kind !== TEXT) return "“" + n.name + "” is " + kindName(n.kind) + ", not a text field";
    this.r.setFieldText(n.path, text);
    return undefined;
  };
  Script.prototype.trace = function (line) {
    if (this.r.budget()) this.r.out(line);
  };
  Script.prototype.read = function (name) {
    const vars = this.r.st.vars.get(keyOf(this.owner));
    if (vars && name in vars.v) return { ok: vars.v[name] };
    const vt = this.tl.vt || {};
    if (Object.prototype.hasOwnProperty.call(vt, name)) return { ok: defaultOf(vt[name]) };
    return { err: "`" + name + "` isn't a variable of this timeline" };
  };
  Script.prototype.write = function (name, v) {
    const vt = this.tl.vt || {};
    if (!Object.prototype.hasOwnProperty.call(vt, name)) return { err: "`" + name + "` isn't a variable of this timeline" };
    const k = keyOf(this.owner);
    let vars = this.r.st.vars.get(k);
    if (!vars) {
      vars = { path: this.owner, v: Object.create(null) };
      this.r.st.vars.set(k, vars);
    }
    vars.v[name] = coerce(vt[name], v);
    return { ok: undefined };
  };
  Script.prototype.fn = function (name) {
    const fns = this.tl.fn || {};
    return Object.prototype.hasOwnProperty.call(fns, name) ? fns[name] : undefined;
  };
  Script.prototype.listen = function (at, ev, f, add) {
    if (!this.r.budget()) return null;
    this.r.listenTo(this.owner, this.ev, at, ev, f, add);
    return undefined;
  };
  Script.prototype.random = function () {
    return nextF64(this.r.st.rng);
  };
  Script.prototype.same = function (a, b) {
    const f = this.from();
    if (f.err !== undefined) return f;
    const x = this.r.place(f.ok, this.ev, a);
    if (x.err !== undefined) return x;
    const y = this.r.place(f.ok, this.ev, b);
    if (y.err !== undefined) return y;
    return { ok: x.ok === y.ok };
  };
  Script.prototype.keyCode = function () {
    return this.ev && this.ev.key !== null ? { ok: this.ev.key } : { err: "this isn't a keyboard event" };
  };

  // ----- the player (player.rs `Run`) ------------------------------------------------------------

  // A player for `movie` (the exported data, as parsed JSON). `host` answers hit tests (see
  // hitIn). `start()` shows the first frame, `step(events)` moves one frame on, `input(events)`
  // routes pointer events between frames; each returns { output, sounds }: the lines the movie
  // wrote (traces, and why an action could not run) and the sounds of the keyframes entered.
  // Events: { type: "move" | "down" | "up", x, y } (stage pixels), { type: "leave" },
  // { type: "keydown" | "keyup", code, dom } (an ActionScript key code; `dom`: it comes from one
  // of the page's field elements, which has edited the field already), and, for the tests' small
  // editing mirror, { type: "char", text } and { type: "selectall" } (see "text fields" below).
  // `seed` seeds Math.random().
  function Player(movie, host, seed) {
    this.M = load(movie);
    this.host = host;
    this.seed = seed || 0;
    this.st = newState(this.seed);
  }
  Player.prototype.start = function () {
    this.st = newState(this.seed);
    const r = new Run(this);
    r.refresh();
    r.pending.unshift([]);
    r.settle();
    return r.finish();
  };
  Player.prototype.step = function (events) {
    const r = new Run(this);
    this.st.tick += 1;
    r.refresh();
    r.rememberGraphics();
    r.advance();
    r.enterFrame();
    r.settle();
    for (const e of (events || []).slice(0, MAX_EVENTS_PER_STEP)) r.event(e);
    return r.finish();
  };
  Player.prototype.input = function (events) {
    const r = new Run(this);
    r.refresh();
    r.rememberGraphics();
    for (const e of (events || []).slice(0, MAX_EVENTS_PER_STEP)) r.event(e);
    return r.finish();
  };
  // The node of the field at instance-path key `key` on the stage now, or null.
  Player.prototype.fieldNode = function (key) {
    const n = tree(this.M, this.st).find((x) => x.key === key && x.kind === TEXT);
    return n || null;
  };
  // The viewer changed input field `key`'s text in the page's element: the text the field keeps
  // (the element's value as the editor would let it in), or null when there is no such input
  // field. No script runs: listeners hear the keys, as they do in the Rust player.
  Player.prototype.typed = function (key, value) {
    const n = this.fieldNode(key);
    if (n === null || n.tf.k !== "input") return null;
    const v = typedText(n.tf, String(value === null || value === undefined ? "" : value));
    let fs = this.st.fields.get(key);
    if (!fs) {
      fs = newField(n.path);
      this.st.fields.set(key, fs);
    }
    fs.text = v;
    fs.caret = fs.anchor = Array.from(v).length;
    return v;
  };
  // The keyboard went to field `key` (an input field, or a selectable dynamic one, on the stage
  // and reachable), or away from every field (null). True when the field has it.
  Player.prototype.focus = function (key) {
    const st = this.st;
    st.selecting = false;
    const n = key === null || key === undefined ? null : this.fieldNode(key);
    if (n === null || !n.hittable || !focusable(n.tf)) {
      st.focus = null;
      return false;
    }
    st.focus = n.path;
    if (!st.fields.has(n.key)) {
      const fs = newField(n.path);
      fs.caret = fs.anchor = Array.from(n.tf.t).length;
      st.fields.set(n.key, fs);
    }
    return true;
  };
  // Field `key`'s text now (what a script or the viewer set, or the authored text), or null.
  Player.prototype.fieldText = function (key) {
    const n = this.fieldNode(key);
    return n === null ? null : fieldValue(this.st.fields.get(key), n.tf);
  };
  // What the player shows now (see evaluate).
  Player.prototype.evaluate = function (v) {
    return evaluate(this.M, this.st, v);
  };
  // The state as plain data, ordered like the Rust player's (for tests and debugging).
  Player.prototype.snapshot = function () {
    const st = this.st;
    const clips = Array.from(st.clips.values()).sort((a, b) => comparePaths(a.path, b.path)).map((c) => [c.path, c.frame, c.playing]);
    const props = Array.from(st.props.values()).sort((a, b) => comparePaths(a.path, b.path)).map((p) => {
      const o = {};
      for (const k of Object.keys(p)) if (k !== "path") o[k] = p[k];
      return [p.path, o];
    });
    const listeners = Array.from(st.listeners.values()).filter((l) => l.length > 0).sort((a, b) => comparePaths(a[0].path, b[0].path)).map((l) => [l[0].path, l.map((x) => [x.ev, x.owner, x.k])]);
    const kind = (v) => (v === undefined ? "u" : v === null ? "null" : typeof v === "boolean" ? "b" : typeof v === "number" ? "n" : "s");
    const vars = Array.from(st.vars.values()).sort((a, b) => comparePaths(a.path, b.path)).map((x) => [x.path, Object.keys(x.v).sort((a, b) => (a < b ? -1 : a > b ? 1 : 0)).map((k) => [k, kind(x.v[k]) + ":" + String(x.v[k])])]);
    const stage = st.stage.map((x) => [x.ev, x.owner, x.k]);
    const fields = Array.from(st.fields.values()).sort((a, b) => comparePaths(a.path, b.path)).map((f) => [f.path, f.text, f.caret, f.anchor]);
    return { scene: st.scene, frame: st.root.frame, playing: st.root.playing, clips: clips, props: props, listeners: listeners, stage: stage, vars: vars, hover: st.hover, pressed: st.pressed, down: st.down, fields: fields, focus: st.focus, selecting: st.selecting, shift: st.shift };
  };

  function Run(player) {
    this.M = player.M;
    this.host = player.host;
    this.st = player.st;
    this.nodes = [];
    this.dirty = true;
    this.pending = [];
    this.actions = 0;
    this.enters = 0;
    this.halted = false;
    this.output = [];
    this.dropped = 0;
    this.sounds = [];
    this.fuel = MAX_FUEL_PER_STEP;
    // The frame each graphic showed when the step began: one on another frame at its end has
    // entered it (and starts its sounds).
    this.graphics = new Dict();
  }
  Run.prototype.rememberGraphics = function () {
    this.graphics = new Dict();
    for (const n of this.nodes) if (n.kind === GRAPHIC) this.graphics.set(n.key, n.frame);
  };
  // The sounds of timeline `tl`'s keyframes starting at `frame`, as
  // [sound, sync, repeat, loop, span, gain, envelope], the gain times `gain`.
  // On a button's frame (`button`) a stream plays as an event: the frame has no span to stream
  // over (ADR 0057).
  Run.prototype.soundsAt = function (tl, frame, gain, button) {
    for (const s of (tl.snd && tl.snd[frame]) || []) {
      if (this.sounds.length >= MAX_SOUNDS_PER_STEP) return;
      const sync = button && s[1] === "stream" ? "event" : s[1];
      this.sounds.push([s[0], sync, s[2], s[3], s[4], (s[5] === undefined ? 1 : s[5]) * gain, s[6] || null]);
    }
  };
  // Where the pointer is, for buttonSounds.
  Run.prototype.pointerNow = function () {
    return { hover: this.st.hover, pressed: this.st.pressed, down: this.st.down };
  };
  // Starts the sounds of the buttons whose state changed since `before` (pointerNow then).
  Run.prototype.buttonSounds = function (before) {
    const st = this.st;
    const same = (a, b) => (a === null ? b === null : samePath(a, b));
    if (same(before.hover, st.hover) && same(before.pressed, st.pressed) && before.down === st.down) return;
    this.refresh();
    const paths = [];
    for (const p of [before.hover, before.pressed, st.hover, st.pressed]) if (p !== null && !paths.some((q) => samePath(q, p))) paths.push(p);
    for (const path of paths) {
      const f = buttonSoundFrame(buttonState(before.hover, before.pressed, before.down, path), buttonState(st.hover, st.pressed, st.down, path));
      if (f < 0) continue;
      const i = this.indexOf(path);
      if (i < 0 || this.nodes[i].kind !== BUTTON) continue;
      const s = this.M.symbols[this.nodes[i].sym];
      if (s) this.soundsAt(this.M.timelines[s.tl], f, this.nodes[i].gain, true);
    }
  };
  Run.prototype.out = function (line) {
    if (this.output.length < MAX_OUTPUT_PER_STEP) this.output.push(line);
    else this.dropped += 1;
  };
  Run.prototype.halt = function (why) {
    if (!this.halted) {
      this.halted = true;
      this.out("Stopped running actions this frame: " + why + " (a go-to loop?)");
    }
  };
  Run.prototype.sceneTimeline = function (s) {
    const sc = this.M.scenes[s];
    return this.M.timelines[sc ? sc.tl : 0];
  };
  // Rebuilds the instance tree after a change: movie clips that appeared start (and enter
  // their first frame), those that left are forgotten.
  Run.prototype.refresh = function () {
    if (!this.dirty) return;
    this.dirty = false;
    const st = this.st;
    if (st.scene >= this.M.scenes.length) st.scene = 0;
    const n = this.sceneTimeline(st.scene).n;
    st.root.frame = Math.min(st.root.frame, Math.max(n - 1, 0));
    this.nodes = tree(this.M, st);
    const live = new Dict();
    const all = new Dict();
    for (const nd of this.nodes) {
      all.set(nd.key, true);
      if (nd.kind === CLIP && !live.has(nd.key)) live.set(nd.key, nd.path);
    }
    for (const k of Array.from(st.clips.keys())) if (!live.has(k)) st.clips.delete(k);
    const born = Array.from(live.entries()).filter(([k]) => !st.clips.has(k)).sort((a, b) => comparePaths(a[1], b[1]));
    for (const [k, path] of born) {
      st.clips.set(k, { path: path, frame: 0, playing: true });
      this.queue(path);
    }
    for (const k of Array.from(st.props.keys())) if (!all.has(k)) st.props.delete(k);
    // A listener goes with its instance, and with the timeline whose script added it.
    for (const [k, list] of st.listeners.entries()) {
      const kept = list.filter((l) => all.has(keyOf(l.owner)));
      if (!all.has(k) || kept.length === 0) st.listeners.delete(k);
      else st.listeners.set(k, kept);
    }
    st.stage = st.stage.filter((l) => all.has(keyOf(l.owner)));
    // A timeline's variables go with its instance, and a field's text with the field.
    for (const k of Array.from(st.vars.keys())) if (!all.has(k)) st.vars.delete(k);
    for (const k of Array.from(st.fields.keys())) if (!all.has(k)) st.fields.delete(k);
    if (st.focus !== null && !all.has(keyOf(st.focus))) {
      st.focus = null;
      st.selecting = false;
    }
    if (st.hover !== null && !all.has(keyOf(st.hover))) st.hover = null;
    if (st.pressed !== null && !all.has(keyOf(st.pressed))) st.pressed = null;
  };
  Run.prototype.queue = function (path) {
    if (!this.pending.some((p) => samePath(p, path))) this.pending.push(path);
  };
  Run.prototype.indexOf = function (path) {
    const key = keyOf(path);
    return this.nodes.findIndex((n) => n.key === key && n.path.length === path.length);
  };
  // Every playing timeline one frame on.
  Run.prototype.advance = function () {
    const st = this.st;
    if (st.root.playing) {
      const n = this.sceneTimeline(st.scene).n;
      let scene = st.scene, frame;
      if (st.root.frame + 1 < n) frame = st.root.frame + 1;
      else if (st.scene + 1 < this.M.scenes.length) {
        scene = st.scene + 1;
        frame = 0;
      } else {
        scene = 0;
        frame = 0;
      }
      if (scene !== st.scene || frame !== st.root.frame) {
        st.scene = scene;
        st.root.frame = frame;
        this.queue([]);
      }
    }
    const lens = new Dict();
    for (const nd of this.nodes) if (nd.kind === CLIP && !lens.has(nd.key)) lens.set(nd.key, nd.frames);
    const moved = [];
    const clips = Array.from(st.clips.entries()).sort((a, b) => comparePaths(a[1].path, b[1].path));
    for (const [k, c] of clips) {
      if (!c.playing) continue;
      const len = Math.max(lens.has(k) ? lens.get(k) : 1, 1);
      const next = (c.frame + 1) % len;
      if (next !== c.frame) {
        c.frame = next;
        moved.push(c.path);
      }
    }
    for (const p of moved) this.queue(p);
    this.dirty = true;
  };
  Run.prototype.settle = function () {
    for (;;) {
      this.refresh();
      if (this.halted) {
        this.pending = [];
        return;
      }
      const path = this.pending.shift();
      if (path === undefined) return;
      this.enters += 1;
      if (this.enters > MAX_ENTERS_PER_STEP) {
        this.halt("more than " + MAX_ENTERS_PER_STEP + " frames entered");
        continue;
      }
      this.enter(path);
    }
  };
  Run.prototype.timelineOf = function (i) {
    const n = this.nodes[i];
    if (!n) return null;
    if (n.kind === ROOT) return this.sceneTimeline(this.st.scene);
    if (n.kind === CLIP) {
      const s = this.M.symbols[n.sym];
      return s ? this.M.timelines[s.tl] : null;
    }
    return null;
  };
  // Runs what the keyframes starting at the timeline's frame hold, top layer first.
  Run.prototype.enter = function (path) {
    const i = this.indexOf(path);
    if (i < 0) return;
    const tl = this.timelineOf(i);
    if (!tl) return;
    const frame = this.nodes[i].frame;
    this.soundsAt(tl, frame, this.nodes[i].gain);
    for (const script of (tl.a && tl.a[frame]) || []) {
      if (Array.isArray(script)) this.runList(path, script, false);
      else if (script.as) this.runScript(path, frame, script.as);
      else this.out("Frame script " + script.plugin + " not run: frame scripts are not exported");
    }
  };
  // Counts one action against the step's budget; false when nothing more may run.
  Run.prototype.budget = function () {
    if (this.halted) return false;
    this.actions += 1;
    if (this.actions > MAX_ACTIONS_PER_STEP) {
      this.halt("more than " + MAX_ACTIONS_PER_STEP + " actions");
      return false;
    }
    return true;
  };
  // `script`: the actions come from ActionScript.
  Run.prototype.runList = function (owner, list, script) {
    for (const a of list) {
      if (!this.budget()) return;
      this.exec(owner, a, script);
    }
  };
  // The timeline node `i` is, for its scripts' functions and variable types (the main
  // timeline's span every scene, so any scene's carries them).
  Run.prototype.declsOf = function (i) {
    const n = this.nodes[i];
    if (n && n.kind === CLIP) {
      const s = this.M.symbols[n.sym];
      if (s) return this.M.timelines[s.tl];
    }
    return this.sceneTimeline(this.st.scene);
  };
  // A frame script in the subset, on timeline `owner` (the export keeps only those that link).
  Run.prototype.runScript = function (owner, frame, code) {
    const i = this.indexOf(owner);
    if (i < 0) return;
    const where = "Script on frame " + (frame + 1) + " of " + this.timelineName(i);
    this.runCode(owner, this.declsOf(i), null, where, (w, fuel) => runScriptCode(w, code, fuel));
  };
  Run.prototype.runCode = function (owner, tl, ev, where, go) {
    if (this.halted) return;
    if (this.fuel === 0) {
      this.halt("scripts ran more than " + MAX_FUEL_PER_STEP + " steps");
      return;
    }
    const fuel = Math.min(this.fuel, FUEL_PER_RUN);
    const [used, stop] = go(new Script(this, owner, tl, ev), fuel);
    this.fuel = Math.max(this.fuel - used, 0);
    if (stop !== null && stop.kind !== "halt") this.out(where + " stopped: " + stop.describe());
  };
  Run.prototype.timelineName = function (i) {
    if (i === 0) return "the main timeline";
    const names = [];
    let cur = i;
    for (let k = 0; k <= MAX_SYMBOL_DEPTH; k++) {
      const n = this.nodes[cur];
      if (!n || n.kind === ROOT) break;
      names.push(n.name === "" ? "?" : n.name);
      cur = n.owner;
    }
    return names.reverse().join(".");
  };
  Run.prototype.callListener = function (l, ev) {
    this.refresh();
    const i = this.indexOf(l.owner);
    if (i < 0) return;
    this.runCode(l.owner, this.declsOf(i), ev, "Listener " + l.k, (w, fuel) => runListenerCode(w, l.f, fuel));
  };
  // Every enter-frame listener, in the order they were added.
  Run.prototype.enterFrame = function () {
    this.refresh();
    const all = [];
    for (const [, list] of this.st.listeners.entries()) for (const l of list) if (l.ev === "enter_frame") all.push([l.path, l]);
    for (const l of this.st.stage) if (l.ev === "enter_frame") all.push([null, l]);
    all.sort((a, b) => a[1].seq - b[1].seq);
    for (const [at, l] of all) {
      if (this.halted) return;
      this.callListener(l, { key: null, target: at, current: at });
      this.settle();
    }
  };
  Run.prototype.key = function (ev, code) {
    const list = this.st.stage.filter((l) => l.ev === ev);
    for (const l of list) {
      if (this.halted) return;
      this.callListener(l, { key: code, target: null, current: null });
      this.settle();
    }
  };
  // The node a script names from timeline node `from`: {ok: index or null (the stage)} or {err}.
  Run.prototype.place = function (from, ev, at) {
    let base = from;
    const path = at.path || "";
    const empty = path.trim() === "" || path.trim() === "this";
    switch (at.base) {
      case "stage":
        return empty ? { ok: null } : { err: "instances on the stage are named from the main timeline (root)" };
      case "target":
      case "current": {
        if (!ev) return { err: "there's no event here" };
        const p = at.base === "target" ? ev.target : ev.current;
        if (p === null) return empty ? { ok: null } : { err: "the event's target is the stage" };
        base = this.indexOf(p);
        if (base < 0) return { err: "the event's target has left the stage" };
      }
    }
    const [i, err] = this.resolve(base, { t: segsOf(path) });
    return i < 0 ? { err: err } : { ok: i };
  };
  Run.prototype.basePath = function (owner, ev, base) {
    if (base === "this") return { ok: owner };
    if (base === "stage") return { err: "the stage has no playhead" };
    if (!ev) return { err: "there's no event here" };
    const p = base === "target" ? ev.target : ev.current;
    return p === null ? { err: "the event's target is the stage" } : { ok: p };
  };
  // A property of node `i` (null: the stage), as player.rs `property` reads it.
  Run.prototype.property = function (i, p) {
    const st = this.st;
    if (i === null) {
      if (p === "mouse_x") return { ok: short(st.mouse.x) };
      if (p === "mouse_y") return { ok: short(st.mouse.y) };
      return { err: "the stage's `" + (READ_NAMES[p] || PROP_NAMES[p]) + "` isn't supported here" };
    }
    const n = this.nodes[i];
    if (!n) return { err: "the instance is gone" };
    if (n.kind === ROOT) {
      switch (p) {
        case "x":
        case "y":
        case "rotation":
          return { ok: 0 };
        case "alpha":
        case "scale_x":
        case "scale_y":
          return { ok: 1 };
        case "visible":
          return { ok: true };
        case "current_frame":
          return { ok: st.root.frame + 1 };
        case "total_frames":
          return { ok: this.M.scenes.reduce((s, sc) => s + this.M.timelines[sc.tl].n, 0) };
        case "current_label":
          return { ok: currentLabel(this.sceneTimeline(st.scene), st.root.frame) };
        case "mouse_x":
          return { ok: short(st.mouse.x) };
        case "mouse_y":
          return { ok: short(st.mouse.y) };
        case "name":
          return { ok: "root1" };
        case "text":
        case "length":
          return { err: "the main timeline isn't a text field" };
      }
    }
    if (p === "text" || p === "length") {
      if (n.kind !== TEXT) return { err: "“" + n.name + "” is " + kindName(n.kind) + ", not a text field" };
      const v = fieldValue(st.fields.get(n.key), n.tf);
      return { ok: p === "text" ? v : v.length };
    }
    const ov = st.props.get(n.key) || {};
    const m = n.local;
    const a = m[0], b = m[1], c = m[2], d = m[3];
    const sx = Math.sqrt(a * a + b * b);
    const r = Math.atan2(b, a);
    const det = a * d - b * c;
    const sy = sx > 1e-12 ? det / sx : Math.sqrt(c * c + d * d);
    const pick = (o, v) => ({ ok: short(o !== undefined ? o : v) });
    switch (p) {
      case "x":
        return pick(ov.x, m[4]);
      case "y":
        return pick(ov.y, m[5]);
      case "scale_x":
        return pick(ov.scale_x, sx);
      case "scale_y":
        return pick(ov.scale_y, sy);
      case "rotation":
        return pick(ov.rotation, r * (180 / Math.PI));
      case "alpha":
        return { ok: short(ov.alpha !== undefined ? ov.alpha : n.alpha) };
      case "visible":
        return { ok: ov.visible !== false };
      case "name":
        return { ok: n.name };
      case "mouse_x":
      case "mouse_y": {
        const x = n.xf;
        const dt = x[0] * x[3] - x[1] * x[2];
        if (dt === 0 || !Number.isFinite(dt)) return { ok: 0 };
        const px = st.mouse.x - x[4], py = st.mouse.y - x[5];
        return { ok: short(p === "mouse_x" ? (x[3] * px - x[2] * py) / dt : (x[0] * py - x[1] * px) / dt) };
      }
    }
    if (n.kind !== CLIP) return { err: "“" + n.name + "” is " + kindName(n.kind) + "; only movie clips and the main timeline have frames" };
    if (p === "current_frame") return { ok: n.frame + 1 };
    if (p === "total_frames") return { ok: n.frames };
    const s = this.M.symbols[n.sym];
    return { ok: s ? currentLabel(this.M.timelines[s.tl], n.frame) : null };
  };
  // addEventListener / removeEventListener (player.rs `listen`).
  Run.prototype.listenTo = function (owner, ev, at, event, f, add) {
    this.refresh();
    const from = this.indexOf(owner);
    if (from < 0) return;
    const what = (add ? "Listen" : "Stop listening") + " for " + event + " (" + placeText(at) + ")";
    const pl = this.place(from, ev, at);
    if (pl.err !== undefined) return this.out(what + ": " + pl.err);
    const st = this.st;
    const same = (x) => x.ev === event && samePath(x.owner, owner) && x.k === f.name;
    if (pl.ok === null) {
      if (add) {
        if (st.stage.some(same)) return;
        if (st.stage.length >= MAX_STAGE_LISTENERS) return this.out(what + ": the stage already has " + MAX_STAGE_LISTENERS + " listeners");
        st.stage.push({ ev: event, owner: owner, k: f.name, f: f, seq: st.seq, path: null });
        st.seq += 1;
      } else st.stage = st.stage.filter((x) => !same(x));
      return;
    }
    const node = this.nodes[pl.ok];
    if (!node) return;
    const ok = event === "enter_frame" ? node.kind !== GRAPHIC : node.kind === BUTTON || node.kind === CLIP;
    if (!ok) {
      const why = node.kind === ROOT ? "the main timeline doesn't get pointer events here" : "“" + node.name + "” is " + kindName(node.kind) + "; only buttons and movie clips get pointer events";
      return this.out(what + ": " + why);
    }
    let list = st.listeners.get(node.key);
    if (!add) {
      if (list) {
        list = list.filter((x) => !same(x));
        if (list.length === 0) st.listeners.delete(node.key);
        else st.listeners.set(node.key, list);
      }
      return;
    }
    if (list && list.some(same)) return;
    if (list && list.length >= MAX_LISTENERS_PER_INSTANCE) return this.out(what + ": “" + node.name + "” already has " + MAX_LISTENERS_PER_INSTANCE + " listeners");
    if (!list) {
      list = [];
      st.listeners.set(node.key, list);
    }
    list.push({ ev: event, owner: owner, k: f.name, f: f, seq: st.seq, path: node.path });
    st.seq += 1;
  };
  // The node `a`'s target names, seen from timeline node `from`: [index] or [-1, why].
  Run.prototype.resolve = function (from, a) {
    if (a.e !== undefined) return [-1, a.e];
    let cur = from;
    for (const seg of a.t || []) {
      if (seg === "^") cur = 0;
      else if (seg === "..") {
        const n = this.nodes[cur];
        if (!n || n.kind === ROOT) return [-1, "the main timeline has no parent"];
        cur = n.owner;
      } else {
        const c = cur;
        const j = this.nodes.findIndex((n, k) => k !== c && n.owner === c && n.name === seg);
        if (j < 0) return [-1, "no instance named “" + seg + "” here"];
        cur = j;
      }
    }
    return [cur];
  };
  Run.prototype.exec = function (owner, a, script) {
    this.refresh();
    const from = this.indexOf(owner);
    if (from < 0) return;
    const st = this.st;
    switch (a.op) {
      case "trace":
        this.out(a.msg || "");
        return;
      case "next_scene":
      case "prev_scene": {
        // In ActionScript a movie clip's timeline has one scene.
        if (script && owner.length > 0) return this.out(a.d + ": a movie clip's timeline has one scene; call it in the main timeline's script");
        const s = a.op === "next_scene" ? st.scene + 1 : st.scene - 1;
        if (s >= 0 && s < this.M.scenes.length) this.gotoRoot(s, 0);
        return;
      }
      case "set_property": {
        const [i, err] = this.resolve(from, a);
        if (i < 0) return this.out(a.d + ": " + err);
        if (i === 0) return this.out(a.d + ": the main timeline has no " + a.pl + " to set");
        const n = this.nodes[i];
        let ov = st.props.get(n.key);
        if (!ov) {
          ov = { path: n.path };
          st.props.set(n.key, ov);
        }
        setProp(ov, a.p, a.v);
        this.dirty = true;
        return;
      }
    }
    const [i, err] = this.resolve(from, a);
    if (i < 0) return this.out(a.d + ": " + err);
    const node = this.nodes[i];
    if (!node) return;
    if (node.kind !== ROOT && node.kind !== CLIP) return this.out(a.d + ": “" + node.name + "” is " + kindName(node.kind) + "; only movie clips and the main timeline have a playhead");
    const path = node.path, frame = node.frame, frames = Math.max(node.frames, 1);
    switch (a.op) {
      case "stop":
      case "play":
        this.setPlaying(path, a.op === "play");
        return;
      case "next_frame":
      case "prev_frame": {
        const f = a.op === "next_frame" ? Math.min(frame + 1, frames - 1) : Math.max(frame - 1, 0);
        this.setPlaying(path, false);
        this.gotoFrame(path, null, f);
        return;
      }
      case "goto_and_play":
      case "goto_and_stop": {
        const [s, f, why] = this.gotoTarget(path, a);
        if (why !== undefined) return this.out(a.d + ": " + why);
        this.setPlaying(path, a.op === "goto_and_play");
        this.gotoFrame(path, s, f);
        return;
      }
    }
  };
  // Where a go-to lands: [scene or null, frame] or [_, _, why].
  Run.prototype.gotoTarget = function (path, a) {
    const M = this.M;
    let scene = null, tl;
    if (path.length === 0) {
      if (a.s !== undefined) {
        let s = M.scenes.findIndex((x) => x.name === a.s);
        if (s < 0) s = M.scenes.findIndex((x) => asciiLower(x.name) === asciiLower(a.s));
        if (s < 0) return [null, 0, "no scene named “" + a.s + "”"];
        scene = s;
      } else scene = this.st.scene;
      tl = this.sceneTimeline(scene);
    } else {
      if (a.s !== undefined) return [null, 0, "only the main timeline has scenes"];
      const i = this.indexOf(path);
      if (i < 0) return [null, 0, "the timeline is gone"];
      tl = this.timelineOf(i);
      if (!tl) return [null, 0, "not a timeline"];
    }
    if (typeof a.f === "number") return [scene, Math.min(a.f, Math.max(tl.n - 1, 0))];
    const label = String(a.f);
    if (tl.labels.has(label)) return [scene, tl.labels.get(label)];
    // The main timeline: a label in another scene, when no scene was named.
    if (path.length === 0 && a.s === undefined) {
      for (let s = 0; s < M.scenes.length; s++) {
        const other = M.timelines[M.scenes[s].tl];
        if (other.labels.has(label)) return [s, other.labels.get(label)];
      }
    }
    return [null, 0, "no frame labelled “" + label + "”"];
  };
  Run.prototype.setPlaying = function (path, on) {
    if (path.length === 0) this.st.root.playing = on;
    else {
      const c = this.st.clips.get(keyOf(path));
      if (c) c.playing = on;
    }
  };
  Run.prototype.gotoRoot = function (scene, frame) {
    const st = this.st;
    if (scene !== st.scene || frame !== st.root.frame) {
      st.scene = scene;
      st.root.frame = frame;
      this.queue([]);
      this.dirty = true;
    }
  };
  Run.prototype.gotoFrame = function (path, scene, frame) {
    if (path.length === 0) return this.gotoRoot(scene === null ? this.st.scene : scene, frame);
    const c = this.st.clips.get(keyOf(path));
    if (c && c.frame !== frame) {
      c.frame = frame;
      this.queue(path);
      this.dirty = true;
    }
  };
  Run.prototype.event = function (e) {
    this.refresh();
    const st = this.st;
    switch (e.type) {
      case "move":
        this.pointer(e);
        // Dragging in the focused field selects.
        if (st.selecting && st.down) this.pointField(true);
        break;
      case "down": {
        this.pointer(e);
        // A field the viewer can use takes the press (unless a button is above it): it gets
        // the keyboard. Anywhere else the focused field loses it.
        const h = st.hover !== null ? this.indexOf(st.hover) : -1;
        const f = this.fieldAt(e.x, e.y);
        if (f !== null && (h < 0 || f[0] > h)) {
          st.focus = f[1];
          st.down = true;
          st.selecting = true;
          this.pointField(false);
          break;
        }
        st.focus = null;
        st.selecting = false;
        const before = this.pointerNow();
        st.down = true;
        st.pressed = st.hover;
        this.dirty = true;
        this.buttonSounds(before);
        if (st.hover !== null) this.fire(st.hover, "press");
        break;
      }
      case "up": {
        if (st.selecting) {
          st.selecting = false;
          st.down = false;
          this.pointer(e);
          break;
        }
        this.pointer(e);
        const before = this.pointerNow();
        st.down = false;
        const pressed = st.pressed;
        st.pressed = null;
        this.dirty = true;
        this.buttonSounds(before);
        if (st.hover !== null) {
          const h = st.hover;
          this.fire(h, "release");
          if (samePath(pressed, h)) this.fire(h, "click");
        }
        break;
      }
      case "leave":
        this.pointer(null);
        break;
      // Keys edit the focused field, then go to the stage's listeners as well. A key the page
      // sends from one of its own field elements (`dom`) has edited that element already.
      case "keydown":
        if (e.code === 16) st.shift = true;
        if (!e.dom) this.fieldKey(e.code);
        this.key("key_down", e.code);
        break;
      case "keyup":
        if (e.code === 16) st.shift = false;
        this.key("key_up", e.code);
        break;
      case "char":
        this.editField((ed) => ed.insert(String(e.text || "")));
        break;
      case "selectall":
        this.editField((ed) => ed.key("select_all", false));
        break;
    }
  };

  // ----- text fields (player.rs, field.rs; ADR 0066) -------------------------------------------
  //
  // In the browser the viewer types into a real <input> or <textarea> that canvas.js lays over
  // each input field: the browser does the editing, selection, IME and clipboard, and tells the
  // player the new text (`Player.typed`) and where the keyboard is (`Player.focus`). What
  // follows is a deliberately small mirror of field.rs's editor, for the `char`, `selectall` and
  // editing-key events the tests send: it edits as the Rust player does before it has laid the
  // text out (no line or glyph positions: Home and Up go to the start, End and Down to the end,
  // and a click puts the caret at the end), which is all the tests compare.

  // A field's text: what a script or the viewer set, or the authored text.
  function fieldValue(fs, tf) {
    return fs && fs.text !== null ? fs.text : tf.t;
  }
  function multiline(tf) {
    return tf.ln === "multiline" || tf.ln === "multiline_no_wrap";
  }
  // An input field, or a selectable dynamic one (TextField::focusable).
  function focusable(tf) {
    return tf.k === "input" || (tf.k === "dynamic" && tf.sel !== false);
  }
  // Text a script sets, as a field keeps it (field.rs `fit`): ActionScript's line ends as line
  // breaks, at most MAX_FIELD_CHARS characters.
  function fit(text) {
    return Array.from(text.replace(/\r\n/g, "\n").replace(/\r/g, "\n")).slice(0, MAX_FIELD_CHARS).join("");
  }
  // Text the viewer typed into the page's element, as the editor would let it in: no control
  // characters, no line breaks in a one-line field, at most the field's character limit.
  function typedText(tf, text) {
    const one = !multiline(tf);
    const cs = Array.from(text.replace(/\r\n?/g, "\n")).filter((c) => !rustControl(c) || (c === "\n" && !one));
    const max = tf.mx > 0 ? Math.min(tf.mx, MAX_FIELD_CHARS) : MAX_FIELD_CHARS;
    return cs.slice(0, max).join("");
  }
  function newField(path) {
    return { path: path, text: null, caret: 0, anchor: 0 };
  }
  // The caret and selection back inside the text (player.rs `relayout`, without a layout).
  function clampField(fs, tf) {
    const n = Array.from(fieldValue(fs, tf)).length;
    fs.caret = Math.min(fs.caret, n);
    fs.anchor = Math.min(fs.anchor, n);
  }
  const EDIT_KEYS = { 37: "left", 39: "right", 38: "up", 40: "down", 36: "home", 35: "end", 8: "backspace", 46: "delete", 13: "enter" };

  // field.rs `Editor`, without a layout. Indices count code points (Rust `char`s).
  function Editor(tf, fs) {
    this.tf = tf;
    this.fs = fs;
  }
  Editor.prototype.len = function () {
    return Array.from(fieldValue(this.fs, this.tf)).length;
  };
  Editor.prototype.editable = function () {
    return this.tf.k === "input";
  };
  Editor.prototype.selection = function () {
    return [Math.min(this.fs.caret, this.fs.anchor), Math.max(this.fs.caret, this.fs.anchor)];
  };
  // Replaces the selection with `s`; true when the text changed.
  Editor.prototype.insert = function (s) {
    if (!this.editable()) return false;
    const one = !multiline(this.tf);
    let add = Array.from(s).filter((c) => !rustControl(c) || (c === "\n" && !one));
    const [a, b] = this.selection();
    const value = Array.from(fieldValue(this.fs, this.tf));
    const kept = value.length - (b - a);
    let room = this.tf.mx > 0 ? Math.max(this.tf.mx - kept, 0) : Infinity;
    room = Math.min(room, Math.max(MAX_FIELD_CHARS - kept, 0));
    if (add.length > room) add = add.slice(0, room);
    if (add.length === 0 && a === b) return false;
    this.fs.text = value.slice(0, a).concat(add, value.slice(b)).join("");
    this.fs.caret = a + add.length;
    this.fs.anchor = this.fs.caret;
    return true;
  };
  // One editing key; `shift` extends the selection with caret moves. True when the text changed.
  Editor.prototype.key = function (e, shift) {
    const len = this.len();
    const fs = this.fs;
    const [a, b] = this.selection();
    const moved = (to, sh) => {
      fs.caret = Math.min(to, len);
      if (!sh) fs.anchor = fs.caret;
      return false;
    };
    switch (e) {
      case "select_all":
        fs.anchor = 0;
        fs.caret = len;
        return false;
      case "left":
        return !shift && a !== b ? moved(a, false) : moved(Math.max(fs.caret - 1, 0), shift);
      case "right":
        return !shift && a !== b ? moved(b, false) : moved(fs.caret + 1, shift);
      case "home":
      case "up":
        return moved(0, shift);
      case "end":
      case "down":
        return moved(len, shift);
      case "backspace":
      case "delete":
        if (!this.editable()) return false;
        if (a !== b) return this.insert("");
        if (e === "backspace") {
          if (a === 0) return false;
          fs.anchor = a - 1;
          return this.insert("");
        }
        if (b >= len) return false;
        fs.anchor = a;
        fs.caret = b + 1;
        return this.insert("");
      case "enter":
        return multiline(this.tf) ? this.insert("\n") : false;
    }
    return false;
  };

  // The topmost field at stage point (x, y) the viewer can use: [node index, path] or null.
  Run.prototype.fieldAt = function (x, y) {
    for (let i = this.nodes.length - 1; i >= 0; i--) {
      const n = this.nodes[i];
      if (n.kind !== TEXT || !n.hittable || !focusable(n.tf)) continue;
      const q = inverseApply(n.xf, x, y);
      if (q !== null && q[0] >= 0 && q[1] >= 0 && q[0] <= n.tf.w && q[1] <= n.tf.h) return [i, n.path];
    }
    return null;
  };
  // The focused field's node, or null.
  Run.prototype.focusedField = function () {
    if (this.st.focus === null) return null;
    const n = this.nodes[this.indexOf(this.st.focus)];
    return n && n.kind === TEXT ? n : null;
  };
  // Runs an edit on the focused field (its state made the first time it is needed).
  Run.prototype.editField = function (go) {
    const n = this.focusedField();
    if (n === null) return;
    let fs = this.st.fields.get(n.key);
    if (!fs) {
      fs = newField(n.path);
      this.st.fields.set(n.key, fs);
    }
    clampField(fs, n.tf);
    if (go(new Editor(n.tf, fs))) clampField(fs, n.tf);
  };
  // The pointer in the focused field: without a layout the caret goes to the end (`extend`:
  // keeping the selection's other end).
  Run.prototype.pointField = function (extend) {
    this.editField((ed) => {
      ed.fs.caret = ed.len();
      if (!extend) ed.fs.anchor = ed.fs.caret;
      return false;
    });
  };
  // A key down for the focused field: an edit, or Tab to the next input field.
  Run.prototype.fieldKey = function (code) {
    const st = this.st;
    if (st.focus === null) return;
    if (code === 9) {
      const inputs = this.nodes.filter((n) => n.hittable && n.kind === TEXT && n.tf.k === "input").map((n) => n.path);
      const at = inputs.findIndex((p) => samePath(p, st.focus));
      const n = inputs.length;
      if (n > 0) {
        const next = at < 0 ? 0 : st.shift ? (at + n - 1) % n : (at + 1) % n;
        st.focus = inputs[next];
        this.editField((ed) => ed.key("select_all", false));
      }
      return;
    }
    const e = EDIT_KEYS[code];
    if (e === undefined) return;
    const shift = st.shift;
    this.editField((ed) => ed.key(e, shift));
  };
  // A script sets field `path`'s text.
  Run.prototype.setFieldText = function (path, text) {
    const n = this.nodes[this.indexOf(path)];
    if (!n || n.kind !== TEXT) return;
    let fs = this.st.fields.get(n.key);
    if (!fs) {
      fs = newField(n.path);
      this.st.fields.set(n.key, fs);
    }
    fs.text = fit(text);
    clampField(fs, n.tf);
  };
  // Moves the pointer: what it leaves gets Out, what it reaches gets Over.
  Run.prototype.pointer = function (p) {
    this.refresh();
    const st = this.st;
    st.pointer = p ? { x: p.x, y: p.y } : null;
    if (p) st.mouse = { x: p.x, y: p.y };
    const now = p ? hitIn(this.M, st, this.nodes, this.host, p.x, p.y) : null;
    const same = now === null ? st.hover === null : samePath(now, st.hover);
    if (!same) {
      const before = this.pointerNow();
      const old = st.hover;
      st.hover = now;
      this.dirty = true;
      this.buttonSounds(before);
      if (old !== null) this.fire(old, "out");
      if (now !== null) this.fire(now, "over");
    }
  };
  // Runs an instance's handler for `event`, in the timeline the instance is on, then the
  // listeners scripts added for it (each in its script's timeline), in the order added.
  Run.prototype.fire = function (path, event) {
    this.refresh();
    const i = this.indexOf(path);
    if (i < 0) return;
    const n = this.nodes[i];
    const h = (n.events || []).find((x) => x[0] === event);
    const ls = (this.st.listeners.get(n.key) || []).filter((l) => l.ev === event);
    if (!h && ls.length === 0) return;
    const owner = this.nodes[n.owner];
    if (h && owner) this.runList(owner.path, h[1], false);
    for (const l of ls) this.callListener(l, { key: null, target: path, current: path });
    this.settle();
  };
  Run.prototype.finish = function () {
    this.refresh();
    // Graphics keep step with their timelines: one now on another frame (or new) entered it.
    const entered = this.nodes.filter((n) => n.kind === GRAPHIC && !(this.graphics.has(n.key) && this.graphics.get(n.key) === n.frame));
    for (const n of entered) {
      const s = this.M.symbols[n.sym];
      if (s) this.soundsAt(this.M.timelines[s.tl], n.frame, n.gain);
    }
    if (this.dropped > 0) this.output.push("… " + this.dropped + " more lines");
    return { output: this.output, sounds: this.sounds };
  };

  function asciiLower(s) {
    return s.replace(/[A-Z]/g, (c) => c.toLowerCase());
  }

  return { Player: Player, evaluate: evaluate, applyEffect: applyEffect, recolours: recolours, mul: mul, meanScale: meanScale, finite: finite };
})();
