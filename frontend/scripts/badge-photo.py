"""Builds frontend/public/badge/front.webp from the organizers' front scan.

Milestone 14, Task 11c. The source is the orthographic close-up scan served
by https://badge.hackthenorth.com/closeup?side=front (5643x3518 PNG, the
badge lying rotated 90 degrees, its top edge on the left; sha256
7b080d2904278896a19423ba3ef147df513c9360baf3b13227dd3a1c5636fa4c). It is
not committed; keep it under local/.

    local/.venv/bin/python -I frontend/scripts/badge-photo.py \
        local/m14-source/badge-front.7b080d290427.png \
        frontend/public/badge/front.webp

prints the outline's measurements and writes the WebP. Add `--geometry` to
also print the board features' positions (badge-geometry.ts' values), in
percent of the photo. Needs Pillow and numpy (Pillow 11.3.0, numpy 2.0.2
were used); neither is a project dependency.

The steps, all in the scan's pixels after the 90 degree turn:

1. Edge lines. The top and bottom edges meet the fixture's background:
   the crossing of B-G = 28 (board margin about 50, background about
   0-5). The long sides meet the panel rails across a light gap; the
   board's edge is the gap's board-side boundary, the crossing of G = 57
   (gap about 90, the board's dark edge margin about 24). Each is a
   robust least-squares line.
2. Level: rotate by the mean angle of the two long sides (the board
   hangs by them; Ruling R14 levels the board, not the LCD).
3. Outline: traced per row (left and right) and per column (top and
   bottom) on the levelled scan (trace_outline), so the rounded corners,
   the two side recesses, the USB-C shell and the slide switch's actuator
   in the left recess, and the power switch's actuator above the top
   edge all follow the real edge. The panel rails, clamps, strips and
   background are outside it. The four through-holes are fitted circles.
4. Corner radii, for the report and badge-geometry.ts' BOARD_CORNERS: an
   arc tangent to the edge lines, fitted on the middle 60% of each corner
   (rays from the corner point inward, the first B-G = 28 crossing).
5. Mask: the traced outline's coverage, 4x4 supersampled, minus the
   holes. Crop to the outline's extents, downscale with Lanczos
   (premultiplied by the mask) to WIDTH px, and encode a lossy WebP with
   lossless alpha. The output is byte-identical on a re-run.
"""

import math
import sys

import numpy as np
from PIL import Image

WIDTH = 974
QUALITY = 88
SUPERSAMPLE = 4  # mask coverage samples per axis at scan resolution

# Approximate feature positions in the turned scan (search windows only;
# every value below is re-fitted).
HOLE_GUESS = [(788, 588), (2700, 580), (820, 5200), (2732, 5192)]
EDGE_SPAN_X = (700, 2800)  # columns used for the top and bottom edges
EDGE_SPAN_Y = (800, 4900)  # rows used for the long sides
CORNER_ARC = (0.2, 0.8)  # the part of each quarter arc used for its fit


def load(path):
    im = Image.open(path).convert("RGB").transpose(Image.Transpose.ROTATE_270)
    return im


def channels(im):
    a = np.asarray(im).astype(np.float32)
    return a[..., 0], a[..., 1], a[..., 2]


def robust_line(t, v, iters=5, k=2.5):
    t = np.asarray(t, float)
    v = np.asarray(v, float)
    keep = np.ones(len(t), bool)
    for _ in range(iters):
        p = np.polyfit(t[keep], v[keep], 1)
        r = v - np.polyval(p, t)
        s = 1.4826 * np.median(np.abs(r[keep])) + 1e-6
        keep = np.abs(r) < max(k * s, 0.5)
    r = v - np.polyval(p, t)
    return p, float(np.sqrt(np.mean(r[keep] ** 2))), int(keep.sum()), len(t)


def crossing(prof, thr, start, step):
    """The first sub-pixel crossing of thr walking from start by step."""
    i = start
    while 0 <= i + step < len(prof):
        a, b = prof[i], prof[i + step]
        if (a - thr) * (b - thr) <= 0 and a != b:
            return i + step * (thr - a) / (b - a)
        i += step
    return None


def edge_points(im):
    _, G, B = channels(im)
    S = B - G
    top, bottom, left, right = [], [], [], []
    for x in range(EDGE_SPAN_X[0], EDGE_SPAN_X[1], 4):
        c = crossing(S[300:750, x], 28, 0, 1)
        if c is not None:
            top.append((x, 300 + c))
        prof = S[5100:5550, x]
        c = crossing(prof, 28, len(prof) - 1, -1)
        if c is not None:
            bottom.append((x, 5100 + c))
    for side, (x0, x1), inward in (("l", (140, 250), 1), ("r", (3280, 3370), -1)):
        for y in range(EDGE_SPAN_Y[0], EDGE_SPAN_Y[1], 4):
            # Skip the light leak's (green) rows.
            if (G[y, x0:x1] > 180).any():
                continue
            c = gap_edge(G[y, x0:x1], inward)
            if c is not None:
                (left if side == "l" else right).append((y, x0 + c))
    return top, bottom, left, right


