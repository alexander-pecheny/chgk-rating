use std::fmt::Write as _;

pub fn e(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Published numbers live on the 2019 display scale and are whole.
pub fn disp(v: f64) -> String {
    format!("{v:.0}")
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

pub fn front_matter(title: &str, body: &str) -> String {
    let t = title.replace('\\', "\\\\").replace('"', "\\\"");
    format!("+++\ntitle = \"{t}\"\n+++\n{body}\n")
}

/// A pager strip; `n` is the current page, `url` maps a page number to a href.
pub fn pager(n: usize, total: usize, url: &dyn Fn(usize) -> String) -> String {
    if total < 2 {
        return String::new();
    }
    let mut s = String::from("<nav class=\"pager\" aria-label=\"Страницы\">");
    for i in 1..=total {
        if i == n {
            let _ = write!(s, "<span class=\"pager-cur\" aria-current=\"page\">{i}</span>");
        } else {
            let _ = write!(s, "<a href=\"{}\">{i}</a>", url(i));
        }
    }
    s.push_str("</nav>");
    s
}

/// A line with an uncertainty band. Points are (x, middle, sigma) already in
/// display units; `ticks` are (x, label) for the horizontal axis.
pub fn chart(points: &[(f64, f64, f64)], ticks: &[(f64, String)], label: &str) -> String {
    if points.len() < 2 {
        return format!("<p class=\"empty\">{}</p>", e("Точек для графика пока нет."));
    }
    const W: f64 = 760.0;
    const H: f64 = 210.0;
    const L: f64 = 46.0;
    const R: f64 = 10.0;
    const T: f64 = 10.0;
    const B: f64 = 24.0;

    let (x0, x1) = (points[0].0, points[points.len() - 1].0);
    let xs = if x1 > x0 { x1 - x0 } else { 1.0 };
    let mut lo = f64::MAX;
    let mut hi = f64::MIN;
    for &(_, m, s) in points {
        lo = lo.min(m - s);
        hi = hi.max(m + s);
    }
    if !(hi > lo) {
        hi = lo + 1.0;
    }
    let pad = (hi - lo) * 0.08;
    let (lo, hi) = (lo - pad, hi + pad);

    let px = |x: f64| L + (x - x0) / xs * (W - L - R);
    let py = |y: f64| T + (hi - y) / (hi - lo) * (H - T - B);

    let mut band = String::new();
    for &(x, m, s) in points {
        let _ = write!(band, "{:.1},{:.1} ", px(x), py(m + s));
    }
    for &(x, m, s) in points.iter().rev() {
        let _ = write!(band, "{:.1},{:.1} ", px(x), py(m - s));
    }
    let mut line = String::new();
    for &(x, m, _) in points {
        let _ = write!(line, "{:.1},{:.1} ", px(x), py(m));
    }

    let mut grid = String::new();
    let step = nice_step(hi - lo, 4);
    let mut v = (lo / step).ceil() * step;
    while v <= hi {
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
        v += step;
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
        "<figure class=\"chart scroll-fade-xy\"><svg viewBox=\"0 0 {W:.0} {H:.0}\" role=\"img\" \
         aria-label=\"{}\">{grid}\
         <polygon points=\"{}\" class=\"c-band\"/>\
         <polyline points=\"{}\" class=\"c-line\"/>{xa}</svg></figure>",
        e(label),
        band.trim_end(),
        line.trim_end()
    )
}

/// Thin out a series to at most `max` points, always keeping the last one.
pub fn thin<T: Copy>(v: &[T], max: usize) -> Vec<T> {
    if v.len() <= max {
        return v.to_vec();
    }
    let step = (v.len() + max - 1) / max;
    let mut out: Vec<T> = v.iter().step_by(step).copied().collect();
    if let Some(last) = v.last() {
        out.push(*last);
    }
    out
}
