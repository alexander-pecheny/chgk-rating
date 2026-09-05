use std::fmt::Write as _;

pub fn e(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Published numbers live on the 2019 display scale and are whole. A value that
/// rounds to zero from below prints "-0" otherwise, which reads as a number.
pub fn disp(v: f64) -> String {
    let r = v.round();
    format!("{:.0}", if r == 0.0 { 0.0 } else { r })
}

pub fn signed(v: f64) -> String {
    let r = v.round();
    if r > 0.0 {
        format!("+{}", disp(r))
    } else if r < 0.0 {
        format!("−{}", disp(-r))
    } else {
        "0".into()
    }
}

/// A step from the 1 / 2 / 2.5 / 5 ladder, so axis labels are round numbers.
fn nice_step(span: f64, target: usize) -> f64 {
    let raw = span / target as f64;
    if !(raw > 0.0) || !raw.is_finite() {
        return 1.0;
    }
    let mag = 10f64.powf(raw.log10().floor());
    let n = raw / mag;
    let s = if n <= 1.0 {
        1.0
    } else if n <= 2.0 {
        2.0
    } else if n <= 2.5 {
        2.5
    } else if n <= 5.0 {
        5.0
    } else {
        10.0
    };
    s * mag
}

pub fn place(v: Option<f64>) -> String {
    match v {
        None => "—".into(),
        Some(p) if (p - p.round()).abs() < 1e-6 => format!("{}", p.round() as i64),
        Some(p) => format!("{p:.1}"),
    }
}

/// An empty cell rather than a dash: a metric the run did not produce is not a
/// value the reader should try to read.
pub fn blank(v: Option<f64>, digits: usize) -> String {
    v.map(|x| format!("{x:.digits$}")).unwrap_or_default()
}

pub fn opt(v: Option<f64>, digits: usize) -> String {
    match v {
        None => "—".into(),
        Some(x) => format!("{x:.digits$}"),
    }
}

/// A pager strip; `n` is the current page, `url` maps a page number to a href.
pub fn pager(n: usize, total: usize, url: &dyn Fn(usize) -> String) -> String {
    if total < 2 {
        return String::new();
    }
    let mut s = String::from("<nav class=\"pager\" aria-label=\"Страницы\">");
    let mut last = 0;
    for i in 1..=total {
        // The listings run to dozens of pages; show the ends, the neighbourhood
        // of the current page, and an ellipsis for the stretch between.
        let near = i <= 2 || i + 2 > total || (i as i64 - n as i64).abs() <= 2;
        if !near {
            continue;
        }
        if i > last + 1 {
            s.push_str("<span class=\"pager-gap\">…</span>");
        }
        if i == n {
            let _ = write!(s, "<span class=\"pager-cur\" aria-current=\"page\">{i}</span>");
        } else {
            let _ = write!(s, "<a href=\"{}\">{i}</a>", url(i));
        }
        last = i;
    }
    s.push_str("</nav>");
    s
}

/// One point of a history: x on the release axis, the value and its uncertainty
/// in the units of its own series, and what the tooltip says about it.
#[derive(Clone)]
pub struct Pt {
    pub x: f64,
    pub mid: f64,
    pub sigma: f64,
    pub date: String,
    pub text: String,
}

/// One thing the chart can show. Rating and place have different units and run
/// in opposite directions, so each carries its own scale and its own geometry.
pub struct Series {
    pub name: String,
    pub pts: Vec<Pt>,
    /// Place: 1 belongs at the top.
    pub invert: bool,
    pub band: bool,
}

/// A line inside its uncertainty band, with a hover crosshair and — when more
/// than one series is given — a switch between them. The SVG carries the shape;
/// each view's `data-points` carries the same points in viewBox coordinates,
/// which is all the crosshair in `site.js` needs.
pub fn chart(series: &[Series], ticks: &[(f64, String)], label: &str) -> String {
    let usable: Vec<&Series> = series.iter().filter(|s| s.pts.len() >= 2).collect();
    let Some(first) = usable.first() else {
        return format!("<p class=\"empty\">{}</p>", e("Точек для графика пока нет."));
    };
    const W: f64 = 760.0;
    const H: f64 = 210.0;
    const L: f64 = 46.0;
    const R: f64 = 10.0;
    const T: f64 = 10.0;
    const B: f64 = 24.0;

    let (x0, x1) = (first.pts[0].x, first.pts[first.pts.len() - 1].x);
    let xs = if x1 > x0 { x1 - x0 } else { 1.0 };
    let px = |x: f64| L + (x - x0) / xs * (W - L - R);

    let mut views = String::new();
    let mut tabs = String::new();
    for (i, sr) in usable.iter().enumerate() {
        let mut lo = f64::MAX;
        let mut hi = f64::MIN;
        for p in &sr.pts {
            let d = if sr.band { p.sigma } else { 0.0 };
            lo = lo.min(p.mid - d);
            hi = hi.max(p.mid + d);
        }
        if !(hi > lo) {
            hi = lo + 1.0;
        }
        let pad = (hi - lo) * 0.08;
        let (lo, hi) = (lo - pad, hi + pad);
        // Place counts downwards, so the axis does too.
        let py = |y: f64| {
            let f = if sr.invert { (y - lo) / (hi - lo) } else { (hi - y) / (hi - lo) };
            T + f * (H - T - B)
        };

        let mut grid = String::new();
        let step = nice_step(hi - lo, 4);
        let mut v = (lo / step).ceil() * step;
        while v <= hi {
            if !sr.invert || v >= 1.0 {
                let y = py(v);
                let _ = write!(
                    grid,
                    "<line x1=\"{L}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\" class=\"c-grid\"/>\
                     <text x=\"{:.1}\" y=\"{:.1}\" class=\"c-axis\" text-anchor=\"end\">{}</text>",
                    W - R,
                    L - 6.0,
                    y + 4.0,
                    e(&disp(v))
                );
            }
            v += step;
        }

        // A team or player leaves the table when it stops playing and comes back
        // later; joining those two releases with a straight line would draw a
        // rating for weeks that never had one. Break the run instead.
        let gap = {
            let mut d: Vec<f64> = sr.pts.windows(2).map(|w| w[1].x - w[0].x).collect();
            d.sort_by(f64::total_cmp);
            let med = d.get(d.len() / 2).copied().unwrap_or(1.0);
            (med * 3.0).max(4.0)
        };
        let runs: Vec<&[Pt]> = {
            let mut out = Vec::new();
            let mut start = 0;
            for i in 1..sr.pts.len() {
                if sr.pts[i].x - sr.pts[i - 1].x > gap {
                    out.push(&sr.pts[start..i]);
                    start = i;
                }
            }
            out.push(&sr.pts[start..]);
            out
        };

        let mut band = String::new();
        let mut line = String::new();
        for run in &runs {
            if sr.band && run.len() > 1 {
                let mut poly = String::new();
                for p in run.iter() {
                    let _ = write!(poly, "{:.1},{:.1} ", px(p.x), py(p.mid + p.sigma));
                }
                for p in run.iter().rev() {
                    let _ = write!(poly, "{:.1},{:.1} ", px(p.x), py(p.mid - p.sigma));
                }
                let _ = write!(band, "<polygon points=\"{}\" class=\"c-band\"/>", poly.trim_end());
            }
            let mut pl = String::new();
            for p in run.iter() {
                let _ = write!(pl, "{:.1},{:.1} ", px(p.x), py(p.mid));
            }
            let _ = write!(line, "<polyline points=\"{}\" class=\"c-line\"/>", pl.trim_end());
        }

        let data: Vec<serde_json::Value> = sr
            .pts
            .iter()
            .map(|p| {
                serde_json::json!([
                    (px(p.x) * 10.0).round() / 10.0,
                    (py(p.mid) * 10.0).round() / 10.0,
                    p.date,
                    p.text
                ])
            })
            .collect();

        let _ = write!(
            views,
            "<g class=\"c-view\" data-points=\"{}\"{}>{grid}{band}{line}</g>",
            e(&serde_json::to_string(&data).unwrap()),
            if i == 0 { "" } else { " hidden" },
        );
        if usable.len() > 1 {
            let _ = write!(
                tabs,
                "<button type=\"button\" class=\"c-tab{}\" aria-pressed=\"{}\">{}</button>",
                if i == 0 { " on" } else { "" },
                i == 0,
                e(&sr.name)
            );
        }
    }

    let mut xa = String::new();
    for (i, (x, t)) in ticks.iter().enumerate() {
        let anchor = if i == 0 {
            "start"
        } else if i + 1 == ticks.len() {
            "end"
        } else {
            "middle"
        };
        let _ = write!(
            xa,
            "<text x=\"{:.1}\" y=\"{:.1}\" class=\"c-axis\" text-anchor=\"{anchor}\">{}</text>",
            px(*x),
            H - 6.0,
            e(t)
        );
    }

    format!(
        "<figure class=\"chart scroll-fade-xy\">{}\
         <svg viewBox=\"0 0 {W:.0} {H:.0}\" role=\"img\" aria-label=\"{}\">{views}{xa}\
         <g class=\"c-cursor\" aria-hidden=\"true\" hidden>\
         <line class=\"c-rule\" y1=\"{T}\" y2=\"{:.1}\"/><circle class=\"c-dot\" r=\"3.5\"/></g>\
         </svg><div class=\"c-tip\" hidden></div></figure>",
        if tabs.is_empty() {
            String::new()
        } else {
            format!("<div class=\"c-tabs\" role=\"group\">{tabs}</div>")
        },
        e(label),
        H - B
    )
}

/// Thin out a series to at most `max` points, always keeping the last one.
pub fn thin<T: Clone>(v: &[T], max: usize) -> Vec<T> {
    if v.len() <= max {
        return v.to_vec();
    }
    let step = (v.len() + max - 1) / max;
    let mut out: Vec<T> = v.iter().step_by(step).cloned().collect();
    if let Some(last) = v.last() {
        out.push(last.clone());
    }
    out
}
