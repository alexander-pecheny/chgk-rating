/* Small behaviours that degrade to nothing without JS: the remembered
   switches, the chart crosshair, pinned columns, the venue filter, the section
   picker, the phone's search, the phone's card rows and the copy of a formula. */

(function () {
  [[".prov-toggle input", "showprov"], [".roster-toggle input", "showroster"]].forEach(function (t) {
    var cb = document.querySelector(t[0]);
    if (!cb) return;
    try { cb.checked = localStorage.getItem(t[1]) === "1"; } catch (e) {}
    cb.addEventListener("change", function () {
      try { localStorage.setItem(t[1], cb.checked ? "1" : "0"); } catch (e) {}
    });
  });
})();

/* Each view carries its own points in viewBox coordinates, so switching between
   rating and place is a visibility swap and the crosshair is a nearest-x lookup
   in whichever view is showing — no scales to reconstruct. */
(function () {
  Array.prototype.forEach.call(document.querySelectorAll(".chart"), function (fig) {
    var svg = fig.querySelector("svg");
    var cursor = fig.querySelector(".c-cursor");
    var rule = fig.querySelector(".c-rule");
    var dot = fig.querySelector(".c-dot");
    var tip = fig.querySelector(".c-tip");
    var views = [].slice.call(fig.querySelectorAll(".c-view[data-points]"));
    var tabs = [].slice.call(fig.querySelectorAll(".c-tab"));
    if (!svg || !cursor || !tip || !views.length) return;

    var series = views.map(function (v) {
      try { return JSON.parse(v.dataset.points); } catch (e) { return []; }
    });
    var active = 0;
    var shown = -1;

    var tipDate = document.createElement("span");
    var tipValue = document.createElement("span");
    tipDate.className = "c-tip-date";
    tipValue.className = "c-tip-value";
    tip.appendChild(tipDate);
    tip.appendChild(tipValue);

    function hide() {
      shown = -1;
      cursor.setAttribute("hidden", "");
      tip.setAttribute("hidden", "");
    }

    tabs.forEach(function (tab, i) {
      tab.addEventListener("click", function () {
        active = i;
        views.forEach(function (v, j) {
          if (i === j) v.removeAttribute("hidden");
          else v.setAttribute("hidden", "");
        });
        tabs.forEach(function (t, j) {
          t.classList.toggle("on", i === j);
          t.setAttribute("aria-pressed", i === j);
        });
        hide();
      });
    });

    function nearest(x) {
      var pts = series[active];
      var lo = 0, hi = pts.length - 1;
      while (hi - lo > 1) {
        var mid = (lo + hi) >> 1;
        if (pts[mid][0] < x) lo = mid; else hi = mid;
      }
      return x - pts[lo][0] <= pts[hi][0] - x ? lo : hi;
    }

    function draw(i) {
      var p = series[active][i];
      rule.setAttribute("x1", p[0]);
      rule.setAttribute("x2", p[0]);
      dot.setAttribute("cx", p[0]);
      dot.setAttribute("cy", p[1]);
      tipDate.textContent = p[2];
      tipValue.textContent = p[3];

      var ctm = svg.getScreenCTM();
      if (!ctm) return;
      var q = svg.createSVGPoint();
      q.x = p[0];
      q.y = p[1];
      var at = q.matrixTransform(ctm);
      var box = fig.getBoundingClientRect();
      tip.style.left = at.x - box.left + fig.scrollLeft + "px";
      tip.style.top = at.y - box.top + fig.scrollTop + "px";
      tip.classList.toggle("flip", at.x - box.left > box.width - 120);
    }

    function show(ev) {
      if (series[active].length < 2) return;
      var ctm = svg.getScreenCTM();
      if (!ctm) return;
      var p = svg.createSVGPoint();
      p.x = ev.clientX;
      p.y = ev.clientY;
      var i = nearest(p.matrixTransform(ctm.inverse()).x);
      if (i !== shown) { shown = i; draw(i); }
      // `hidden` is an HTMLElement property; on an SVG <g> only the attribute
      // counts, so both are set through attributes.
      cursor.removeAttribute("hidden");
      tip.removeAttribute("hidden");
    }

    // A finger has no hover: a tap shows the point under it and a drag along
    // the line moves it; the crosshair stays until a tap elsewhere.
    svg.addEventListener("pointermove", show);
    svg.addEventListener("pointerdown", show);
    svg.addEventListener("pointerleave", function (ev) {
      if (ev.pointerType === "mouse") hide();
    });
    document.addEventListener("pointerdown", function (ev) {
      if (!fig.contains(ev.target)) hide();
    });
  });
})();

/* Wide tables: pin the name column (and the narrow place column in front of
   it) while the frame scrolls sideways, and mark the frame once it has so the
   edge shows a shadow. Measured, so it follows whatever the table holds. */