def fit_edges(im):
    top, bottom, left, right = edge_points(im)
    out = {}
    for name, pts in (("top", top), ("bottom", bottom), ("left", left), ("right", right)):
        p, rms, kept, n = robust_line([q[0] for q in pts], [q[1] for q in pts])
        out[name] = dict(p=p, deg=math.degrees(math.atan(p[0])), rms=rms, kept=kept, n=n)
    return out


def level_angle(edges):
    """Degrees to rotate counter-clockwise (PIL's sense) to level the long sides.

    A side's x grows down the image by tan(t) per row when the board is
    turned t counter-clockwise on screen, so rotating by -t levels it.
    """
    return -(edges["left"]["deg"] + edges["right"]["deg"]) / 2


def bilinear(img, x, y):
    x = min(max(x, 0), img.shape[1] - 2.001)
    y = min(max(y, 0), img.shape[0] - 2.001)
    x0, y0 = int(x), int(y)
    fx, fy = x - x0, y - y0
    return (
        img[y0, x0] * (1 - fx) * (1 - fy)
        + img[y0, x0 + 1] * fx * (1 - fy)
        + img[y0 + 1, x0] * (1 - fx) * fy
        + img[y0 + 1, x0 + 1] * fx * fy
    )


def gap_edge(g, inward):
    """The board-side boundary (G = 57) of the light gap between rail and board.

    The gap is a 5-25 px run of G > 60 with the board's dark edge margin
    (G < 50 for 6 px) on its inward side; that rejects the rail's lit
    patches and the board's silkscreen. A recess row has no gap.
    """
    on = g > 60
    runs, i = [], 0
    while i < len(g):
        if on[i]:
            j = i
            while j + 1 < len(g) and on[j + 1]:
                j += 1
            runs.append((i, j))
            i = j + 1
        else:
            i += 1
    for a, b in runs if inward < 0 else reversed(runs):
        if not 5 <= b - a + 1 <= 25:
            continue
        if inward > 0 and b + 7 <= len(g) and (g[b + 1 : b + 7] < 50).all():
            return crossing(g, 57, b, 1)
        if inward < 0 and a - 6 >= 0 and (g[a - 6 : a] < 50).all():
            return crossing(g, 57, a, -1)
    return None


def axis_edges(lev):
    """The levelled board's four edges (scan px), as trimmed means of the crossings."""
    top, bottom, left, right = edge_points(lev)

    def med(pts):
        v = np.asarray([q[1] for q in pts])
        m = np.median(v)
        return float(np.mean(v[np.abs(v - m) < 2.0]))

    return med(left), med(top), med(right), med(bottom)


def corner_radius(S, corner, L, T, Rt, Bm):
    """Fits one corner's radius, its arc tangent to both edges."""
    sx = 1 if corner in ("TL", "BL") else -1
    sy = 1 if corner in ("TL", "TR") else -1
    ex = L if sx > 0 else Rt
    ey = T if sy > 0 else Bm
    # Rays from the corner point (outside the arc) inward, across the middle
    # of the arc; the first B-G = 28 crossing upward is the board's edge.
    base = math.degrees(math.atan2(sy, sx))
    pts = []
    for f in np.linspace(CORNER_ARC[0], CORNER_ARC[1], 120):
        ang = math.radians(base - 45 + 90 * f)
        dx, dy = math.cos(ang), math.sin(ang)
        ts = np.arange(0, 400, 0.5)
        prof = np.array([bilinear(S, ex + dx * t, ey + dy * t) for t in ts])
        c = crossing(prof, 28, 0, 1)
        if c is not None:
            pts.append((ex + dx * c * 0.5, ey + dy * c * 0.5))
    pts = np.array(pts)

    def resid(r):
        ccx, ccy = ex + sx * r, ey + sy * r
        return np.hypot(pts[:, 0] - ccx, pts[:, 1] - ccy) - r

    lo, hi = 50.0, 900.0
    for _ in range(80):  # golden-section search on the squared residual
        m1, m2 = lo + (hi - lo) * 0.382, lo + (hi - lo) * 0.618
        if np.sum(resid(m1) ** 2) < np.sum(resid(m2) ** 2):
            hi = m2
        else:
            lo = m1
    r = (lo + hi) / 2
    res = resid(r)
    # The free (untangent) circle, for the report.
    A = np.c_[2 * pts[:, 0], 2 * pts[:, 1], np.ones(len(pts))]
    c = np.linalg.lstsq(A, pts[:, 0] ** 2 + pts[:, 1] ** 2, rcond=None)[0]
    free_r = math.sqrt(c[2] + c[0] ** 2 + c[1] ** 2)
    return dict(r=r, rms=float(np.sqrt(np.mean(res**2))), max=float(np.max(np.abs(res))), n=len(pts), free_r=free_r, free_c=(c[0], c[1]))


