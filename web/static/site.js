/* Two small behaviours: the provisional switch on the listings, and the hover
   crosshair on every history chart. Both degrade to nothing without JS. */

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

    svg.addEventListener("pointermove", function (ev) {
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
    });

    svg.addEventListener("pointerleave", hide);
  });
})();
