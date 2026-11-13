// AniCraft HTML canvas player: filters and per-pixel blending (ADR 0072).
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// A port of crates/render/src/filters.rs (the CPU renderer, the oracle) and of its `blend` for
// the modes canvas 2D has no operation for (Subtract, Invert). It works on premultiplied RGBA
// in a Float32Array, `w × h`, row by row, exactly as the oracle's buffer: the same box-blur
// passes (widths and offsets rounded from the same f32 products), the same shadow, glow and
// bevel formulas over the blurred opacity, the same ramp tables and colour matrices. It is our
// own blur over the pixels, not CSS `filter`, so every browser shows the same result; the tests
// run it in a JavaScript engine against the oracle. Nothing here touches the page.
"use strict";
var AniCraftFilters = (function () {
  const f32 = Math.fround;
  // Steps in a gradient filter's ramp table (filters.rs `RAMP_STEPS`).
  const RAMP_STEPS = 256;

  // f32::round: halves away from zero.
  function round(x) {
    return x < 0 ? -Math.round(-x) : Math.round(x);
  }
  function num(v, d) {
    return typeof v === "number" && Number.isFinite(v) ? v : d;
  }
  function clamp(v, lo, hi) {
    return v < lo ? lo : v > hi ? hi : v;
  }
  function box(bx, by, q, scale) {
    return { nx: Math.max(round(f32(num(bx, 0) * scale)), 0), ny: Math.max(round(f32(num(by, 0) * scale)), 0), passes: Math.max(num(q, 1) | 0, 1) };
  }
  // Whole-pixel offset of `distance` (already scaled) along the unit vector `u` (the renderer's
  // own cosine and sine of the angle, so the products, and their rounding, are its own).
  function offset(u, distance) {
    const c = Array.isArray(u) ? f32(num(u[0], 0)) : 0, s = Array.isArray(u) ? f32(num(u[1], 0)) : 0;
    return [round(f32(distance * c)), round(f32(distance * s))];
  }
  function sane(c) {
    const s = (v) => (typeof v === "number" && Number.isFinite(v) ? clamp(v, 0, 1) : 0);
    return Array.isArray(c) ? [s(c[0]), s(c[1]), s(c[2]), s(c[3])] : [0, 0, 0, 0];
  }
  function premultiply(c) {
    const s = sane(c);
    return [f32(s[0] * s[3]), f32(s[1] * s[3]), f32(s[2] * s[3]), s[3]];
  }
  // filter.rs `ramp_at` over [offset, r, g, b, a] stops sorted by offset.
  function rampAt(stops, t) {
    if (!stops.length) return [0, 0, 0, 0];
    const first = stops[0];
    if (Number.isNaN(t) || t <= first[0]) return first.slice(1);
    for (let i = 0; i + 1 < stops.length; i++) {
      const a = stops[i], b = stops[i + 1];
      if (t <= b[0]) {
        const span = f32(b[0] - a[0]);
        const k = span > 1e-6 ? f32(f32(t - a[0]) / span) : 1;
        const out = new Array(4);
        for (let j = 0; j < 4; j++) out[j] = f32(a[j + 1] + f32(f32(b[j + 1] - a[j + 1]) * k));
        return out;
      }
    }
    return stops[stops.length - 1].slice(1);
  }
  function rampLut(stops) {
    const st = Array.isArray(stops) ? stops.filter((s) => Array.isArray(s) && s.length >= 5) : [];
    const out = [];
    for (let i = 0; i <= RAMP_STEPS; i++) out.push(premultiply(rampAt(st, f32(i / RAMP_STEPS))));
    return out;
  }
  function lutAt(lut, t) {
    const i = Number.isFinite(t) ? round(f32(clamp(t, 0, 1) * RAMP_STEPS)) : 0;
    return lut[i] || [0, 0, 0, 0];
  }

  // An exported filter (html.rs `filter_json`) at view scale `scale`, as filters.rs `resolve`.
  function resolve(f, scale) {
    scale = Number.isFinite(scale) && scale > 0 ? f32(scale) : 1;
    if (!f || typeof f !== "object") return null;
    const blur = box(f.x, f.y, f.q, scale);
    const dist = f32(num(f.d, 0) * scale);
    const strength = num(f.s, 1);
    const side = f.side === "outer" || f.side === "full" ? f.side : "inner";
    switch (f.k) {
      case "blur":
        return { k: "blur", blur: blur };
      case "shadow": {
        const o = offset(f.u, dist);
        return { k: "shadow", blur: blur, dx: o[0], dy: o[1], strength: strength, color: sane(f.c), inner: !!f.in, knockout: !!f.ko, hide: !!f.ho };
      }
      case "bevel": {
        const o = offset(f.u, dist);
        const paint = f.st ? { ramp: rampLut(f.st) } : { hi: premultiply(f.hc), sh: premultiply(f.sc) };
        return { k: "bevel", blur: blur, dx: o[0], dy: o[1], strength: strength, paint: paint, side: side, knockout: !!f.ko };
      }
      case "gglow": {
        const o = offset(f.u, dist);
        return { k: "gglow", blur: blur, dx: o[0], dy: o[1], strength: strength, ramp: rampLut(f.st), side: side, knockout: !!f.ko };
      }
      case "adjust":
        return Array.isArray(f.m) && f.m.length === 20 ? { k: "adjust", m: f.m.map((v) => num(v, 0)) } : null;
    }
    return null;
  }

  // Box blur of width `n` along rows or columns of a `c`-channel buffer, centred (filters.rs
  // `box_pass`): an odd width covers n/2 pixels each side, an even one a pixel more each side at
  // half weight. Outside the buffer is transparent.
  //
  // Both directions walk the buffer row by row (memory order), reading a copy of it (`src`): a
  // row pass keeps one running sum per channel, a column pass one per column and channel.
  function rowPass(buf, src, c, w, h, n) {
    const r = n >> 1, even = (n & 1) === 0, inv = f32(1 / n);
    const first = r < w - 1 ? r : w - 1;
    // Pixels whose window lies inside the row: no edge tests.
    const a = Math.min(r, w), b = Math.max(w - r - 1, a);
    const back = r * c, ahead = (r + 1) * c, half = 0.5 * inv;
    for (let y = 0; y < h; y++) {
      const row = y * w * c;
      for (let k = 0; k < c; k++) {
        const base = row + k;
        let sum = 0;
        for (let i = 0; i <= first; i++) sum += src[base + i * c];
        const edge = (i) => {
          const lo = i - r, hi = i + r, p = base + i * c;
          buf[p] = even ? (sum - 0.5 * ((lo >= 0 ? src[base + lo * c] : 0) + (hi < w ? src[base + hi * c] : 0))) * inv : sum * inv;
          if (hi + 1 < w) sum += src[base + (hi + 1) * c];
          if (lo >= 0) sum -= src[base + lo * c];
        };
        for (let i = 0; i < a; i++) edge(i);
        const end = base + b * c;
        if (even)
          for (let p = base + a * c; p < end; p += c) {
            const lo = src[p - back];
            buf[p] = sum * inv - (lo + src[p + back]) * half;
            sum += src[p + ahead] - lo;
          }
        else
          for (let p = base + a * c; p < end; p += c) {
            buf[p] = sum * inv;
            sum += src[p + ahead] - src[p - back];
          }
        for (let i = b; i < w; i++) edge(i);
      }
    }
  }
  function columnPass(buf, src, sums, c, w, h, n) {
    const r = n >> 1, even = (n & 1) === 0, inv = f32(1 / n);
    const stride = w * c;
    const first = r < h - 1 ? r : h - 1;
    sums.fill(0);
    for (let i = 0; i <= first; i++) for (let j = 0, p = i * stride; j < stride; j++, p++) sums[j] += src[p];
    const half = 0.5 * inv;
    for (let i = 0; i < h; i++) {
      const lo = i - r, hi = i + r, out = i * stride;
      const lop = lo * stride, hip = hi * stride, add = (hi + 1) * stride;
      if (lo >= 0 && hi + 1 < h) {
        // The window inside the buffer: no edge tests.
        if (even)
          for (let j = 0; j < stride; j++) {
            const s = sums[j], l = src[lop + j];
            buf[out + j] = s * inv - (l + src[hip + j]) * half;
            sums[j] = s + src[add + j] - l;
          }
        else
          for (let j = 0; j < stride; j++) {
            const s = sums[j];
            buf[out + j] = s * inv;
            sums[j] = s + src[add + j] - src[lop + j];
          }
        continue;
      }
      for (let j = 0; j < stride; j++) {
        const s = sums[j];
        const l = lo >= 0 ? src[lop + j] : 0;
        buf[out + j] = even ? (s - 0.5 * (l + (hi < h ? src[hip + j] : 0))) * inv : s * inv;
        sums[j] = s + (hi + 1 < h ? src[add + j] : 0) - l;
      }
    }
  }
  function blur(buf, c, w, h, b) {
    if (b.nx <= 1 && b.ny <= 1) return;
    const src = new Float32Array(buf.length);
    const sums = b.ny > 1 ? new Float64Array(w * c) : null;
    for (let i = 0; i < b.passes; i++) {
      if (b.nx > 1) {
        src.set(buf);
        rowPass(buf, src, c, w, h, b.nx);
      }
      if (b.ny > 1) {
        src.set(buf);
        columnPass(buf, src, sums, c, w, h, b.ny);
      }
    }
  }
  // The opacity at (x, y), or 0 outside.
  function alphaAt(buf, w, h, x, y) {
    return x >= 0 && y >= 0 && x < w && y < h ? buf[(y * w + x) * 4 + 3] : 0;
  }
  // The artwork's opacity (or `inverse`, its transparency) moved by (dx, dy).
  function shifted(buf, w, h, dx, dy, inverse) {
    const cov = new Float32Array(w * h);
    for (let y = 0; y < h; y++)
      for (let x = 0; x < w; x++) {
        const a = alphaAt(buf, w, h, x - dx, y - dy);
        cov[y * w + x] = inverse ? 1 - a : a;
      }
    return cov;
  }

  function shadow(buf, w, h, s) {
    const cov = shifted(buf, w, h, s.dx, s.dy, s.inner);
    blur(cov, 1, w, h, s.blur);
    const c = s.color;
    for (let i = 0, p = 0; i < w * h; i++, p += 4) {
      const sa = clamp(cov[i] * s.strength, 0, 1) * c[3];
      const ba = buf[p + 3];
      if (s.inner) {
        const a = sa * ba;
        if (s.knockout) {
          buf[p] = c[0] * a;
          buf[p + 1] = c[1] * a;
          buf[p + 2] = c[2] * a;
          buf[p + 3] = a;
        } else {
          buf[p] = c[0] * a + buf[p] * (1 - sa);
          buf[p + 1] = c[1] * a + buf[p + 1] * (1 - sa);
          buf[p + 2] = c[2] * a + buf[p + 2] * (1 - sa);
        }
      } else if (s.hide) {
        buf[p] = c[0] * sa;
        buf[p + 1] = c[1] * sa;
        buf[p + 2] = c[2] * sa;
        buf[p + 3] = sa;
      } else if (s.knockout) {
        const a = sa * (1 - ba);
        buf[p] = c[0] * a;
        buf[p + 1] = c[1] * a;
        buf[p + 2] = c[2] * a;
        buf[p + 3] = a;
      } else {
        const k = 1 - ba;
        buf[p] += c[0] * sa * k;
        buf[p + 1] += c[1] * sa * k;
        buf[p + 2] += c[2] * sa * k;
        buf[p + 3] = ba + sa * k;
      }
    }
  }

  // The bevel's signed field: the blurred opacity a distance towards the light minus away.
  function bevelField(buf, w, h, b, dx, dy) {
    const a = new Float32Array(w * h);
    for (let i = 0; i < w * h; i++) a[i] = buf[i * 4 + 3];
    blur(a, 1, w, h, b);
    const at = (x, y) => (x >= 0 && y >= 0 && x < w && y < h ? a[y * w + x] : 0);
    const out = new Float32Array(w * h);
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) out[y * w + x] = clamp(at(x + dx, y + dy) - at(x - dx, y - dy), -1, 1);
    return out;
  }

  // An effect with the artwork (filters.rs `composite`): `inner` atop it, `outer` under it, as
  // `side` says; Knockout keeps only the effect. Both premultiplied, 4 floats a pixel, or null.
  function composite(buf, inner, outer, side, knockout) {
    for (let i = 0; i < buf.length; i += 4) {
      const ba = buf[i + 3];
      const eia = side !== "outer" && inner ? inner[i + 3] : 0;
      for (let c = 0; c < 4; c++) {
        const ei = side !== "outer" && inner ? inner[i + c] : 0;
        const eo = side !== "inner" && outer ? outer[i + c] : 0;
        buf[i + c] = knockout ? ei * ba + eo * (1 - ba) : ei * ba + buf[i + c] * (1 - eia) + eo * (1 - ba);
      }
    }
  }

  function adjust(buf, m) {
    for (let i = 0; i < buf.length; i += 4) {
      const a = buf[i + 3];
      if (a <= 1e-6) continue;
      const r = buf[i] / a, g = buf[i + 1] / a, b = buf[i + 2] / a;
      const ch = (k) => clamp(m[k * 5] * r + m[k * 5 + 1] * g + m[k * 5 + 2] * b + m[k * 5 + 3] * a + m[k * 5 + 4], 0, 1);
      const oa = ch(3);
      buf[i] = ch(0) * oa;
      buf[i + 1] = ch(1) * oa;
      buf[i + 2] = ch(2) * oa;
      buf[i + 3] = oa;
    }
  }

  // Applies exported filters in order to premultiplied pixels `buf` (Float32Array, w × h × 4),
  // at view scale `scale`, as filters.rs `apply`.
  function apply(buf, w, h, filters, scale) {
    if (!buf || buf.length !== w * h * 4 || w <= 0 || h <= 0 || !Array.isArray(filters)) return;
    for (const f of filters) {
      const op = resolve(f, scale);
      if (!op) continue;
      if (op.k === "blur") blur(buf, 4, w, h, op.blur);
      else if (op.k === "shadow") shadow(buf, w, h, op);
      else if (op.k === "bevel") {
        const field = bevelField(buf, w, h, op.blur, op.dx, op.dy);
        const eff = new Float32Array(w * h * 4);
        const p = op.paint;
        for (let i = 0; i < w * h; i++) {
          const v = clamp(field[i] * op.strength, -1, 1);
          const c = p.ramp ? lutAt(p.ramp, 0.5 - 0.5 * v) : v >= 0 ? p.hi : p.sh;
          const k = p.ramp ? 1 : Math.abs(v);
          for (let j = 0; j < 4; j++) eff[i * 4 + j] = c[j] * k;
        }
        composite(buf, eff, eff, op.side, op.knockout);
      } else if (op.k === "gglow") {
        const glow = (inner) => {
          const cov = shifted(buf, w, h, op.dx, op.dy, inner);
          blur(cov, 1, w, h, op.blur);
          const out = new Float32Array(w * h * 4);
          for (let i = 0; i < w * h; i++) {
            const c = lutAt(op.ramp, clamp(cov[i] * op.strength, 0, 1));
            for (let j = 0; j < 4; j++) out[i * 4 + j] = c[j];
          }
          return out;
        };
        const inner = op.side === "outer" ? null : glow(true);
        const outer = op.side === "inner" ? null : glow(false);
        composite(buf, inner, outer, op.side, op.knockout);
      } else if (op.k === "adjust") adjust(buf, op.m);
    }
  }

  // lib.rs `blend` for one premultiplied source over a premultiplied backdrop, for the modes the
  // page composites itself.
  function blendPixel(mode, s, b, out) {
    const sa = clamp(s[3], 0, 1), ba = clamp(b[3], 0, 1);
    const un = (c, a) => (a > 1e-6 ? clamp(c / a, 0, 1) : 0);
    for (let i = 0; i < 3; i++) {
      const cs = un(s[i], sa), cb = un(b[i], ba);
      const f = mode === "subtract" ? Math.max(cb - cs, 0) : mode === "invert" ? 1 - cb : cs;
      out[i] = (1 - ba) * s[i] + (1 - sa) * b[i] + sa * ba * f;
    }
    out[3] = sa + ba - sa * ba;
    return out;
  }

  // Composites straight 8-bit RGBA `src` (an ImageData's data) at `alpha` onto `dst` (the same
  // size) in `mode` ("subtract" or "invert"), in place, as the oracle's `close_group` does.
  // (The formula of `blendPixel`, written out on straight colour: `s` and `b` premultiplied are
  // `cs·sa` and `cb·ba`.)
  function blendImage(dst, src, alpha, mode) {
    alpha = clamp(num(alpha, 1), 0, 1);
    const invert = mode === "invert";
    const n = Math.min(src.length, dst.length);
    for (let i = 0; i < n; i += 4) {
      const sa = (src[i + 3] / 255) * alpha;
      if (sa <= 0) continue;
      const ba = dst[i + 3] / 255;
      const a = sa + ba - sa * ba;
      for (let c = 0; c < 3; c++) {
        const cs = src[i + c] / 255, cb = dst[i + c] / 255;
        const f = invert ? 1 - cb : cb > cs ? cb - cs : 0;
        const o = (1 - ba) * cs * sa + (1 - sa) * cb * ba + sa * ba * f;
        dst[i + c] = a > 0 ? Math.round(clamp(o / a, 0, 1) * 255) : 0;
      }
      dst[i + 3] = Math.round(a * 255);
    }
  }

  // Straight 8-bit RGBA to premultiplied floats, and back (rounded).
  function premul(px) {
    const out = new Float32Array(px.length);
    for (let i = 0; i < px.length; i += 4) {
      const a = px[i + 3] / 255;
      out[i] = (px[i] / 255) * a;
      out[i + 1] = (px[i + 1] / 255) * a;
      out[i + 2] = (px[i + 2] / 255) * a;
      out[i + 3] = a;
    }
    return out;
  }
  function straight(buf, px) {
    for (let i = 0; i < buf.length && i < px.length; i += 4) {
      const a = clamp(buf[i + 3], 0, 1);
      px[i + 3] = Math.round(a * 255);
      for (let c = 0; c < 3; c++) px[i + c] = a > 1e-6 ? Math.round(clamp(buf[i + c] / a, 0, 1) * 255) : 0;
    }
  }

  // The rectangle [x0, y0, x1, y1) of `w × h` straight RGBA `px` with any opacity, grown by
  // `reach` and kept within the buffer; null when nothing shows. The oracle sizes its buffer by
  // the artwork's bounds grown by the filters' reach: outside that every filter reads
  // transparency, so a buffer at least that large gives the same pixels.
  function region(px, w, h, reach) {
    let x0 = w, y0 = h, x1 = -1, y1 = -1;
    for (let y = 0; y < h; y++) {
      const row = y * w * 4;
      for (let x = 0; x < w; x++)
        if (px[row + x * 4 + 3] !== 0) {
          if (x < x0) x0 = x;
          if (x > x1) x1 = x;
          if (y < y0) y0 = y;
          y1 = y;
        }
    }
    if (x1 < 0) return null;
    const r = Math.max(num(reach, 0) | 0, 0);
    return [Math.max(x0 - r, 0), Math.max(y0 - r, 0), Math.min(x1 + 1 + r, w), Math.min(y1 + 1 + r, h)];
  }

  // How far a group's filters reach in surface pixels (lib.rs `filter_setup`): their margin in
  // stage pixels times the view scale, rounded up, plus 2.
  function reach(margin, scale) {
    return Math.ceil(f32(num(margin, 0) * scale)) + 2;
  }

  return { apply: apply, resolve: resolve, blendPixel: blendPixel, blendImage: blendImage, premul: premul, straight: straight, region: region, reach: reach };
})();