def hole_circle(S, guess):
    """Fits a through-hole's edge: rays out from its middle, first B-G = 28 crossing."""
    cx, cy = guess
    for _ in range(2):
        pts = []
        for ang in np.linspace(0, 2 * math.pi, 180, endpoint=False):
            dx, dy = math.cos(ang), math.sin(ang)
            ts = np.arange(0, 140, 0.5)
            prof = np.array([bilinear(S, cx + dx * t, cy + dy * t) for t in ts])
            c = crossing(prof, 28, 0, 1)
            if c is not None:
                t = c * 0.5
                pts.append((cx + dx * t, cy + dy * t))
        pts = np.array(pts)
        A = np.c_[2 * pts[:, 0], 2 * pts[:, 1], np.ones(len(pts))]
        b = pts[:, 0] ** 2 + pts[:, 1] ** 2
        keep = np.ones(len(pts), bool)
        for _ in range(4):
            c = np.linalg.lstsq(A[keep], b[keep], rcond=None)[0]
            r = math.sqrt(c[2] + c[0] ** 2 + c[1] ** 2)
            res = np.hypot(pts[:, 0] - c[0], pts[:, 1] - c[1]) - r
            keep = np.abs(res) < max(3 * 1.4826 * np.median(np.abs(res[keep])), 0.5)
        cx, cy = c[0], c[1]
    return dict(x=cx, y=cy, r=r, rms=float(np.sqrt(np.mean(res[keep] ** 2))))


def _scan_in(ok, black, bright, start, step, stop, need=3):
    """Walks from start (outside the board) by step toward stop; the board's first index.

    Leading board-like pixels are a panel rail and are skipped. After a
    fixture-black pixel (a recess), any bright pixel is board (the USB-C
    shell and the slide switch's actuator stand in the left recess; the
    shell's face is dark inside a thin bright rim). Otherwise the board
    must hold for `need` pixels in a row.
    """
    i = start
    while (i - stop) * step < 0 and ok[i]:
        i += step
    seen_black = False
    while (i - stop) * step < 0:
        seen_black = seen_black or black[i]
        if (seen_black and bright[i]) or all(ok[k] for k in range(i, i + step * need, step)):
            return i
        i += step
    return None