(function () {
  var frames = [];

  function setup(table) {
    if (!table || !table.tHead || !table.tHead.rows.length) return;
    var heads = [].slice.call(table.tHead.rows[0].cells);
    var k = -1;
    heads.forEach(function (th, i) {
      if (k < 0 && th.classList.contains("pin-name")) k = i;
    });
    if (k < 0) return;
    // The place column rides along when the table opens with one.
    var cols = k > 0 && heads[0].classList.contains("results-num") ? [0, k] : [k];
    var width = heads.length;
    var rows = [].slice.call(table.rows).filter(function (r) { return r.cells.length === width; });
    rows.forEach(function (r) {
      cols.forEach(function (i) { r.cells[i].classList.add("pinned"); });
      r.cells[k].classList.add("pin-last");
    });
    frames.push({ rows: rows, cols: cols, heads: heads });
  }

  function layout() {
    if (window.matchMedia("(max-width: 760px)").matches) return;
    var lefts = frames.map(function (f) {
      var left = 0;
      return f.cols.map(function (i) {
        var x = left;
        left += f.heads[i].getBoundingClientRect().width;
        return x + "px";
      });
    });
    frames.forEach(function (f, k) {
      f.cols.forEach(function (i, j) {
        f.rows.forEach(function (r) { r.cells[i].style.left = lefts[k][j]; });
      });
    });
  }

  [].forEach.call(document.querySelectorAll("table.rating-table"), setup);
  layout();
  window.addEventListener("resize", layout);
  var frame = document.querySelector(".sheet-frame");
  if (frame) {
    frame.addEventListener("scroll", function () {
      frame.classList.toggle("scrolled", frame.scrollLeft > 0);
    }, { passive: true });
  }
  // Opening a lineup changes the cell widths the pins are measured from.
  document.addEventListener("change", function (ev) {
    if (ev.target.closest && ev.target.closest(".roster-open, .roster-toggle")) layout();
  });
})();

/* A package played in several cities is one field. The page lists every venue;
   the filter keeps one and swaps each place for the place at that venue, which
   the cell carries in data-own. The choice rides in ?venue= so it can be shared. */
(function () {
  var sel = document.querySelector(".venue-filter select");
  if (!sel) return;
  var table = sel.closest(".section").querySelector(".results-table");
  if (!table) return;
  var rows = [].slice.call(table.querySelectorAll("tbody tr"));
  var cells = [].slice.call(table.querySelectorAll("td[data-own]"));
  cells.forEach(function (td) {
    td.dataset.all = td.textContent;
    td.dataset.allClass = td.className;
  });
  function apply() {
    var v = sel.value;
    rows.forEach(function (tr) {
      tr.hidden = v !== "" && tr.dataset.venue !== v;
    });
    cells.forEach(function (td) {
      td.textContent = v ? td.dataset.own : td.dataset.all;
      td.className = v ? (td.dataset.ownClass || td.dataset.allClass) : td.dataset.allClass;
    });
  }
  var m = /[?&]venue=(\d+)/.exec(location.search);
  if (m && sel.querySelector('option[value="' + m[1] + '"]')) sel.value = m[1];
  apply();
  sel.addEventListener("change", function () {
    apply();
    var u = new URL(location.href);
    if (sel.value) u.searchParams.set("venue", sel.value);
    else u.searchParams.delete("venue");
    try { history.replaceState(null, "", u); } catch (e) {}
  });
})();

/* The section strip shows every tab when they fit and only the picker, which
   drops the list of sections, when they do not: dope's fitTabBar. */
(function () {
  var strip = document.querySelector(".site-tabs");
  if (!strip) return;
  var picker = strip.querySelector(".tab-picker");
  var panel = strip.querySelector(".tab-picker-panel");
  function fit() {
    strip.classList.remove("tabs-collapsed");
    strip.classList.toggle("tabs-collapsed", strip.scrollWidth > strip.clientWidth + 1);
  }
  function open(on) {
    panel.hidden = !on;
    picker.setAttribute("aria-expanded", on ? "true" : "false");
    if (!on) return;
    var r = picker.getBoundingClientRect();
    panel.style.position = "fixed";
    panel.style.left = Math.max(8, Math.min(r.left, window.innerWidth - panel.offsetWidth - 8)) + "px";
    panel.style.top = r.bottom + 2 + "px";
    panel.style.right = "auto";
  }
  picker.addEventListener("pointerdown", function (ev) {
    if (ev.button !== 0) return;
    ev.preventDefault();
    open(panel.hidden);
  });
  picker.addEventListener("click", function (ev) {
    // A keyboard's Enter or Space arrives as a click with no press before it.
    if (ev.detail === 0) open(panel.hidden);
  });
  document.addEventListener("pointerdown", function (ev) {
    if (!panel.hidden && !strip.contains(ev.target)) open(false);
  });
  document.addEventListener("keydown", function (ev) {
    if (ev.key === "Escape" && !panel.hidden) { open(false); picker.focus(); }
  });
  fit();
  window.addEventListener("resize", function () { open(false); fit(); });
})();

