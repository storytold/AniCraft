// AniCraft HTML canvas player: the core (ADR 0028). SPDX-License-Identifier: MIT OR Apache-2.0
//
// Plays a movie exported by File > Export > HTML Canvas: the instance tree, movie clip
// playheads, frame actions, button events, and frame scripts in the ActionScript subset
// (compiled by AniCraft to the same actions, with the mouse listeners they add). It is a line-by-line port of the Rust player
// (crates/doc/src/player.rs, the reference) over the exported data, and the tests run it in a
// JavaScript engine against that player. Nothing here touches the page: canvas.js draws and
// listens to the pointer; this file only computes.
//
// The movie data (see crates/render/src/html.rs): every timeline's frames are baked (tweens,
// easing, motion paths and shape morphs already evaluated), so this file never interpolates.
// A frame is a list of layers, bottom first; a layer holds items: shapes (a path, a paint, a
// stroke) and symbol instances (a symbol, its element id, its name and event handlers).
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
  const ROOT = 0, GRAPHIC = 1, CLIP = 2, BUTTON = 3;
  const KINDS = { graphic: GRAPHIC, movieclip: CLIP, button: BUTTON };
  const I = [1, 0, 0, 1, 0, 0];
  const NO_EFFECT = [1, 1, 1, 1, 0];
  const EMPTY = { l: [] };

  // ----- geometry ------------------------------------------------------------------------------

  // `p · q`: apply q first, then p (Affine::mul).
  function mul(p, q) {
    return [p[0] * q[0] + p[2] * q[1], p[1] * q[0] + p[3] * q[1], p[0] * q[2] + p[2] * q[3], p[1] * q[2] + p[3] * q[3], p[0] * q[4] + p[2] * q[5] + p[4], p[1] * q[4] + p[3] * q[5] + p[5]];
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

  // ----- colour effects: [alpha, tint r, g, b, tint amount] ---------------------------------------

  // ColorEffect::concat: tints keep the stronger one; alpha multiplies.
  function concat(o, i) {
    const t = i[4] >= o[4] ? i : o;
    return [o[0] * i[0], t[1], t[2], t[3], t[4]];
  }
  // ColorEffect::apply on straight [r, g, b, a].
  function applyEffect(fx, c) {
    const k = Math.min(Math.max(fx[4], 0), 1);
    const a = Math.min(Math.max(fx[0], 0), 1);
    return [c[0] + (fx[1] - c[0]) * k, c[1] + (fx[2] - c[1]) * k, c[2] + (fx[3] - c[2]) * k, c[3] * a];
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

  function newState() {
    return { scene: 0, root: { frame: 0, playing: true }, clips: new Dict(), props: new Dict(), listeners: new Dict(), pointer: null, down: false, hover: null, pressed: null, tick: 0 };
  }
  // Each clip entry: { path, frame, playing }; each props entry: { path, ...overrides }; each
  // listeners entry: [{ ev, owner, k, h, path }] in the order added.
  function clipFrame(st, key, len) {
    const c = st.clips.get(key);
    return Math.min(c ? c.frame : 0, Math.max(len, 1) - 1);
  }
  // Up (0), Over (1) under the pointer, Down (2) while pressed on it.
  function buttonFrame(st, path, len) {
    const over = samePath(st.hover, path);
    const f = over ? (st.down && samePath(st.pressed, path) ? 2 : 1) : 0;
    return Math.min(f, Math.max(len, 1) - 1);
  }

  // Property ranges (anicraft_actions::Property::range).
  const RANGES = { x: [-1e6, 1e6], y: [-1e6, 1e6], alpha: [0, 1], visible: [0, 1], rotation: [-36000, 36000], scale_x: [-1000, 1000], scale_y: [-1000, 1000] };
  function setProp(ov, p, value) {
    const r = RANGES[p];
    if (!r || !Number.isFinite(value)) return;
    const v = Math.min(Math.max(value, r[0]), r[1]);
    if (p === "visible") ov.visible = v !== 0;
    else ov[p] = v;
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
      fx[0] = ov.alpha;
    }
    return { m: m, c: fx || NO_EFFECT };
  }

  // ----- the instance tree (player.rs `tree`, `walk`) --------------------------------------------

  function tree(M, st) {
    const tl = M.scenes[st.scene] ? M.scenes[st.scene].tl : 0;
    const out = [{ path: [], key: "", parent: -1, owner: 0, name: "", sym: -1, kind: ROOT, frame: st.root.frame, frames: M.timelines[tl].n, xf: I, hittable: true, events: null, gain: 1 }];
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
        out.push({ path: path, key: key, parent: parent, owner: owner, name: e.n || "", sym: e.y, kind: kind, frame: local, frames: len, xf: nxf, hittable: hit && shown, events: e.ev || null, gain: gain });
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
    if (e.y !== undefined && play) {
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
    const fade = Math.min(Math.max(fx[0], 0), 1);
    const blend = e.bl || "normal";
    let color = concat(cx.c, fx), group = cx.group;
    if (masks === null && (fade < 1 - 1e-4 || blend !== "normal" || e.fx)) {
      group = dl.groups.length;
      dl.groups.push({ parent: cx.group, alpha: fade, blend: blend, clip: clip });
      color = concat(cx.c, [1, fx[1], fx[2], fx[3], fx[4]]);
    }
    if (v && group !== cx.group) v.groupBegin(group, dl.groups[group]);
    const inner = { xf: xf, c: color, depth: cx.depth + 1, clip: clip, masks: masks, group: group };
    evalTimeline(M, sym.tl, local, inner, dl, child, v);
    if (v && group !== cx.group) v.groupEnd(group, dl.groups[group]);
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

  // ----- the player (player.rs `Run`) ------------------------------------------------------------

  // A player for `movie` (the exported data, as parsed JSON). `host` answers hit tests (see
  // hitIn). `start()` shows the first frame, `step(events)` moves one frame on, `input(events)`
  // routes pointer events between frames; each returns { output, sounds }: the lines the movie
  // wrote (traces, and why an action could not run) and the sounds of the keyframes entered.
  // Events: { type: "move" | "down" | "up", x, y } (stage pixels) or { type: "leave" }.
  function Player(movie, host) {
    this.M = load(movie);
    this.host = host;
    this.st = newState();
  }
  Player.prototype.start = function () {
    this.st = newState();
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
    return { scene: st.scene, frame: st.root.frame, playing: st.root.playing, clips: clips, props: props, listeners: listeners, hover: st.hover, pressed: st.pressed, down: st.down };
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
  Run.prototype.soundsAt = function (tl, frame, gain) {
    for (const s of (tl.snd && tl.snd[frame]) || []) {
      if (this.sounds.length >= MAX_SOUNDS_PER_STEP) return;
      this.sounds.push([s[0], s[1], s[2], s[3], s[4], (s[5] === undefined ? 1 : s[5]) * gain, s[6] || null]);
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
      else if (script.as) this.runOps(path, script.as);
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
  // A compiled script's steps: actions, and `listen` (addEventListener).
  Run.prototype.runOps = function (owner, ops) {
    for (const a of ops) {
      if (!this.budget()) return;
      if (a.op === "listen") this.listen(owner, a);
      else this.exec(owner, a, true);
    }
  };
  // The instance the listener's target names gets it (once per function and event).
  Run.prototype.listen = function (owner, a) {
    this.refresh();
    const from = this.indexOf(owner);
    if (from < 0) return;
    const [i, err] = this.resolve(from, a);
    if (i < 0) return this.out(a.d + ": " + err);
    const node = this.nodes[i];
    if (!node) return;
    if (node.kind !== BUTTON && node.kind !== CLIP) {
      const why = node.kind === ROOT ? "the main timeline doesn't get pointer events here" : "“" + node.name + "” is a graphic; only buttons and movie clips get pointer events";
      return this.out(a.d + ": " + why);
    }
    let list = this.st.listeners.get(node.key);
    if (list && list.some((x) => x.ev === a.ev && samePath(x.owner, owner) && x.k === a.k)) return;
    if (list && list.length >= MAX_LISTENERS_PER_INSTANCE) return this.out(a.d + ": “" + node.name + "” already has " + MAX_LISTENERS_PER_INSTANCE + " listeners");
    if (!list) {
      list = [];
      this.st.listeners.set(node.key, list);
    }
    list.push({ ev: a.ev, owner: owner, k: a.k, h: a.h, path: node.path });
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
    if (node.kind !== ROOT && node.kind !== CLIP) {
      const what = node.kind === BUTTON ? "a button" : "a graphic";
      return this.out(a.d + ": “" + node.name + "” is " + what + "; only movie clips and the main timeline have a playhead");
    }
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
        break;
      case "down":
        this.pointer(e);
        st.down = true;
        st.pressed = st.hover;
        this.dirty = true;
        if (st.hover !== null) this.fire(st.hover, "press");
        break;
      case "up": {
        this.pointer(e);
        st.down = false;
        const pressed = st.pressed;
        st.pressed = null;
        this.dirty = true;
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
    }
  };
  // Moves the pointer: what it leaves gets Out, what it reaches gets Over.
  Run.prototype.pointer = function (p) {
    this.refresh();
    const st = this.st;
    st.pointer = p ? { x: p.x, y: p.y } : null;
    const now = p ? hitIn(this.M, st, this.nodes, this.host, p.x, p.y) : null;
    const same = now === null ? st.hover === null : samePath(now, st.hover);
    if (!same) {
      const old = st.hover;
      st.hover = now;
      this.dirty = true;
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
    for (const l of ls) this.runList(l.owner, l.h, true);
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

  return { Player: Player, evaluate: evaluate, applyEffect: applyEffect, mul: mul, meanScale: meanScale, finite: finite };
})();