def trace_outline(lev, box):
    """The board's edge per row (left, right) and per column (top, bottom).

    Rows are scanned inward from just outside the straight sides (in the
    light gap, which is never board) and columns inward from just outside
    the top and bottom edges. In a row, a pixel is board when it is purple
    (B-G at least 28) and not the gap's grey (G under 57), or bright and
    not green; after the fixture's black (a recess) any non-dark,
    non-green pixel is board (see _scan_in). Columns use B-G at least 35
    (the silkscreen reaches the bottom edge; 35 skips the faint purple
    line outside the top edge's light rim), or bright and not green: the
    power switch's actuator stands above the top edge, so the top scan
    starts 40 px above it. The board's edge is
    the outer edge of its dark solder-mask margin: the thin light rim
    beside the sides and the top (the gap, or the edge's face) is left
    out. The crossing is interpolated on B-G, or on the max channel at 40
    for a bright pixel. Each boundary is median filtered over 9 lines
    against specks.
    """
    R, G, B = channels(lev)
    S = B - G
    mx = np.maximum(np.maximum(R, G), B)
    board = ((S >= 28) & (G < 57)) | ((mx >= 170) & (G <= R + 10))
    black = mx < 30
    bright = (mx >= 40) & (G <= R + 10)
    # The bright rule is for the parts standing in a recess, well inside the
    # straight edge; nearer, a bright pixel is the light gap's grey.
    xs = np.arange(S.shape[1])
    bright_l = bright & (xs >= box[0] + 15)[None, :]
    bright_r = bright & (xs <= box[2] - 15)[None, :]
    L, T, Rt, Bm = box
    h, w = S.shape
    left = np.full(h, np.nan)
    right = np.full(h, np.nan)
    top = np.full(w, np.nan)
    bottom = np.full(w, np.nan)
    mid_x, mid_y = int((L + Rt) / 2), int((T + Bm) / 2)

    def sub(prof_s, prof_m, i, step, thr=28):
        # Edge between i - step (outside) and i (board).
        o = i - step
        if prof_s[i] >= thr and prof_s[o] < thr and prof_s[i] != prof_s[o]:
            return o + step * (thr - prof_s[o]) / (prof_s[i] - prof_s[o])
        if prof_m[i] != prof_m[o] and min(prof_m[o], prof_m[i]) <= 40 <= max(prof_m[o], prof_m[i]):
            return o + step * (40 - prof_m[o]) / (prof_m[i] - prof_m[o])
        return o + step * 0.5

    for y in range(int(T) - 20, int(Bm) + 21):
        row = board[y]
        i = _scan_in(row, black[y], bright_l[y], int(round(L)) - 7, 1, mid_x)
        if i is not None:
            left[y] = sub(S[y], mx[y], i, 1)
        i = _scan_in(row, black[y], bright_r[y], int(round(Rt)) + 7, -1, mid_x)
        if i is not None:
            right[y] = sub(S[y], mx[y], i, -1)
    col_board = (S >= 35) | ((mx >= 170) & (G <= R + 10))
    nothing = np.zeros(h, bool)
    for x in range(int(L) - 20, int(Rt) + 21):
        col = col_board[:, x]
        i = _scan_in(col, nothing, nothing, int(round(T)) - 40, 1, mid_y)
        if i is not None:
            top[x] = sub(S[:, x], mx[:, x], i, 1, 35)
        i = _scan_in(col, nothing, nothing, int(round(Bm)) + 15, -1, mid_y)
        if i is not None:
            bottom[x] = sub(S[:, x], mx[:, x], i, -1, 35)

    def median9(v):
        out = v.copy()
        for k in range(4, len(v) - 4):
            win = v[k - 4 : k + 5]
            win = win[~np.isnan(win)]
            if len(win) >= 5 and not np.isnan(v[k]):
                out[k] = np.median(win)
        return out

    left, right, top, bottom = (median9(v) for v in (left, right, top, bottom))
    # Rows (columns) whose scans met at the middle without finding board, or
    # that lie outside the board, have no edge.
    return dict(left=left, right=right, top=top, bottom=bottom)


def build_mask(shape, trace, holes):
    """Board coverage in [0, 1] at scan resolution (supersampled).

    Inside means right of the row's left edge, left of its right edge,
    below the column's top edge and above its bottom edge, and outside
    the four holes.
    """
    h, w = shape
    n = SUPERSAMPLE
    off = (np.arange(n) + 0.5) / n
    big = 1e9
    lft = np.nan_to_num(trace["left"], nan=big)
    rgt = np.nan_to_num(trace["right"], nan=-big)
    top = np.nan_to_num(trace["top"], nan=big)
    bot = np.nan_to_num(trace["bottom"], nan=-big)
    ys = np.arange(h)
    xs = np.arange(w)
    cov = np.zeros((h, w), np.float32)
    for oy in off:
        Y = (ys + oy)[:, None]
        for ox in off:
            X = (xs + ox)[None, :]
            inside = (X > lft[ys][:, None]) & (X < rgt[ys][:, None]) & (Y > top[xs][None, :]) & (Y < bot[xs][None, :])
            for hole in holes:
                inside &= (X - hole["x"]) ** 2 + (Y - hole["y"]) ** 2 > hole["r"] ** 2
            cov += inside
    return cov / (n * n)


def measure_outline(scan):
    edges = fit_edges(scan)
    angle = level_angle(edges)
    lev = scan.rotate(angle, resample=Image.Resampling.BICUBIC, fillcolor=(0, 0, 0))
    _, G, B = channels(lev)
    S = B - G
    lines = axis_edges(lev)
    lev_edges = fit_edges(lev)
    trace = trace_outline(lev, lines)
    # The photo's frame: the traced board's extents.
    box = (
        float(np.nanmin(trace["left"])),
        float(np.nanmin(trace["top"])),
        float(np.nanmax(trace["right"])),
        float(np.nanmax(trace["bottom"])),
    )
    radii = {c: corner_radius(S, c, *lines) for c in ("TL", "TR", "BL", "BR")}
    holes = [hole_circle(S, g) for g in level_points(HOLE_GUESS, angle, scan.size)]
    return dict(edges=edges, angle=angle, lev=lev, lev_edges=lev_edges, lines=lines, box=box, trace=trace, radii=radii, holes=holes)


