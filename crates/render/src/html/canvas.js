// AniCraft HTML canvas player: the page (ADR 0028). SPDX-License-Identifier: MIT OR Apache-2.0
//
// Draws what player.js evaluates on a <canvas> 2D context, steps the movie at its frame rate,
// sends the pointer to its buttons and plays its sounds with Web Audio; the viewer types into
// input text fields through real form elements laid over the canvas. Drawing follows the
// CPU renderer (crates/render, the oracle): fills by their rule, round-joined strokes of the
// stage-space width, colour effects baked into colours, a symbol that fades or blends as one
// drawn on its own layer first, and masks as the union of their fills.
(function () {
  "use strict";
  const data = document.getElementById("anicraft-movie");
  const canvas = document.getElementById("anicraft-stage");
  if (!data || !canvas || !canvas.getContext) return;
  const movie = JSON.parse(data.textContent);
  const W = movie.stage.w, H = movie.stage.h, fit = movie.fit !== false;
  const ctx = canvas.getContext("2d");
  const paths = movie.paths.map((d) => new Path2D(d));
  const images = movie.images.map((src) => {
    const im = new Image();
    im.src = src;
    return im;
  });
  const patterns = new Map();
  const scratch = document.createElement("canvas").getContext("2d");

  // ----- hit tests, by the canvas -----------------------------------------------------------------
  const host = {
    inFill: function (p, m, x, y, eo) {
      const path = paths[p];
      if (!path) return false;
      scratch.setTransform(m[0], m[1], m[2], m[3], m[4], m[5]);
      return scratch.isPointInPath(path, x, y, eo ? "evenodd" : "nonzero");
    },
    nearStroke: function (p, m, x, y, reach) {
      const path = paths[p];
      if (!path || !(reach > 0)) return false;
      scratch.setTransform(1, 0, 0, 1, 0, 0);
      scratch.lineWidth = reach * 2;
      scratch.lineJoin = "round";
      scratch.lineCap = "round";
      return scratch.isPointInStroke(stagePath(path, m), x, y);
    },
  };
  // Each page load gets its own Math.random() sequence (a movie's, not for security); the
  // player itself is deterministic.
  const seed = Date.now() % 4294967296;
  const player = new AniCraft.Player(movie, host, seed);

  function stagePath(path, m) {
    const out = new Path2D();
    out.addPath(path, { a: m[0], b: m[1], c: m[2], d: m[3], e: m[4], f: m[5] });
    return out;
  }
  function css(c) {
    const b = (v) => Math.round(Math.min(Math.max(v, 0), 1) * 255);
    return "rgba(" + b(c[0]) + "," + b(c[1]) + "," + b(c[2]) + "," + Math.min(Math.max(c[3], 0), 1) + ")";
  }

  // ----- the canvas size: the stage fitted to the window, sharp on high-density screens ------------
  let scale = 1;
  function resize() {
    const dpr = window.devicePixelRatio || 1;
    let w = W, h = H;
    if (fit) {
      const k = Math.min(window.innerWidth / W, window.innerHeight / H);
      if (k > 0 && Number.isFinite(k)) {
        w = W * k;
        h = H * k;
      }
    }
    canvas.style.width = w + "px";
    canvas.style.height = h + "px";
    const bw = Math.max(1, Math.round(w * dpr)), bh = Math.max(1, Math.round(h * dpr));
    if (canvas.width !== bw || canvas.height !== bh) {
      canvas.width = bw;
      canvas.height = bh;
      pool.length = 0;
    }
    scale = bw / W;
    draw();
  }

  // ----- drawing ----------------------------------------------------------------------------------
  // Offscreen layers for groups and clips, reused frame to frame.
  const pool = [];
  function layer() {
    const c = pool.pop() || document.createElement("canvas");
    if (c.width !== canvas.width || c.height !== canvas.height) {
      c.width = canvas.width;
      c.height = canvas.height;
    }
    const g = c.getContext("2d");
    g.setTransform(1, 0, 0, 1, 0, 0);
    g.globalAlpha = 1;
    g.globalCompositeOperation = "source-over";
    g.clearRect(0, 0, c.width, c.height);
    return { canvas: c, ctx: g };
  }
  function release(l) {
    pool.push(l.canvas);
  }
  // Canvas compositing for each blend mode; Subtract and Invert have none and draw normally.
  const BLEND = { normal: "source-over", layer: "source-over", multiply: "multiply", screen: "screen", overlay: "overlay", darken: "darken", lighten: "lighten", hardlight: "hard-light", add: "lighter", difference: "difference", alpha: "destination-in", erase: "destination-out", subtract: "source-over", invert: "source-over" };

  function paintStyle(g, f, fx) {
    const p = movie.paints[f];
    if (!p) return null;
    if (p.s) return css(AniCraft.applyEffect(fx, p.s));
    if (p.l || p.r) {
      const gr = p.l ? g.createLinearGradient(p.l[0], p.l[1], p.l[2], p.l[3]) : g.createRadialGradient(p.r[0], p.r[1], 0, p.r[0], p.r[1], p.r[2]);
      for (const s of p.st) gr.addColorStop(s[0], css(AniCraft.applyEffect(fx, [s[1], s[2], s[3], s[4]])));
      return gr;
    }
    if (p.b !== undefined) {
      const im = images[p.b];
      if (!im || !im.complete || !im.naturalWidth) return "rgba(128,128,128,1)";
      // A colour effect beyond alpha draws the bitmap's pixels through it (ADR 0056), as the
      // renderer does: once per bitmap and effect while the effect stays the same.
      const recolour = AniCraft.recolours(fx);
      const key = recolour ? f + ":" + fx.join(",") : f;
      let pat = patterns.get(key);
      if (!pat) {
        if (patterns.size > 256) patterns.clear();
        pat = g.createPattern(recolour ? recoloured(im, fx) : im, p.rep ? "repeat" : "no-repeat");
        if (pat && p.m && pat.setTransform) pat.setTransform(new DOMMatrix(p.m));
        patterns.set(key, pat);
      }
      return pat;
    }
    return null;
  }

  // `im`'s pixels through colour transform `fx`, on a canvas of its size.
  function recoloured(im, fx) {
    const c = document.createElement("canvas");
    c.width = im.naturalWidth;
    c.height = im.naturalHeight;
    const g = c.getContext("2d");
    g.drawImage(im, 0, 0);
    let px;
    try {
      px = g.getImageData(0, 0, c.width, c.height);
    } catch (e) {
      return im;
    }
    const d = px.data;
    for (let i = 0; i < d.length; i += 4) {
      const o = AniCraft.applyEffect(fx, [d[i] / 255, d[i + 1] / 255, d[i + 2] / 255, d[i + 3] / 255]);
      d[i] = Math.round(o[0] * 255);
      d[i + 1] = Math.round(o[1] * 255);
      d[i + 2] = Math.round(o[2] * 255);
      d[i + 3] = Math.round(o[3] * 255);
    }
    g.putImageData(px, 0, 0);
    return c;
  }

  // ----- text fields (ADR 0066) ------------------------------------------------------------------
  // A field draws its background, its text cut to its box, and its border, as the renderer does.
  // Its authored text is its outline (the author's fonts, as everywhere); text a script or the
  // viewer changed is drawn here with the browser's fonts. An input field is a real <input> (one
  // line, or a password) or <textarea> laid over the canvas where the field is, so typing,
  // selection, IME, the clipboard and screen readers are the browser's own: the canvas draws its
  // background and border, the element its text and caret, and it tells the player what was
  // typed (`player.typed`) and where the keyboard is (`player.focus`).
  const overlay = document.createElement("div");
  overlay.style.cssText = "position:absolute;left:0;top:0;width:0;height:0;overflow:visible;pointer-events:none";
  document.body.appendChild(overlay);
  // The input elements on the stage, by the field's instance-path key.
  const inputs = new Map();
  const MAX_LINES = 1000;
  function ours(t) {
    return !!t && t.dataset !== undefined && t.dataset.anicraftField !== undefined;
  }
  function fieldFont(tf) {
    const f = tf.font || {};
    const family = String(f.family || "").replace(/["\\\r\n;{}]/g, "");
    return (f.italic ? "italic " : "") + (f.bold ? "bold " : "") + (f.size || 12) + "px " + (family ? '"' + family + '", ' : "") + "sans-serif";
  }
  function lineHeight(tf) {
    const f = tf.font || {};
    return Math.max((f.size || 12) * 1.2 + (f.lsp || 0), 1);
  }
  function fieldAlign(tf) {
    const a = tf.font && tf.font.align;
    return a === "center" || a === "right" ? a : "left";
  }
  function fieldColour(f) {
    const p = movie.paints[f.tf.f];
    return css(AniCraft.applyEffect(f.c, p && p.s ? p.s : [0, 0, 0, 1]));
  }
  // `text` as the field's lines: broken where it breaks, and in a Multiline field wrapped at the
  // box less its gutters (greedily, at spaces).
  function fieldLines(g, tf, text) {
    const out = [];
    const wrap = tf.ln === "multiline" ? Math.max(tf.w - 4, 1) : 0;
    for (const para of text.split("\n")) {
      if (out.length >= MAX_LINES) break;
      if (!wrap) {
        out.push(para);
        continue;
      }
      let line = "";
      for (const word of para.split(" ")) {
        const next = line === "" ? word : line + " " + word;
        if (line !== "" && g.measureText(next).width > wrap) {
          out.push(line);
          line = word;
        } else line = next;
      }
      out.push(line);
    }
    return out;
  }
  // Text a script or the viewer changed, in the field's font, colour and alignment.
  function fieldText(g, f) {
    const tf = f.tf;
    const shown = tf.k === "input" && tf.ln === "password" ? Array.from(f.text, () => "*").join("") : f.text;
    g.font = fieldFont(tf);
    g.fillStyle = fieldColour(f);
    g.textBaseline = "top";
    const align = fieldAlign(tf);
    g.textAlign = align;
    if ("letterSpacing" in g) g.letterSpacing = ((tf.font && tf.font.ls) || 0) + "px";
    const x = align === "center" ? tf.w / 2 : align === "right" ? tf.w - 2 : 2;
    const lh = lineHeight(tf);
    const lines = fieldLines(g, tf, shown);
    for (let i = 0; i < lines.length; i++) {
      const y = 2 + i * lh;
      if (y > tf.h) break;
      g.fillText(lines[i], x, y);
    }
  }
  // One field on `g` (`set` places a context by a stage transform); input fields with an
  // instance path go into `seen`, to get their elements.
  function drawField(g, f, set, seen) {
    const tf = f.tf, m = f.m, box = paths[tf.r];
    if (!AniCraft.finite(m) || !box) return;
    if (tf.bg !== undefined) {
      const style = paintStyle(g, tf.bg, f.c);
      if (style) {
        set(g, m);
        g.fillStyle = style;
        g.fill(box);
      }
    }
    const element = tf.k === "input" && f.key !== null;
    if (element) seen.set(f.key, f);
    // While its element shows an input field's text, the canvas doesn't.
    else {
      g.save();
      set(g, m);
      g.beginPath();
      g.rect(0, 0, tf.w, tf.h);
      g.clip();
      if (f.text === null) {
        const p = paths[tf.p];
        if (p) {
          g.fillStyle = fieldColour(f);
          g.fill(p);
        }
      } else fieldText(g, f);
      g.restore();
    }
    if (tf.bd) {
      // 1 px in the field's space, mitred, like any stroke (stage width × the mean scale).
      g.setTransform(scale, 0, 0, scale, 0, 0);
      g.lineWidth = AniCraft.meanScale(m);
      g.lineJoin = "miter";
      g.lineCap = "butt";
      g.strokeStyle = css(AniCraft.applyEffect(f.c, tf.bd));
      g.stroke(stagePath(box, m));
      g.lineJoin = "round";
      g.lineCap = "round";
    }
  }
  function makeInput(key, f) {
    const tf = f.tf, font = tf.font || {};
    const multi = tf.ln === "multiline" || tf.ln === "multiline_no_wrap";
    const el = document.createElement(multi ? "textarea" : "input");
    if (!multi) el.type = tf.ln === "password" ? "password" : "text";
    else el.wrap = tf.ln === "multiline" ? "soft" : "off";
    el.dataset.anicraftField = key;
    el.setAttribute("aria-label", f.name || "Text field");
    el.setAttribute("autocomplete", "off");
    el.setAttribute("autocapitalize", "off");
    el.spellcheck = false;
    if (tf.mx > 0) el.maxLength = tf.mx;
    const lh = lineHeight(tf);
    // The box at its size in the field's space; the transform places and scales it.
    const h = multi ? tf.h : Math.min(tf.h, lh + 4);
    el.style.cssText = "position:absolute;left:0;top:0;margin:0;border:0;outline:none;background:transparent;box-shadow:none;box-sizing:border-box;padding:2px;resize:none;overflow:hidden;pointer-events:auto;transform-origin:0 0;-webkit-appearance:none;appearance:none";
    el.style.width = tf.w + "px";
    el.style.height = h + "px";
    el.style.font = fieldFont(tf);
    el.style.lineHeight = lh + "px";
    el.style.letterSpacing = (font.ls || 0) + "px";
    el.style.textAlign = fieldAlign(tf);
    if (multi) el.style.whiteSpace = tf.ln === "multiline" ? "pre-wrap" : "pre";
    el._shown = null;
    el._composing = false;
    // What the viewer typed goes to the player as it is typed (an IME's once composed).
    const typed = () => {
      const v = player.typed(key, el.value);
      if (v === null) return;
      if (v !== el.value) el.value = v;
      el._shown = v;
      draw();
    };
    el.addEventListener("input", (e) => {
      if (!el._composing && !e.isComposing) typed();
    });
    el.addEventListener("compositionstart", () => (el._composing = true));
    el.addEventListener("compositionend", () => {
      el._composing = false;
      typed();
    });
    el.addEventListener("focus", () => {
      audioOn();
      player.focus(key);
    });
    el.addEventListener("blur", () => {
      const f = player.st.focus;
      if (f !== null && f.join(",") === key) player.focus(null);
    });
    return el;
  }
  // The input elements where their fields are now, showing their text; elements of fields that
  // left the stage go, and the keyboard goes where the player has it.
  function placeInputs(seen) {
    const r = canvas.getBoundingClientRect();
    overlay.style.left = r.left + window.scrollX + "px";
    overlay.style.top = r.top + window.scrollY + "px";
    const kc = r.width / W;
    for (const [key, f] of seen) {
      let el = inputs.get(key);
      if (!el) {
        el = makeInput(key, f);
        inputs.set(key, el);
        overlay.appendChild(el);
      }
      el.style.transform = "matrix(" + f.m.map((v) => v * kc).join(",") + ")";
      const colour = fieldColour(f);
      if (el._colour !== colour) {
        el.style.color = colour;
        el.style.caretColor = colour;
        el._colour = colour;
      }
      // A script set its text (setting it unchanged would move the viewer's caret).
      const v = f.text !== null ? f.text : f.tf.t;
      if (v !== el._shown && !el._composing) {
        el.value = v;
        el._shown = v;
      }
    }
    for (const [key, el] of Array.from(inputs)) {
      if (seen.has(key)) continue;
      inputs.delete(key);
      if (document.activeElement === el) el.blur();
      el.remove();
    }
    const want = player.st.focus === null ? undefined : inputs.get(player.st.focus.join(","));
    const active = document.activeElement;
    if (want && active !== want) want.focus({ preventScroll: true });
    else if (!want && ours(active)) active.blur();
  }

  function draw() {
    const stack = [{ ctx: ctx, canvas: canvas }];
    const seen = new Map();
    const masks = new Map();
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.globalAlpha = 1;
    ctx.globalCompositeOperation = "source-over";
    ctx.fillStyle = movie.stage.bg;
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    const k = scale;
    const set = (g, m) => g.setTransform(k * m[0], k * m[1], k * m[2], k * m[3], k * m[4], k * m[5]);
    player.evaluate({
      item: function (it) {
        const path = paths[it.p];
        if (!path || !AniCraft.finite(it.m)) return;
        if (it.masks !== null) {
          // Mask artwork: its fill area only, opaque.
          const mk = masks.get(it.masks);
          if (!mk || it.f < 0) return;
          set(mk.ctx, it.m);
          mk.ctx.fillStyle = "#000";
          mk.ctx.fill(path, it.eo ? "evenodd" : "nonzero");
          return;
        }
        const g = stack[stack.length - 1].ctx;
        if (it.f >= 0) {
          const style = paintStyle(g, it.f, it.c);
          if (style) {
            const bitmap = movie.paints[it.f] && movie.paints[it.f].b !== undefined;
            g.globalAlpha = bitmap && !AniCraft.recolours(it.c) ? Math.min(Math.max(it.c[3], 0), 1) : 1;
            set(g, it.m);
            g.fillStyle = style;
            // A bitmap without Allow Smoothing keeps hard pixel edges (ADR 0050).
            const hard = bitmap && movie.paints[it.f].px === true;
            if (hard) g.imageSmoothingEnabled = false;
            g.fill(path, it.eo ? "evenodd" : "nonzero");
            if (hard) g.imageSmoothingEnabled = true;
            g.globalAlpha = 1;
          }
        }
        if (it.s) {
          const scale = AniCraft.meanScale(it.m);
          const w = it.s[4] * scale * k;
          if (w > 0 && Number.isFinite(w)) {
            g.setTransform(k, 0, 0, k, 0, 0);
            g.lineWidth = w / k;
            // Caps, joins and dashes (stage units: local lengths × the mean scale, like the width).
            const st = it.k;
            g.lineJoin = st ? st.j : "round";
            g.lineCap = st ? st.c : "round";
            if (st) g.miterLimit = st.ml;
            const dashed = st && st.d && st.d.length > 0;
            if (dashed) {
              g.setLineDash(st.d.map((x) => x * scale));
              g.lineDashOffset = (st.o || 0) * scale;
            }
            g.strokeStyle = css(AniCraft.applyEffect(it.c, it.s));
            g.stroke(stagePath(path, it.m));
            if (dashed) {
              g.setLineDash([]);
              g.lineDashOffset = 0;
            }
            if (st) g.miterLimit = 10;
          }
        }
      },
      field: function (f) {
        drawField(stack[stack.length - 1].ctx, f, set, seen);
      },
      groupBegin: function () {
        stack.push(layer());
      },
      groupEnd: function (_, grp) {
        const l = stack.pop();
        const g = stack[stack.length - 1].ctx;
        g.setTransform(1, 0, 0, 1, 0, 0);
        g.globalAlpha = Math.min(Math.max(grp.alpha, 0), 1);
        g.globalCompositeOperation = BLEND[grp.blend] || "source-over";
        g.drawImage(l.canvas, 0, 0);
        g.globalAlpha = 1;
        g.globalCompositeOperation = "source-over";
        release(l);
      },
      clipBegin: function () {
        stack.push(layer());
      },
      // A clip that ends without meeting its mask shows nothing.
      clipEnd: function () {
        release(stack.pop());
      },
      maskBegin: function (id) {
        masks.set(id, layer());
      },
      maskEnd: function (id, open) {
        const mk = masks.get(id);
        masks.delete(id);
        if (!mk) return;
        if (open) {
          const l = stack.pop();
          l.ctx.setTransform(1, 0, 0, 1, 0, 0);
          l.ctx.globalCompositeOperation = "destination-in";
          l.ctx.drawImage(mk.canvas, 0, 0);
          l.ctx.globalCompositeOperation = "source-over";
          const g = stack[stack.length - 1].ctx;
          g.setTransform(1, 0, 0, 1, 0, 0);
          g.drawImage(l.canvas, 0, 0);
          release(l);
        }
        release(mk);
      },
    });
    placeInputs(seen);
  }

  // ----- sound ------------------------------------------------------------------------------------
  // Event and Start sounds play in full from their keyframe; Stream sounds play over their
  // keyframe's span; Stop stops the sound. Each is heard at its layers' volume, through its
  // effect's envelope (a gain per channel, straight lines between [seconds, left, right]
  // points). Browsers start audio only after a click or a key.
  let audio = null;
  const buffers = [];
  const voices = [];
  function audioOn() {
    if (!movie.sounds.length) return;
    if (!audio) {
      const AC = window.AudioContext || window.webkitAudioContext;
      if (!AC) return;
      audio = new AC();
      movie.sounds.forEach((s, i) => {
        const bin = atob(s.data);
        const bytes = new Uint8Array(bin.length);
        for (let j = 0; j < bin.length; j++) bytes[j] = bin.charCodeAt(j);
        audio.decodeAudioData(bytes.buffer).then((b) => (buffers[i] = b), () => console.warn("AniCraft: this browser can't play the sound “" + s.name + "”"));
      });
    }
    if (audio.state === "suspended") audio.resume();
  }
  function play(sounds) {
    if (!audio || audio.state !== "running") return;
    for (const [i, sync, repeat, loop, span, gain, env] of sounds) {
      const b = buffers[i];
      const playing = voices.filter((v) => v.sound === i && v.end > audio.currentTime);
      if (sync === "stop") {
        for (const v of playing) v.src.stop();
        continue;
      }
      if (!b || (sync === "start" && playing.length)) continue;
      const src = audio.createBufferSource();
      src.buffer = b;
      const t = audio.currentTime;
      let out = src;
      if (env && env.length) {
        // Up to stereo (a mono sound on both sides), split, one gain per channel, merged.
        const up = audio.createGain();
        up.channelCount = 2;
        up.channelCountMode = "explicit";
        up.channelInterpretation = "speakers";
        const split = audio.createChannelSplitter(2);
        const merge = audio.createChannelMerger(2);
        src.connect(up);
        up.connect(split);
        for (const c of [0, 1]) {
          const g = audio.createGain();
          g.gain.setValueAtTime(env[0][1 + c], t);
          for (const p of env) g.gain.linearRampToValueAtTime(p[1 + c], t + p[0]);
          split.connect(g, c);
          g.connect(merge, 0, c);
        }
        out = merge;
      }
      const volume = audio.createGain();
      volume.gain.value = gain === undefined || gain === null ? 1 : gain;
      out.connect(volume);
      volume.connect(audio.destination);
      let end = t + b.duration * Math.max(repeat, 1);
      if (sync === "stream") end = t + span / movie.stage.fps;
      src.loop = loop || repeat > 1 || sync === "stream";
      src.start(t);
      if (!loop || sync === "stream") src.stop(end);
      voices.push({ sound: i, src: src, end: loop && sync !== "stream" ? Infinity : end });
    }
    for (let j = voices.length - 1; j >= 0; j--) if (voices[j].end <= audio.currentTime) voices.splice(j, 1);
  }

  // ----- playback ---------------------------------------------------------------------------------
  function report(r) {
    for (const line of r.output) console.log(line);
    play(r.sounds);
  }
  function stagePoint(e) {
    const r = canvas.getBoundingClientRect();
    return { x: ((e.clientX - r.left) * W) / Math.max(r.width, 1), y: ((e.clientY - r.top) * H) / Math.max(r.height, 1) };
  }
  function pointer(type, e) {
    const p = stagePoint(e);
    report(player.input([{ type: type, x: p.x, y: p.y }]));
    canvas.style.cursor = player.st.hover ? "pointer" : "default";
    draw();
  }
  canvas.addEventListener("pointermove", (e) => pointer("move", e));
  canvas.addEventListener("pointerdown", (e) => {
    audioOn();
    if (canvas.setPointerCapture) canvas.setPointerCapture(e.pointerId);
    pointer("down", e);
  });
  canvas.addEventListener("pointerup", (e) => pointer("up", e));
  canvas.addEventListener("pointerleave", () => {
    report(player.input([{ type: "leave" }]));
    draw();
  });
  // Keys go to the movie's keyboard listeners (ActionScript key codes are the browser's).
  function key(type, e) {
    if (e.repeat && type === "keyup") return;
    // Typing in one of the movie's input fields: the element edits it (and keeps its arrows and
    // space, so nothing is prevented here); the listeners hear the key once the edit is in, as
    // in the player.
    if (ours(e.target)) {
      const code = e.keyCode;
      setTimeout(() => {
        report(player.input([{ type: type, code: code, dom: true }]));
        draw();
      }, 0);
      return;
    }
    report(player.input([{ type: type, code: e.keyCode }]));
    // Arrows and space would scroll the page; a movie that listens for keys keeps them.
    if (player.st.stage.some((l) => l.ev === "key_down" || l.ev === "key_up") && [32, 37, 38, 39, 40].indexOf(e.keyCode) >= 0) e.preventDefault();
    draw();
  }
  window.addEventListener("keydown", (e) => {
    audioOn();
    key("keydown", e);
  });
  window.addEventListener("keyup", (e) => key("keyup", e));
  window.addEventListener("resize", resize);

  const dt = 1000 / movie.stage.fps;
  let last = null, acc = 0;
  function tick(t) {
    if (last === null) last = t;
    acc += Math.min(t - last, 250);
    last = t;
    let stepped = false;
    while (acc >= dt) {
      acc -= dt;
      report(player.step([]));
      stepped = true;
    }
    if (stepped) draw();
    requestAnimationFrame(tick);
  }
  // Images first, so the first frame shows them.
  Promise.all(images.map((im) => (im.decode ? im.decode().catch(() => {}) : Promise.resolve()))).then(() => {
    report(player.start());
    resize();
    requestAnimationFrame(tick);
  }).catch((e) => console.error("AniCraft: the movie couldn't start", e));
})();