/* On a phone the bar shows the search as a button; a tap opens the form over
   the whole bar, and closing it brings the bar back. */
(function () {
  var bar = document.querySelector(".game-host-top");
  var form = bar && bar.querySelector(".site-search");
  if (!form) return;
  var input = form.querySelector("input");
  form.querySelector(".search-open").addEventListener("click", function () {
    bar.classList.add("searching");
    input.focus();
  });
  function close() { bar.classList.remove("searching"); }
  form.querySelector(".search-close").addEventListener("click", close);
  input.addEventListener("keydown", function (ev) { if (ev.key === "Escape") close(); });
})();

/* A phone shows every results table as cards: a row is its lead number, its
   name with the line under it, and its main value; the other columns form a
   panel of «label: value» lines that a tap on the card opens. The roles come
   from the header: numbers before the name lead, words before it go under it,
   the bold column (else the first number after the name) is the value. */
(function () {
  var phone = window.matchMedia("(max-width: 760px)");
  var tables = [].slice.call(document.querySelectorAll("table.rating-table"));

  function roles(table) {
    var heads = [].slice.call(table.tHead ? table.tHead.rows[0].cells : []);
    var k = -1;
    heads.forEach(function (th, i) { if (k < 0 && th.classList.contains("pin-name")) k = i; });
    if (k < 0) return null;
    var out = heads.map(function () { return "panel"; });
    out[k] = "title";
    var lead = false;
    for (var i = 0; i < k; i++) {
      var num = heads[i].classList.contains("results-num");
      out[i] = num && !lead ? "lead" : "under";
      if (num && !lead) lead = true;
    }
    var body = table.tBodies[0];
    var bold = -1;
    for (var j = k + 1; j < heads.length && body; j++) {
      if (body.querySelector("td:nth-child(" + (j + 1) + ").rating-strong")) { bold = j; break; }
    }
    if (bold < 0) {
      for (j = k + 1; j < heads.length; j++) {
        if (heads[j].classList.contains("results-num")) { bold = j; break; }
      }
    }
    if (bold >= 0) out[bold] = "value";
    return { roles: out, heads: heads, k: k };
  }

  // A panel line is labelled by its column's name, or by the explanation
  // of a one-letter name (±, θ, v), which is too short to read on its own.
  function label(th) {
    var text = th.textContent.trim();
    return text.length <= 1 && th.title ? th.title : text;
  }

  function setup(table) {
    var r = roles(table);
    if (!r) return;
    table.classList.add("cards");
    [].forEach.call(table.tBodies, function (tb) {
      [].forEach.call(tb.rows, function (tr) {
        if (tr.cells.length !== r.heads.length) return;
        var under = [];
        [].forEach.call(tr.cells, function (td, i) {
          td.classList.add("card-" + r.roles[i]);
          if (r.roles[i] === "panel") td.dataset.label = label(r.heads[i]);
          if (r.roles[i] === "under" && td.textContent.trim()) under.push(td.textContent.trim());
        });
        if (under.length) {
          var line = document.createElement("span");
          line.className = "card-only results-team-city";
          line.textContent = under.join(" · ");
          tr.cells[r.k].appendChild(line);
        }
        if (tr.querySelector(".card-panel")) tr.classList.add("card-has-panel");
      });
    });
  }
  tables.forEach(setup);

  document.addEventListener("click", function (ev) {
    if (!phone.matches) return;
    var tr = ev.target.closest && ev.target.closest("table.cards tr.card-has-panel");
    if (!tr || ev.target.closest("a, input, label, select, button")) return;
    tr.classList.toggle("card-open");
  });
})();

/* A formula on /method/ is an SVG with its TeX and its MathML hidden behind
   it. The browser's own copy breaks the line at each MathML, so a copy that
   takes a formula takes its TeX instead: the selection, without the drawings
   and the MathML, read as the page lays it out. */
(function () {
  document.addEventListener("copy", function (e) {
    var sel = window.getSelection();
    if (!sel.rangeCount || !e.clipboardData) return;
    var frag = sel.getRangeAt(0).cloneContents();
    if (!frag.querySelector(".tex-src")) return;
    Array.prototype.forEach.call(frag.querySelectorAll("svg, .tex-mml"), function (n) { n.remove(); });
    var box = document.createElement("div");
    box.className = "post tex-copy";
    box.appendChild(frag);
    document.body.appendChild(box);
    e.clipboardData.setData("text/plain", box.innerText.trim());
    document.body.removeChild(box);
    e.preventDefault();
  });
})();
