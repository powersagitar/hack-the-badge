"""Measures the badge photo's features for badge-geometry.ts (Task 11c).

Run through badge-photo.py with `--geometry`; this module is loaded by it
and works on the levelled scan, so every value is measured at the scan's
resolution and mapped into the photo by the same crop and scale as the
image. Percentages are of the photo's width (x) and height (y).

The search windows start from the previous photo's values (the old
badge-geometry.ts); every printed value is re-measured in them.
"""

import math
from collections import deque

import numpy as np

# The previous photo's centres, in percent (search windows only).
OLD = {
    "LED": [(10.3, 5.58), (85.13, 5.45), (17.08, 52.44), (79.53, 51.85), (17.27, 91.19), (77.86, 91.19)],
    "BUTTONS": {
        "UP": (24.44, 62.06),
        "LEFT": (17.57, 67.11),
        "RIGHT": (31.41, 67.11),
        "DOWN": (24.74, 72.36),
        "A": (84.34, 63.44),
        "B": (69.12, 68.2),
        "HOME": (44.18, 81.28),
        "START": (59.2, 81.28),
    },
    "AUX1": (4.41, 37.28),
    "POWER": (34.1, 1.5),
    "CHIP": (41.4, 62.1),
}
OLD_W, OLD_H = 974, 1513

# The design's SCREEN relation, from Task 11b's fit on the previous photo:
# its 4:3 rectangle (442.68 px wide) on a glass 438.28 px wide, centred.
SCREEN_OVER_GLASS = 442.68 / 438.28


def label(mask):
    """Yields the 4-connected components of mask, each as a list of (y, x)."""
    h, w = mask.shape
    seen = np.zeros_like(mask)
    for y0, x0 in zip(*np.nonzero(mask)):
        if seen[y0, x0]:
            continue
        q = deque([(y0, x0)])
        seen[y0, x0] = True
        pix = []
        while q:
            y, x = q.popleft()
            pix.append((y, x))
            for dy, dx in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                ny, nx = y + dy, x + dx
                if 0 <= ny < h and 0 <= nx < w and mask[ny, nx] and not seen[ny, nx]:
                    seen[ny, nx] = True
                    q.append((ny, nx))
        yield pix


def component(mask, origin, near, area, max_dist=200, core=False):
    """The component of mask (a window at origin) with area in range, nearest to near."""
    best, best_d = None, None
    for pix in label(mask):
        if not area[0] <= len(pix) <= area[1]:
            continue
        ys = np.array([p[0] for p in pix]) + origin[1]
        xs = np.array([p[1] for p in pix]) + origin[0]
        box = (xs.min(), ys.min(), xs.max() + 1, ys.max() + 1)
        if core:
            # The rows and columns holding at least half the busiest one's
            # pixels: a switch's body without its tabs, legs or shadow.
            cols = np.bincount(xs - box[0])
            rows = np.bincount(ys - box[1])
            cx_ = np.nonzero(cols >= cols.max() / 2)[0]
            cy_ = np.nonzero(rows >= rows.max() / 2)[0]
            box = (box[0] + cx_.min(), box[1] + cy_.min(), box[0] + cx_.max() + 1, box[1] + cy_.max() + 1)
        c = ((box[0] + box[2]) / 2, (box[1] + box[3]) / 2)
        d = math.hypot(c[0] - near[0], c[1] - near[1])
        if d <= max_dist and (best_d is None or d < best_d):
            best, best_d = dict(box=box, centre=c, area=len(pix)), d
    return best


class Ctx:
    def __init__(self, m, scale, height):
        a = np.asarray(m["lev"]).astype(np.float32)
        self.R, self.G, self.B = a[..., 0], a[..., 1], a[..., 2]
        self.lum = 0.299 * self.R + 0.587 * self.G + 0.114 * self.B
        self.S = self.B - self.G
        self.L, self.T = m["box"][0], m["box"][1]
        self.scale = scale
        self.w, self.h = 974, height

    def scan_of_old(self, px, py):
        return (self.L + px / 100 * OLD_W / self.scale, self.T + py / 100 * OLD_H / self.scale)

    def photo(self, x, y):
        return ((x - self.L) * self.scale, (y - self.T) * self.scale)

    def pct(self, x, y):
        X, Y = self.photo(x, y)
        return (100 * X / self.w, 100 * Y / self.h)

    def window(self, c, r):
        x0, y0 = int(c[0] - r), int(c[1] - r)
        return (x0, y0), slice(y0, y0 + 2 * r), slice(x0, x0 + 2 * r)


def fit(t, v):
    t, v = np.asarray(t, float), np.asarray(v, float)
    keep = np.ones(len(t), bool)
    for _ in range(5):
        p = np.polyfit(t[keep], v[keep], 1)
        r = v - np.polyval(p, t)
        keep = np.abs(r) < max(2.5 * 1.4826 * np.median(np.abs(r[keep])), 0.3)
    return p


