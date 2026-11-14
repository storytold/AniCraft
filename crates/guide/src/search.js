// AniCraft User Guide search (ADR 0096). Works offline from a file:// folder: the index is
// search-index.js, loaded with a script tag. The ranking is a port of crates/guide/src/search.rs
// (`anicraft-cli guide search` prints the same results); keep the two in step. Results are built
// with textContent only, never as markup.
(function () {
  "use strict";
  var MAX_QUERY = 200, MAX_TERMS = 8, SNIPPET = 180, LEAD = 60;
  var STOP_WORDS = ["a", "an", "and", "are", "as", "at", "be", "by", "can", "do", "does", "for", "from", "how", "i", "if", "in", "is", "it", "my", "of", "on", "or", "the", "to", "what", "when", "where", "with"];
  var WORD = /[\p{Alphabetic}\p{N}]/u;

  function words(s) {
    var out = [], cur = "", start = 0, chars = Array.from(s);
    for (var i = 0; i < chars.length; i++) {
      var c = chars[i];
      if (WORD.test(c)) {
        if (cur === "") start = i;
        cur += c.toLowerCase();
      } else if (cur !== "") {
        out.push([start, cur]);
        cur = "";
      }
    }
    if (cur !== "") out.push([start, cur]);
    return out;
  }

  function terms(query) {
    var all = [];
    words(Array.from(query).slice(0, MAX_QUERY).join("")).forEach(function (w) {
      if (all.indexOf(w[1]) < 0) all.push(w[1]);
    });
    var kept = all.filter(function (w) { return STOP_WORDS.indexOf(w) < 0; });
    return (kept.length ? kept : all).slice(0, MAX_TERMS);
  }

  function len(s) { return Array.from(s).length; }

  function matchClass(q, t) {
    if (q === t) return 3;
    var ql = len(q), tl = len(t);
    if (ql >= 2 && t.indexOf(q) === 0) return 2;
    if (tl >= 3 && ql <= tl + 2 && q.indexOf(t) === 0) return 1;
    return 0;
  }

  function best(q, ws) {
    var b = 0;
    for (var i = 0; i < ws.length; i++) b = Math.max(b, matchClass(q, ws[i][1]));
    return b;
  }

  function phrase(ws) {
    return " " + ws.map(function (w) { return w[1]; }).join(" ") + " ";
  }

  function score(p, ts) {
    var total = 0;
    for (var i = 0; i < ts.length; i++) {
      var q = ts[i], body = 0;
      for (var j = 0; j < p.body.length; j++) body += matchClass(q, p.body[j][1]);
      var s = 30 * best(q, p.heading) + 8 * best(q, p.title) + Math.min(body, 15);
      if (s === 0) return null;
      total += s;
    }
    if (ts.length > 1) {
      var wanted = " " + ts.join(" ") + " ";
      if (phrase(p.heading).indexOf(wanted) >= 0) total += 60;
      else if (phrase(p.body).indexOf(wanted) >= 0) total += 20;
    }
    return total;
  }

  function snippet(text, ts) {
    var chars = Array.from(text), ws = words(text), at = 0;
    for (var i = 0; i < ws.length; i++) {
      if (ts.some(function (q) { return matchClass(q, ws[i][1]) > 0; })) { at = ws[i][0]; break; }
    }
    var start = Math.max(0, at - LEAD);
    if (start > 0) {
      while (start < at && chars[start - 1] !== " ") start++;
    }
    var end = Math.min(start + SNIPPET, chars.length);
    if (end < chars.length) {
      var e = end;
      while (e > start + SNIPPET / 2 && chars[e] !== undefined && chars[e] !== " ") e--;
      end = e;
    }
    var body = chars.slice(start, end).join("").trim();
    return (start > 0 ? "…" : "") + body + (end < chars.length ? "…" : "");
  }

  // The words of every section, worked out once.
  var prepared = null;
  function prepare(index) {
    if (prepared && prepared.index === index) return prepared.list;
    var titles = index.pages.map(function (p) { return words(p.t); });
    var list = index.sections.map(function (s) {
      return { s: s, heading: words(s.h), title: titles[s.p] || [], body: words(s.x) };
    });
    prepared = { index: index, list: list };
    return list;
  }

  // The best `limit` sections for `query`, best first: {chapter, heading, url, snippet, score}.
  function search(index, query, limit) {
    var ts = terms(query);
    if (!ts.length || limit <= 0) return [];
    var scored = [];
    prepare(index).forEach(function (p, i) {
      var n = score(p, ts);
      if (n !== null) scored.push({ n: n, i: i, p: p });
    });
    scored.sort(function (a, b) { return b.n - a.n || a.i - b.i; });
    return scored.slice(0, limit).map(function (r) {
      var s = r.p.s, page = index.pages[s.p] || { f: "", t: "" };
      return {
        chapter: page.t,
        heading: s.h,
        url: s.l === 1 || !s.a ? page.f : page.f + "#" + s.a,
        snippet: snippet(s.x, ts),
        score: r.n,
        terms: ts
      };
    });
  }

  if (typeof module !== "undefined" && module.exports) {
    module.exports = { search: search, terms: terms, words: words };
  }
  if (typeof document === "undefined") return;

  // ---- In the book ----
  var LIMIT = 30;
  // A page of the book from the index (search.html sits at the book's root): `page.html` or
  // `page.html#anchor`, never a scheme or a path that climbs out.
  var PAGE = /^[a-z0-9-]+(\/[a-z0-9-]+)*\.html(#[\p{L}\p{N}_-]*)?$/u;

  function isTyping(el) {
    return el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);
  }

  // "/" anywhere in the book goes to the search field.
  document.addEventListener("keydown", function (e) {
    var q = document.getElementById("q");
    if (e.key === "/" && !e.ctrlKey && !e.metaKey && !e.altKey && q && !isTyping(document.activeElement)) {
      e.preventDefault();
      q.focus();
      q.select();
    }
  });

  if (document.body.getAttribute("data-page") !== "search") return;

  var input = document.getElementById("q");
  var status = document.getElementById("search-status");
  var results = document.getElementById("results");
  var index = window.ANICRAFT_GUIDE_INDEX;

  // The snippet with the matching words marked.
  function marked(text, ts) {
    var frag = document.createDocumentFragment(), chars = Array.from(text), at = 0;
    words(text).forEach(function (w) {
      if (!ts.some(function (q) { return matchClass(q, w[1]) > 0; })) return;
      var n = len(w[1]);
      frag.appendChild(document.createTextNode(chars.slice(at, w[0]).join("")));
      var m = document.createElement("mark");
      m.textContent = chars.slice(w[0], w[0] + n).join("");
      frag.appendChild(m);
      at = w[0] + n;
    });
    frag.appendChild(document.createTextNode(chars.slice(at).join("")));
    return frag;
  }

  function links() {
    return Array.prototype.slice.call(results.querySelectorAll("a"));
  }

  function run() {
    var query = input.value;
    while (results.firstChild) results.removeChild(results.firstChild);
    if (!index) {
      status.textContent = "The search index is missing. Use the chapter list instead.";
      return;
    }
    if (!terms(query).length) {
      status.textContent = "Type a word or two to search the guide.";
      return;
    }
    var hits = search(index, query, LIMIT);
    status.textContent = hits.length === 0
      ? "Nothing in the guide matches “" + query.trim() + "”."
      : (hits.length === 1 ? "1 result" : hits.length + (hits.length === LIMIT ? "+ results" : " results")) + " for “" + query.trim() + "”.";
    hits.forEach(function (h) {
      var li = document.createElement("li");
      if (!PAGE.test(h.url)) return;
      var a = document.createElement("a");
      a.setAttribute("href", h.url);
      var head = document.createElement("span");
      head.className = "hit-heading";
      head.textContent = h.heading;
      var chapter = document.createElement("span");
      chapter.className = "hit-chapter";
      chapter.textContent = h.chapter;
      a.appendChild(head);
      a.appendChild(chapter);
      li.appendChild(a);
      if (h.snippet) {
        var p = document.createElement("p");
        p.className = "hit-snippet";
        p.appendChild(marked(h.snippet, h.terms));
        li.appendChild(p);
      }
      results.appendChild(li);
    });
    try {
      history.replaceState(null, "", "?q=" + encodeURIComponent(query));
    } catch (_) {
      // Some browsers refuse to change a file:// address; the results stand all the same.
    }
  }

  input.addEventListener("input", run);
  input.addEventListener("keydown", function (e) {
    var first = links()[0];
    if (e.key === "ArrowDown" && first) {
      e.preventDefault();
      first.focus();
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (first) first.click();
    } else if (e.key === "Escape") {
      input.value = "";
      run();
    }
  });
  results.addEventListener("keydown", function (e) {
    var all = links(), i = all.indexOf(document.activeElement);
    if (i < 0) return;
    if (e.key === "ArrowDown" && i + 1 < all.length) {
      e.preventDefault();
      all[i + 1].focus();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (i === 0) input.focus();
      else all[i - 1].focus();
    } else if (e.key === "Escape") {
      e.preventDefault();
      input.focus();
    }
  });

  var params = new URLSearchParams(location.search);
  input.value = params.get("q") || "";
  run();
  input.focus();
})();