def level_points(pts, angle, size):
    """Maps turned-scan points into the levelled image (PIL rotates about the centre)."""
    cx, cy = size[0] / 2, size[1] / 2
    a = math.radians(angle)
    out = []
    for x, y in pts:
        dx, dy = x - cx, y - cy
        # PIL's rotate(angle) turns the content counter-clockwise on screen.
        out.append((cx + dx * math.cos(a) + dy * math.sin(a), cy - dx * math.sin(a) + dy * math.cos(a)))
    return out


def render(m):
    L, T, Rt, Bm = m["box"]
    lev = m["lev"]
    scale = WIDTH / (Rt - L)
    height = round((Bm - T) * scale)
    box = (L, T, Rt, T + height / scale)
    mask = build_mask((lev.size[1], lev.size[0]), m["trace"], m["holes"])
    rgb = np.asarray(lev).astype(np.float32)
    planes = []
    for ch in range(3):
        pm = Image.fromarray(rgb[..., ch] * mask)
        planes.append(np.asarray(pm.resize((WIDTH, height), Image.Resampling.LANCZOS, box=box)))
    alpha = np.asarray(Image.fromarray(mask).resize((WIDTH, height), Image.Resampling.LANCZOS, box=box))
    alpha = np.clip(alpha, 0, 1)
    out = np.zeros((height, WIDTH, 4), np.uint8)
    safe = np.maximum(alpha, 1e-3)
    for ch in range(3):
        out[..., ch] = np.clip(np.round(planes[ch] / safe), 0, 255).astype(np.uint8)
    out[..., 3] = np.round(alpha * 255).astype(np.uint8)
    out[out[..., 3] == 0, :3] = 0
    return Image.fromarray(out), scale, height


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 2
    src, dst = argv[1], argv[2]
    scan = load(src)
    m = measure_outline(scan)
    e = m["edges"]
    print("turned scan", scan.size)
    for k in ("left", "right", "top", "bottom"):
        print(f"  {k:6s} edge {e[k]['deg']:+.4f} deg (rms {e[k]['rms']:.2f} px, {e[k]['kept']}/{e[k]['n']} points)")
    print(f"  shear (sides vs top/bottom): {abs((e['left']['deg'] + e['right']['deg']) / 2 + (e['top']['deg'] + e['bottom']['deg']) / 2):.4f} deg")
    print(f"level: rotate {m['angle']:+.4f} deg (counter-clockwise positive)")
    le = m["lev_edges"]
    for k in ("left", "right", "top", "bottom"):
        print(f"  levelled {k:6s} {le[k]['deg']:+.4f} deg")
    L, T, Rt, Bm = m["box"]
    print(f"board box (levelled scan px): x {L:.2f}-{Rt:.2f}, y {T:.2f}-{Bm:.2f}; {Rt - L:.2f} x {Bm - T:.2f}, aspect {(Rt - L) / (Bm - T):.5f}")
    img, scale, height = render(m)
    print(f"photo: {WIDTH} x {height}, aspect {WIDTH / height:.5f}, scale {scale:.6f}")
    for k, v in m["radii"].items():
        print(f"  corner {k}: r {v['r']:.1f} scan px = {v['r'] * scale:.2f} photo px (trace vs arc: rms {v['rms']:.2f}, max {v['max']:.2f} scan px; untangent fit r {v['free_r']:.1f})")
    for i, hole in enumerate(m["holes"]):
        print(f"  hole {i}: centre {(hole['x'] - L) * scale:.2f}, {(hole['y'] - T) * scale:.2f} photo px, r {hole['r'] * scale:.2f} (rms {hole['rms']:.2f} scan px)")
    lines = m["lines"]
    print(f"  straight-edge lines (levelled scan px): left {lines[0]:.2f}, top {lines[1]:.2f}, right {lines[2]:.2f}, bottom {lines[3]:.2f}")
    tr = m["trace"]
    for side, depth in (("left", tr["left"] - lines[0]), ("right", lines[2] - tr["right"])):
        rows = np.where(depth > 15)[0]
        rows = rows[(rows > T + 400) & (rows < Bm - 700)]
        if len(rows):
            print(f"  {side} recess: y {(rows[0] - T) * scale:.1f}-{(rows[-1] - T) * scale:.1f} photo px, depth up to {np.nanmax(depth[rows]) * scale:.2f} photo px")
    img.save(dst, "WEBP", quality=QUALITY, alpha_quality=100, method=6, exact=False)
    if "--geometry" in argv:
        import importlib.util
        import os

        spec = importlib.util.spec_from_file_location("badge_features", os.path.join(os.path.dirname(os.path.abspath(__file__)), "badge-features.py"))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        mod.report(m, scale, height)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