def cross(prof, thr, lo, step=1):
    i = 0 if step > 0 else len(prof) - 1
    while 0 <= i + step < len(prof):
        a, b = prof[i], prof[i + step]
        if (a - thr) * (b - thr) <= 0 and a != b:
            return lo + i + step * (thr - a) / (b - a)
        i += step
    return None


def cw_deg(edge, p):
    """An edge line's angle, clockwise on screen positive (CSS rotate's sense)."""
    a = math.degrees(math.atan(p[0]))
    return a if edge in ("top", "bottom") else -a


def line_peak(prof, start, step, span=14):
    """Walks from start (in the module's bright frame or rim) by step until lum < 30,
    then returns the sub-pixel peak of the next `span` px: the polarizer's line."""
    i = start
    while 0 <= i < len(prof) and prof[i] >= 30:
        i += step
    idx = [i + step * k for k in range(span) if 0 <= i + step * k < len(prof)]
    vals = np.array([prof[k] for k in idx])
    j = int(np.argmax(vals))
    pos = float(idx[j])
    if 0 < j < len(vals) - 1:
        a, b, c = vals[j - 1], vals[j], vals[j + 1]
        den = a - 2 * b + c
        if den != 0:
            pos += step * 0.5 * (a - c) / den
    return pos


def glass(c):
    """The LCD's flat glass: inside the polarizer's thin line (lum about 35-65 on 12).

    Each side's line is the first peak after the module's frame (and, at
    the bottom, its grey rim) gives way to the glass's black.
    """
    pts = {"left": [], "right": [], "top": [], "bottom": []}
    for y in range(1300, 2100, 2):
        pts["left"].append((y, line_peak(c.lum[y], 655, 1)))
        # The right side has no rim: dark (2-7) beyond the line, then the
        # module's blue edge; the line is the window's peak.
        pts["right"].append((y, line_peak(c.lum[y], 2106, -1)))
    for x in range(1000, 1900, 2):
        pts["top"].append((x, line_peak(c.lum[:, x], 1135, 1)))
        pts["bottom"].append((x, line_peak(c.lum[:, x], 2266, -1)))
    return {k: fit([q[0] for q in v], [q[1] for q in v]) for k, v in pts.items()}


def htn_box(c):
    """The "Hack the North" box: the purple box's edges against its lilac surround (lum 150)."""
    pts = {"left": [], "right": [], "top": [], "bottom": []}
    for y in range(2420, 2580, 2):
        v = cross(c.lum[y, 960:1060], 150, 960)
        if v is not None:
            pts["left"].append((y, v))
        v = cross(c.lum[y, 2570:2670], 150, 2570, -1)
        if v is not None:
            pts["right"].append((y, v))
    for x in list(range(1100, 1400, 2)) + list(range(2200, 2550, 2)):
        v = cross(c.lum[2340:2420, x], 150, 2340, -1)
        if v is not None:
            pts["top"].append((x, v))
        v = cross(c.lum[2580:2650, x], 150, 2580)
        if v is not None:
            pts["bottom"].append((x, v))
    return {k: fit([q[0] for q in v], [q[1] for q in v]) for k, v in pts.items()}


