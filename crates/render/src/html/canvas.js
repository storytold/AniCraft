// AniCraft HTML canvas player: the page (ADR 0028). SPDX-License-Identifier: MIT OR Apache-2.0
//
// Draws what player.js evaluates on a <canvas> 2D context, steps the movie at its frame rate,
// sends the pointer to its buttons and plays its sounds with Web Audio. Drawing follows the
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
  const player = new AniCraft.Player(movie, host);

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
      const key = f;
      let pat = patterns.get(key);
      if (!pat) {
        pat = g.createPattern(im, p.rep ? "repeat" : "no-repeat");
        if (pat && p.m && pat.setTransform) pat.setTransform(new DOMMatrix(p.m));
        patterns.set(key, pat);
      }
      return pat;
    }
    return null;
  }

  function draw() {
    const stack = [{ ctx: ctx, canvas: canvas }];
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
            g.globalAlpha = bitmap ? Math.min(Math.max(it.c[0], 0), 1) : 1;
            set(g, it.m);
            g.fillStyle = style;
            g.fill(path, it.eo ? "evenodd" : "nonzero");
            g.globalAlpha = 1;
          }
        }
        if (it.s) {
          const w = it.s[4] * AniCraft.meanScale(it.m) * k;
          if (w > 0 && Number.isFinite(w)) {
            g.setTransform(k, 0, 0, k, 0, 0);
            g.lineWidth = w / k;
            g.lineJoin = "round";
            g.lineCap = "round";
            g.strokeStyle = css(AniCraft.applyEffect(it.c, it.s));
            g.stroke(stagePath(path, it.m));
          }
        }
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
  window.addEventListener("keydown", audioOn);
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
  });
})();
