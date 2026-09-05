/* Two small behaviours: the provisional switch on the listings, and the hover
   crosshair on every rating chart. Both degrade to nothing without JS. */

(function () {
  var cb = document.querySelector(".prov-toggle input");
  if (!cb) return;
  try { cb.checked = localStorage.getItem("showprov") === "1"; } catch (e) {}
  cb.addEventListener("change", function () {
    try { localStorage.setItem("showprov", cb.checked ? "1" : "0"); } catch (e) {}
  });
})();

/* The chart carries its own points in viewBox coordinates, so the crosshair is
   a nearest-x lookup and two attribute writes — no scales to reconstruct. */
(function () {
  Array.prototype.forEach.call(document.querySelectorAll(".chart[data-points]"), function (fig) {
    var pts;
    try { pts = JSON.parse(fig.dataset.points); } catch (e) { return; }
    if (!pts || pts.length < 2) return;

    var svg = fig.querySelector("svg");
    var cursor = fig.querySelector(".c-cursor");
    var rule = fig.querySelector(".c-rule");
    var dot = fig.querySelector(".c-dot");
    var tip = fig.querySelector(".c-tip");
    if (!svg || !cursor || !tip) return;
    var shown = -1;
    var tipDate = document.createElement("span");
    var tipValue = document.createElement("span");
    tipDate.className = "c-tip-date";
    tipValue.className = "c-tip-value";
    tip.appendChild(tipDate);
    tip.appendChild(tipValue);

    function nearest(x) {
      var lo = 0, hi = pts.length - 1;
      while (hi - lo > 1) {
        var mid = (lo + hi) >> 1;
        if (pts[mid][0] < x) lo = mid; else hi = mid;
      }
      return x - pts[lo][0] <= pts[hi][0] - x ? lo : hi;
    }

    function toClient(x, y) {
      var ctm = svg.getScreenCTM();
      if (!ctm) return null;
      var p = svg.createSVGPoint();
      p.x = x; p.y = y;
      return p.matrixTransform(ctm);
    }

    function draw(i) {
      var p = pts[i];
      rule.setAttribute("x1", p[0]);
      rule.setAttribute("x2", p[0]);
      dot.setAttribute("cx", p[0]);
      dot.setAttribute("cy", p[1]);
      tipDate.textContent = p[2];
      tipValue.textContent = p[3] + " ± " + p[4];

      var at = toClient(p[0], p[1]);
      if (!at) return;
      var box = fig.getBoundingClientRect();
      var x = at.x - box.left + fig.scrollLeft;
      tip.style.left = x + "px";
      tip.style.top = at.y - box.top + fig.scrollTop + "px";
      tip.classList.toggle("flip", at.x - box.left > box.width - 120);
    }

    fig.addEventListener("pointermove", function (ev) {
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

    fig.addEventListener("pointerleave", function () {
      shown = -1;
      cursor.setAttribute("hidden", "");
      tip.setAttribute("hidden", "");
    });
  });
})();