def report(m, scale, height):
    c = Ctx(m, scale, height)
    found = {}
    print("\n-- geometry (percent of the photo; x of width, y of height) --")
    print(f"PHOTO = {{ w: {c.w}, h: {c.h} }}")

    # SCREEN.
    g = glass(c)
    angs = {k: cw_deg(k, p) for k, p in g.items()}
    print("glass edges (deg, clockwise +):", ", ".join(f"{k} {v:+.4f}" for k, v in angs.items()))
    rot = float(np.mean(list(angs.values())))
    print(f"glass angle vs the levelled board: {rot:+.4f} deg")
    h_ = htn_box(c)
    hang = {k: cw_deg(k, p) for k, p in h_.items()}
    print("HtN box edges (deg, clockwise +):", ", ".join(f"{k} {v:+.4f}" for k, v in hang.items()))
    print(f"HtN box angle: {np.mean(list(hang.values())):+.4f} deg")
    yc = (1300 + 2100) / 2
    xc = (1000 + 1900) / 2
    left, right = np.polyval(g["left"], yc), np.polyval(g["right"], yc)
    top, bottom = np.polyval(g["top"], xc), np.polyval(g["bottom"], xc)
    cos = math.cos(math.radians(rot))
    gw, gh = (right - left) * cos * scale, (bottom - top) * cos * scale
    cx, cy = c.photo((left + right) / 2, (top + bottom) / 2)
    print(f"glass: centre {cx:.2f}, {cy:.2f} photo px; {gw:.2f} x {gh:.2f} (its own frame), aspect {gw / gh:.4f}")
    sw = gw * SCREEN_OVER_GLASS
    sh = sw * 3 / 4
    print(f"screen rect: {sw:.2f} x {sh:.2f} photo px; insets left/right {(gw - sw) / 2:+.2f}, top/bottom {(gh - sh) / 2:+.2f}")
    print(
        f"SCREEN = {{ left: {100 * (cx - sw / 2) / c.w:.2f}, top: {100 * (cy - sh / 2) / c.h:.2f}, "
        f"width: {100 * sw / c.w:.2f}, height: {100 * sh / c.h:.2f}"
        + (f", rotateDeg: {rot:.2f} }}" if abs(rot) > 0.1 else " }")
    )

    # HOLES.
    for i, hole in enumerate(m["holes"][:2]):
        print(f"HOLES[{i}] = [{c.pct(hole['x'], hole['y'])[0]:.2f}, {c.pct(hole['x'], hole['y'])[1]:.2f}]  (r {hole['r'] * scale:.2f} px)")

    # POWER: the slide switch's dark body.
    near = c.scan_of_old(*OLD["POWER"])
    near = (near[0], near[1] + 70)
    o, ys, xs = c.window(near, 220)
    body = component(dark_body(c, ys, xs), o, near, (8000, 80000), core=True)
    print_box("POWER body", c, body)
    found["POWER"] = body

    # AUX1: the side slide switch's dark body.
    near = c.scan_of_old(*OLD["AUX1"])
    o, ys, xs = c.window(near, 220)
    side = component(dark_body(c, ys, xs), o, near, (8000, 80000), core=True)
    print_box("AUX1 body", c, side)
    found["AUX1"] = side

    # LEDs: the WS2812-2020 packages (warm white, R >= B; the silkscreen is lilac, B > R).
    for i, spot in enumerate(OLD["LED"]):
        near = c.scan_of_old(*spot)
        o, ys, xs = c.window(near, 180)
        R, G, B = c.R[ys, xs], c.G[ys, xs], c.B[ys, xs]
        mask = (c.lum[ys, xs] > 150) & (R >= B - 2) & (R - B < 45) & (G > 150)
        led = component(mask, o, near, (1500, 9000))
        print_box(f"LED_SPOTS[{i}]", c, led)
        found[f"LED{i}"] = led

    # Buttons: the tactile switches' black housings.
    sizes = []
    for name, spot in OLD["BUTTONS"].items():
        near = c.scan_of_old(*spot)
        o, ys, xs = c.window(near, 230)
        mask = (c.lum[ys, xs] < 70) & (np.abs(c.S[ys, xs]) < 20)
        sw_ = component(mask, o, near, (20000, 120000))
        print_box(f"BUTTON {name}", c, sw_)
        found[name] = sw_
        if sw_:
            sizes.append((sw_["box"][2] - sw_["box"][0]) * scale)
    print(f"tactile housing width: mean {np.mean(sizes):.2f} photo px ({100 * np.mean(sizes) / c.w:.3f} %)")

    # CHIP_SPOT: the LGA-12 (2 x 2 mm, pads 4/2/4/2 a side around a dark
    # body whose purple-grey matches the board's), located by its pads: the
    # small bright components (20-400 px; the silkscreen's are larger)
    # within 55 px. The four on each side (more than 25 px out in x) are
    # symmetric about the centre: x is the mean of the two sides' columns,
    # y the mean of the eight. From the old estimate: first the mean of the
    # pads within 75 px, then the sides, twice.
    cx, cy = c.scan_of_old(*OLD["CHIP"])
    for k in range(3):
        o, ys, xs = c.window((cx, cy), 75)
        pads = []
        for pix in label(c.lum[ys, xs] > 140):
            if 20 <= len(pix) <= 400:
                py = np.mean([q[0] for q in pix]) + o[1]
                px = np.mean([q[1] for q in pix]) + o[0]
                if math.hypot(px - cx, py - cy) < (75 if k == 0 else 55):
                    pads.append((px, py))
        if k == 0:
            cx, cy = float(np.mean([q[0] for q in pads])), float(np.mean([q[1] for q in pads]))
            continue
        left = [q for q in pads if q[0] < cx - 25]
        right = [q for q in pads if q[0] > cx + 25]
        cx = (np.mean([q[0] for q in left]) + np.mean([q[0] for q in right])) / 2
        cy = float(np.mean([q[1] for q in left + right]))
    fx, fy = c.photo(cx, cy)
    print(f"CHIP_SPOT = {{ x: {fx / c.w:.4f}, y: {fy / c.h:.4f} }}  ({len(left)} + {len(right)} side pads; photo {fx:.1f}, {fy:.1f})")
    found["CHIP"] = dict(box=(cx - 34, cy - 34, cx + 34, cy + 34), centre=(cx, cy))
    return found


def dark_body(c, ys, xs):
    """A switch's dark body: lum 14-70, unsaturated (the fixture's black is under 14)."""
    lum = c.lum[ys, xs]
    return (lum >= 14) & (lum < 70) & (np.abs(c.S[ys, xs]) < 25)


def print_box(name, c, comp):
    if comp is None:
        print(f"{name}: NOT FOUND")
        return
    x0, y0, x1, y1 = comp["box"]
    px, py = c.pct(*comp["centre"])
    print(f"{name}: centre [{px:.2f}, {py:.2f}] %, size {(x1 - x0) * c.scale:.1f} x {(y1 - y0) * c.scale:.1f} photo px ({100 * (x1 - x0) * c.scale / c.w:.2f} % x {100 * (y1 - y0) * c.scale / c.h:.2f} %)")
