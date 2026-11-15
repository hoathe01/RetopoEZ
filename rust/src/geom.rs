//! Pure geometry: OBJ load, raycast, closest point, patch generation, weld + OBJ export.
use glam::Vec3;
use std::{collections::HashMap, path::Path};

pub struct Mesh {
    pub pos: Vec<Vec3>,
    pub idx: Vec<u32>,
    pub center: Vec3,
    pub diag: f32,
    bvh: Bvh,
}

impl Mesh {
    pub fn new(pos: Vec<Vec3>, idx: Vec<u32>) -> Mesh {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in &pos {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let bvh = Bvh::build(&pos, &idx);
        Mesh { pos, idx, center: (lo + hi) * 0.5, diag: (hi - lo).length().max(1e-6), bvh }
    }
}

// ---------------------------------------------------------------- BVH (median split, leaves of <= 4 triangles)
struct Node {
    lo: Vec3,
    hi: Vec3,
    a: u32, // inner: left child (right = a + 1 is NOT assumed, see b) / leaf: first slot in `order`
    b: u32, // inner: right child / leaf: unused
    n: u32, // 0 = inner node, else triangle count
}

struct Bvh {
    nodes: Vec<Node>,
    order: Vec<u32>, // triangle ids, leaf ranges index into this
}

impl Bvh {
    fn build(pos: &[Vec3], idx: &[u32]) -> Bvh {
        let nt = idx.len() / 3;
        let bb: Vec<(Vec3, Vec3)> = idx
            .chunks_exact(3)
            .map(|t| {
                let (a, b, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
                (a.min(b).min(c), a.max(b).max(c))
            })
            .collect();
        let cen: Vec<Vec3> = bb.iter().map(|(l, h)| (*l + *h) * 0.5).collect();
        let mut order: Vec<u32> = (0..nt as u32).collect();
        let mut nodes = Vec::with_capacity(nt / 2 + 1);
        // explicit stack: (node slot, range start, range end)
        nodes.push(Node { lo: Vec3::ZERO, hi: Vec3::ZERO, a: 0, b: 0, n: 0 });
        let mut stack = vec![(0usize, 0usize, nt)];
        while let Some((slot, s, e)) = stack.pop() {
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            let (mut clo, mut chi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for &t in &order[s..e] {
                lo = lo.min(bb[t as usize].0);
                hi = hi.max(bb[t as usize].1);
                clo = clo.min(cen[t as usize]);
                chi = chi.max(cen[t as usize]);
            }
            if e - s <= 4 || (chi - clo).max_element() <= 0.0 {
                nodes[slot] = Node { lo, hi, a: s as u32, b: 0, n: (e - s) as u32 };
                continue;
            }
            let ext = chi - clo;
            let ax = if ext.x >= ext.y && ext.x >= ext.z { 0 } else if ext.y >= ext.z { 1 } else { 2 };
            let mid = (s + e) / 2;
            order[s..e].select_nth_unstable_by(mid - s, |&x, &y| cen[x as usize][ax].total_cmp(&cen[y as usize][ax]));
            let (l, r) = (nodes.len(), nodes.len() + 1);
            nodes.push(Node { lo: Vec3::ZERO, hi: Vec3::ZERO, a: 0, b: 0, n: 0 });
            nodes.push(Node { lo: Vec3::ZERO, hi: Vec3::ZERO, a: 0, b: 0, n: 0 });
            nodes[slot] = Node { lo, hi, a: l as u32, b: r as u32, n: 0 };
            stack.push((l, s, mid));
            stack.push((r, mid, e));
        }
        Bvh { nodes, order }
    }
}

pub fn load_obj(path: &Path) -> Result<Mesh, String> {
    let opts = tobj::LoadOptions { triangulate: true, single_index: true, ..Default::default() };
    let (models, _) = tobj::load_obj(path, &opts).map_err(|e| e.to_string())?;
    let (mut pos, mut idx) = (Vec::new(), Vec::new());
    for m in &models {
        let base = pos.len() as u32;
        pos.extend(m.mesh.positions.chunks_exact(3).map(|p| Vec3::new(p[0], p[1], p[2])));
        idx.extend(m.mesh.indices.iter().map(|i| i + base));
    }
    if idx.is_empty() {
        return Err("no triangles".into());
    }
    Ok(Mesh::new(pos, idx))
}

fn tri(m: &Mesh, t: &[u32]) -> (Vec3, Vec3, Vec3) {
    (m.pos[t[0] as usize], m.pos[t[1] as usize], m.pos[t[2] as usize])
}

/// Moller-Trumbore: hit distance along d (> 0) or None.
fn ray_tri(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (e1, e2) = (b - a, c - a);
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0).then_some(t)
}

/// Closest ray hit: (t, geometric normal facing the ray).
pub fn raycast(m: &Mesh, o: Vec3, d: Vec3) -> Option<(f32, Vec3)> {
    let inv = Vec3::ONE / d;
    let mut best: Option<(f32, usize)> = None;
    let mut stack = vec![0usize];
    while let Some(i) = stack.pop() {
        let nd = &m.bvh.nodes[i];
        let (t0, t1) = ((nd.lo - o) * inv, (nd.hi - o) * inv);
        let (tn, tf) = (t0.min(t1).max_element(), t0.max(t1).min_element());
        if tf < tn.max(0.0) || best.map_or(false, |(bt, _)| tn > bt) {
            continue;
        }
        if nd.n == 0 {
            stack.push(nd.a as usize);
            stack.push(nd.b as usize);
        } else {
            for &t in &m.bvh.order[nd.a as usize..(nd.a + nd.n) as usize] {
                let (a, b, c) = tri(m, &m.idx[t as usize * 3..t as usize * 3 + 3]);
                if let Some(h) = ray_tri(o, d, a, b, c) {
                    if best.map_or(true, |(bt, _)| h < bt) {
                        best = Some((h, t as usize));
                    }
                }
            }
        }
    }
    best.map(|(t, ti)| {
        let (a, b, c) = tri(m, &m.idx[ti * 3..ti * 3 + 3]);
        let n = (b - a).cross(c - a).normalize_or_zero();
        (t, if n.dot(d) > 0.0 { -n } else { n })
    })
}

fn closest_on_tri(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let k = 1.0 / (va + vb + vc);
    a + ab * (vb * k) + ac * (vc * k)
}

/// Closest surface point and the normal of its triangle.
pub fn closest(m: &Mesh, p: Vec3) -> (Vec3, Vec3) {
    let box_d2 = |n: &Node| (n.lo - p).max(Vec3::ZERO).max(p - n.hi).length_squared();
    let mut best = (f32::MAX, p, Vec3::Y);
    let mut stack = vec![0usize];
    while let Some(i) = stack.pop() {
        let nd = &m.bvh.nodes[i];
        if box_d2(nd) >= best.0 {
            continue;
        }
        if nd.n == 0 {
            let (l, r) = (nd.a as usize, nd.b as usize);
            // visit the nearer child first (pushed last)
            if box_d2(&m.bvh.nodes[l]) <= box_d2(&m.bvh.nodes[r]) {
                stack.push(r);
                stack.push(l);
            } else {
                stack.push(l);
                stack.push(r);
            }
        } else {
            for &t in &m.bvh.order[nd.a as usize..(nd.a + nd.n) as usize] {
                let (a, b, c) = tri(m, &m.idx[t as usize * 3..t as usize * 3 + 3]);
                let q = closest_on_tri(p, a, b, c);
                let d = q.distance_squared(p);
                if d < best.0 {
                    best = (d, q, (b - a).cross(c - a).normalize_or_zero());
                }
            }
        }
    }
    (best.1, best.2)
}

// ---------------------------------------------------------------- patches
/// n+1 points at even arc length along a polyline.
pub fn sample(s: &[Vec3], n: usize) -> Vec<Vec3> {
    let mut cum = vec![0.0f32];
    for w in s.windows(2) {
        cum.push(cum.last().unwrap() + w[0].distance(w[1]));
    }
    let total = *cum.last().unwrap();
    let mut seg = 0;
    (0..=n)
        .map(|k| {
            let l = total * k as f32 / n as f32;
            while seg + 2 < s.len() && cum[seg + 1] < l {
                seg += 1;
            }
            let span = (cum[seg + 1] - cum[seg]).max(1e-12);
            s[seg].lerp(s[seg + 1], ((l - cum[seg]) / span).clamp(0.0, 1.0))
        })
        .collect()
}

/// Reverse b if it runs opposite to a.
pub fn align(a: &[Vec3], mut b: Vec<Vec3>) -> Vec<Vec3> {
    let (n, m) = (a.len() - 1, b.len() - 1);
    let same = a[0].distance_squared(b[0]) + a[n].distance_squared(b[m]);
    let flip = a[0].distance_squared(b[m]) + a[n].distance_squared(b[0]);
    if same > flip {
        b.reverse();
    }
    b
}

pub type Grid = Vec<Vec<Vec3>>; // grid[j][i]

pub fn bridge(a: &[Vec3], b: &[Vec3], rows: usize) -> Grid {
    (0..=rows).map(|j| a.iter().zip(b).map(|(p, q)| p.lerp(*q, j as f32 / rows as f32)).collect()).collect()
}

/// bottom/top: nu+1 pts left->right; left/right: nv+1 pts bottom->top.
pub fn coons(bottom: &[Vec3], right: &[Vec3], top: &[Vec3], left: &[Vec3]) -> Grid {
    let (nu, nv) = (bottom.len() - 1, left.len() - 1);
    let (p00, p10, p01, p11) = (bottom[0], bottom[nu], top[0], top[nu]);
    (0..=nv)
        .map(|j| {
            let v = j as f32 / nv as f32;
            (0..=nu)
                .map(|i| {
                    let u = i as f32 / nu as f32;
                    bottom[i] * (1.0 - v) + top[i] * v + left[j] * (1.0 - u) + right[j] * u
                        - (p00 * ((1.0 - u) * (1.0 - v)) + p10 * (u * (1.0 - v)) + p01 * ((1.0 - u) * v) + p11 * (u * v))
                })
                .collect()
        })
        .collect()
}

/// ends = (start, end) of 4 strokes -> [(index, reversed)] chained head to tail.
pub fn order_loop(ends: &[(Vec3, Vec3)]) -> Vec<(usize, bool)> {
    let mut order = vec![(0, false)];
    let mut cur = ends[0].1;
    let mut left = vec![1, 2, 3];
    while !left.is_empty() {
        let k = *left
            .iter()
            .min_by(|&&a, &&b| {
                let d = |n: usize| cur.distance_squared(ends[n].0).min(cur.distance_squared(ends[n].1));
                d(a).total_cmp(&d(b))
            })
            .unwrap();
        let rev = cur.distance_squared(ends[k].1) < cur.distance_squared(ends[k].0);
        order.push((k, rev));
        cur = if rev { ends[k].0 } else { ends[k].1 };
        left.retain(|&x| x != k);
    }
    order
}

/// sampler(stroke_index, n) -> n+1 points. U spans along bottom/top, V along the sides.
pub fn four_grid(ends: &[(Vec3, Vec3)], sampler: impl Fn(usize, usize) -> Vec<Vec3>, u: usize, v: usize) -> Grid {
    let order = order_loop(ends);
    let trav = |k: usize, n: usize| {
        let (idx, rev) = order[k];
        let mut p = sampler(idx, n);
        if rev {
            p.reverse();
        }
        p
    };
    let rev = |mut p: Vec<Vec3>| {
        p.reverse();
        p
    };
    coons(&trav(0, u), &trav(1, v), &rev(trav(2, u)), &rev(trav(3, v)))
}

pub struct Patch {
    pub nu: usize,
    pub nv: usize,
    pub pts: Vec<Vec3>, // row-major, j*(nu+1)+i, snapped to the surface
    pub nrm: Vec<Vec3>, // surface normal at each point
}

pub fn snap(m: &Mesh, grid: &Grid) -> Patch {
    let (nv, nu) = (grid.len() - 1, grid[0].len() - 1);
    let (pts, nrm) = grid.iter().flatten().map(|&p| closest(m, p)).unzip();
    Patch { nu, nv, pts, nrm }
}

/// Weld all patches (vertices closer than tol merge) and write an OBJ of quads.
pub fn to_obj<'a>(patches: impl IntoIterator<Item = &'a Patch>, tol: f32) -> String {
    let tol = tol.max(1e-9);
    let mut verts: Vec<Vec3> = Vec::new();
    let mut cells: HashMap<(i32, i32, i32), Vec<usize>> = HashMap::new();
    let cell = |p: Vec3| ((p.x / tol).floor() as i32, (p.y / tol).floor() as i32, (p.z / tol).floor() as i32);
    let mut faces: Vec<[usize; 4]> = Vec::new();
    for pa in patches {
        let ids: Vec<usize> = pa
            .pts
            .iter()
            .map(|&p| {
                let (cx, cy, cz) = cell(p);
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            for &id in cells.get(&(cx + dx, cy + dy, cz + dz)).into_iter().flatten() {
                                if verts[id].distance(p) <= tol {
                                    return id;
                                }
                            }
                        }
                    }
                }
                verts.push(p);
                cells.entry((cx, cy, cz)).or_default().push(verts.len() - 1);
                verts.len() - 1
            })
            .collect();
        let w = pa.nu + 1;
        let n0 = (pa.pts[1] - pa.pts[0]).cross(pa.pts[w + 1] - pa.pts[0]);
        let flip = n0.dot(pa.nrm[0]) < 0.0;
        for j in 0..pa.nv {
            for i in 0..pa.nu {
                let a = j * w + i;
                let mut q = [ids[a], ids[a + 1], ids[a + w + 1], ids[a + w]];
                if flip {
                    q.reverse();
                }
                if q[0] != q[1] && q[1] != q[2] && q[2] != q[3] && q[3] != q[0] && q[0] != q[2] && q[1] != q[3] {
                    faces.push(q);
                }
            }
        }
    }
    let mut s = String::new();
    for v in &verts {
        s += &format!("v {} {} {}\n", v.x, v.y, v.z);
    }
    for f in &faces {
        s += &format!("f {} {} {} {}\n", f[0] + 1, f[1] + 1, f[2] + 1, f[3] + 1);
    }
    s
}

// ---------------------------------------------------------------- symmetry
/// Reflect across the plane `axis = plane` (axis 0/1/2 = X/Y/Z) and drop the point back on the surface.
pub fn mirror_on_surface(m: &Mesh, p: Vec3, axis: usize, plane: f32, lift: f32) -> Vec3 {
    let mut r = p;
    r[axis] = 2.0 * plane - p[axis];
    let (q, n) = closest(m, r);
    q + n * lift // outward by winding, same side the original strokes are lifted to
}

// ---------------------------------------------------------------- auto patch
/// Closest points between segments p1q1 and p2q2: (s, t, dist^2).
fn seg_seg(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (f32, f32, f32) {
    const E: f32 = 1e-12;
    let (d1, d2, r) = (q1 - p1, q2 - p2, p1 - p2);
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    let (mut s, mut t);
    if a <= E && e <= E {
        return (0.0, 0.0, r.length_squared());
    }
    if a <= E {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= E {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let den = a * e - b * b;
            s = if den > E { ((b * f - c * e) / den).clamp(0.0, 1.0) } else { 0.0 };
            t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
        }
    }
    (s, t, (p1 + d1 * s - (p2 + d2 * t)).length_squared())
}

fn find(uf: &mut Vec<usize>, x: usize) -> usize {
    let mut r = x;
    while uf[r] != r {
        r = uf[r];
    }
    let mut c = x;
    while uf[c] != r {
        let nx = uf[c];
        uf[c] = r;
        c = nx;
    }
    r
}

fn unite(uf: &mut Vec<usize>, a: usize, b: usize) {
    let (ra, rb) = (find(uf, a), find(uf, b));
    uf[ra] = rb;
}

fn path_len(p: &[Vec3]) -> f32 {
    p.windows(2).map(|w| w[0].distance(w[1])).sum()
}

pub struct Auto {
    pub patches: Vec<Patch>,
    pub skipped: usize, // closed regions that are not 4-sided
}

/// Strokes that cross / touch (closer than `tol`) form a network; every 4-sided region becomes a
/// patch. Opposite sides of a quad share a span count, so neighbouring patches always weld.
// ponytail: each stroke pair intersects at most once (their closest approach); 3-/5-sided regions are skipped.
pub fn auto_patches(m: &Mesh, strokes: &[&[Vec3]], tol: f32, target: f32) -> Auto {
    let ns = strokes.len();
    let cum: Vec<Vec<f32>> = strokes
        .iter()
        .map(|s| {
            let mut c = vec![0.0];
            for w in s.windows(2) {
                c.push(c.last().unwrap() + w[0].distance(w[1]));
            }
            c
        })
        .collect();
    // 1. intersections -> raw nodes (position) + per-stroke events (arc position, node)
    let bb: Vec<(Vec3, Vec3)> = strokes
        .iter()
        .map(|s| s.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(l, h), p| (l.min(*p), h.max(*p))))
        .collect();
    let mut raw: Vec<Vec3> = Vec::new();
    let mut ev: Vec<Vec<(f32, usize)>> = vec![vec![]; ns];
    for i in 0..ns {
        for j in i + 1..ns {
            let t = Vec3::splat(tol);
            if (bb[i].0 - t).cmpgt(bb[j].1).any() || (bb[j].0 - t).cmpgt(bb[i].1).any() {
                continue; // boxes apart: cannot touch
            }
            // every crossing / touch between the pair (a U-shape and an arc meet twice), best first
            let mut cand: Vec<(f32, usize, f32, usize, f32)> = Vec::new();
            for (a, wa) in strokes[i].windows(2).enumerate() {
                for (b, wb) in strokes[j].windows(2).enumerate() {
                    let (s, t, d2) = seg_seg(wa[0], wa[1], wb[0], wb[1]);
                    if d2 < tol * tol {
                        cand.push((d2, a, s, b, t));
                    }
                }
            }
            cand.sort_by(|x, y| x.0.total_cmp(&y.0));
            let mut taken: Vec<(f32, f32)> = Vec::new();
            for (_, a, s, b, t) in cand {
                let (li, lj) = (cum[i][a] + s * (cum[i][a + 1] - cum[i][a]), cum[j][b] + t * (cum[j][b + 1] - cum[j][b]));
                if taken.len() >= 4 || taken.iter().any(|&(x, y)| (x - li).abs() < tol * 3.0 && (y - lj).abs() < tol * 3.0) {
                    continue; // same crossing seen from the neighbouring segment, or strokes running on top of each other
                }
                taken.push((li, lj));
                let pa = strokes[i][a].lerp(strokes[i][a + 1], s);
                let pb = strokes[j][b].lerp(strokes[j][b + 1], t);
                raw.push((pa + pb) * 0.5);
                let id = raw.len() - 1;
                ev[i].push((li, id));
                ev[j].push((lj, id));
            }
        }
    }
    // 1b. sharp bends become corner nodes, so a stroke that wraps around two sides of a cell still gives a quad
    for i in 0..ns {
        let len = *cum[i].last().unwrap();
        let win = (len * 0.05).max(tol * 4.0);
        if len < win * 3.0 {
            continue;
        }
        let at = |l: f32| {
            let k = cum[i].partition_point(|&c| c < l).clamp(1, strokes[i].len() - 1);
            let t = ((l - cum[i][k - 1]) / (cum[i][k] - cum[i][k - 1]).max(1e-12)).clamp(0.0, 1.0);
            strokes[i][k - 1].lerp(strokes[i][k], t)
        };
        let turn = |k: usize| {
            let l = cum[i][k];
            if l < win || l > len - win {
                return 0.0;
            }
            let (d0, d1) = ((strokes[i][k] - at(l - win)).normalize_or_zero(), (at(l + win) - strokes[i][k]).normalize_or_zero());
            d0.dot(d1).clamp(-1.0, 1.0).acos()
        };
        let a: Vec<f32> = (0..strokes[i].len()).map(turn).collect();
        for k in 0..a.len() {
            let peak = a[k] > 0.7 // ~40 degrees
                && (0..a.len()).all(|o| (cum[i][o] - cum[i][k]).abs() > win || a[o] < a[k] || (a[o] == a[k] && o >= k));
            if peak {
                raw.push(strokes[i][k]);
                ev[i].push((cum[i][k], raw.len() - 1));
            }
        }
    }
    // 2. merge raw nodes closer than 1.5 tol (3+ strokes meeting at one spot)
    let mut uf: Vec<usize> = (0..raw.len()).collect();
    for a in 0..raw.len() {
        for b in a + 1..raw.len() {
            if raw[a].distance(raw[b]) < tol * 1.5 {
                unite(&mut uf, a, b);
            }
        }
    }
    let mut ids = HashMap::new();
    let mut pos: Vec<Vec3> = Vec::new();
    let mut cnt: Vec<f32> = Vec::new();
    let node_of: Vec<usize> = (0..raw.len())
        .map(|r| {
            let root = find(&mut uf, r);
            let id = *ids.entry(root).or_insert_with(|| {
                pos.push(Vec3::ZERO);
                cnt.push(0.0);
                pos.len() - 1
            });
            pos[id] += raw[r];
            cnt[id] += 1.0;
            id
        })
        .collect();
    for (p, c) in pos.iter_mut().zip(&cnt) {
        *p /= *c;
    }
    // 3. edges = stroke pieces between consecutive nodes
    struct Edge {
        a: usize,
        b: usize,
        pts: Vec<Vec3>,
    }
    let mut edges: Vec<Edge> = Vec::new();
    for i in 0..ns {
        let mut e: Vec<(f32, usize)> = ev[i].iter().map(|&(l, r)| (l, node_of[r])).collect();
        e.sort_by(|x, y| x.0.total_cmp(&y.0));
        e.dedup_by_key(|x| x.1);
        for w in e.windows(2) {
            let ((l0, n0), (l1, n1)) = (w[0], w[1]);
            let mut pts = vec![pos[n0]];
            pts.extend((0..strokes[i].len()).filter(|&k| cum[i][k] > l0 && cum[i][k] < l1).map(|k| strokes[i][k]));
            pts.push(pos[n1]);
            edges.push(Edge { a: n0, b: n1, pts });
        }
    }
    // 4. prune dead ends
    let mut alive = vec![true; edges.len()];
    loop {
        let mut deg = vec![0; pos.len()];
        for (k, e) in edges.iter().enumerate() {
            if alive[k] {
                deg[e.a] += 1;
                deg[e.b] += 1;
            }
        }
        let mut changed = false;
        for (k, e) in edges.iter().enumerate() {
            if alive[k] && (deg[e.a] < 2 || deg[e.b] < 2) {
                alive[k] = false;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // 5. rotation system: outgoing half-edges per node sorted CCW around the surface normal
    // half-edge h = 2*edge + dir (dir 0: a->b, 1: b->a)
    let head = |h: usize| if h % 2 == 0 { edges[h / 2].b } else { edges[h / 2].a };
    let oriented = |h: usize| -> Vec<Vec3> {
        let mut p = edges[h / 2].pts.clone();
        if h % 2 == 1 {
            p.reverse();
        }
        p
    };
    let mut out: Vec<Vec<(f32, usize)>> = vec![vec![]; pos.len()];
    let mut nrm = vec![Vec3::Y; pos.len()];
    for n in 0..pos.len() {
        nrm[n] = closest(m, pos[n]).1;
        let r = if nrm[n].x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
        let t1 = (r - nrm[n] * r.dot(nrm[n])).normalize();
        let t2 = nrm[n].cross(t1);
        for h in 0..edges.len() * 2 {
            if !alive[h / 2] || (if h % 2 == 0 { edges[h / 2].a } else { edges[h / 2].b }) != n {
                continue;
            }
            let p = oriented(h);
            let d = p.iter().skip(1).map(|q| *q - pos[n]).find(|d| d.length() > tol * 0.5).unwrap_or(p[1] - pos[n]);
            out[n].push((d.dot(t2).atan2(d.dot(t1)), h));
        }
        out[n].sort_by(|x, y| x.0.total_cmp(&y.0));
    }
    // 6. trace faces (face on the left); keep CCW 4-cycles
    let mut seen = vec![false; edges.len() * 2];
    let mut quads: Vec<[usize; 4]> = Vec::new();
    let mut skipped = 0;
    for h0 in 0..edges.len() * 2 {
        if !alive[h0 / 2] || seen[h0] {
            continue;
        }
        let mut face = vec![];
        let mut h = h0;
        loop {
            seen[h] = true;
            face.push(h);
            let v = head(h);
            let list = &out[v];
            let k = list.iter().position(|x| x.1 == (h ^ 1)).unwrap();
            h = list[(k + list.len() - 1) % list.len()].1;
            if h == h0 || face.len() > 4096 {
                break;
            }
        }
        let pts: Vec<Vec3> = face.iter().map(|&h| pos[if h % 2 == 0 { edges[h / 2].a } else { edges[h / 2].b }]).collect();
        let area: Vec3 = (0..pts.len()).map(|i| pts[i].cross(pts[(i + 1) % pts.len()])).sum();
        let navg: Vec3 = face.iter().map(|&h| nrm[head(h)]).sum();
        if area.dot(navg) <= 0.0 {
            continue; // outer face
        }
        if face.len() == 4 {
            quads.push([face[0], face[1], face[2], face[3]]);
        } else {
            skipped += 1;
        }
    }
    // 7. opposite sides share a span count
    let mut cls: Vec<usize> = (0..edges.len()).collect();
    quads.retain(|q| q[0] / 2 != q[2] / 2 && q[1] / 2 != q[3] / 2);
    for q in &quads {
        unite(&mut cls, q[0] / 2, q[2] / 2);
        unite(&mut cls, q[1] / 2, q[3] / 2);
    }
    let mut sum: HashMap<usize, (f32, f32)> = HashMap::new();
    for e in 0..edges.len() {
        let r = find(&mut cls, e);
        let s = sum.entry(r).or_default();
        s.0 += path_len(&edges[e].pts);
        s.1 += 1.0;
    }
    let mut patches = Vec::new();
    for q in &quads {
        let n = |h: usize, cls: &mut Vec<usize>| {
            let (l, c) = sum[&find(cls, h / 2)];
            ((l / c / target).round() as usize).clamp(1, 256)
        };
        let (nu, nv) = (n(q[0], &mut cls), n(q[1], &mut cls));
        let side = |h: usize, k: usize| sample(&oriented(h), k);
        let (s0, s1, mut s2, mut s3) = (side(q[0], nu), side(q[1], nv), side(q[2], nu), side(q[3], nv));
        s2.reverse();
        s3.reverse();
        patches.push(snap(m, &coons(&s0, &s1, &s2, &s3)));
    }
    Auto { patches, skipped }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad_mesh() -> Mesh {
        Mesh::new(
            vec![Vec3::new(-1., -1., 0.), Vec3::new(1., -1., 0.), Vec3::new(1., 1., 0.), Vec3::new(-1., 1., 0.)],
            vec![0, 1, 2, 0, 2, 3],
        )
    }
    #[test]
    fn ray_and_closest() {
        let m = quad_mesh();
        let (t, n) = raycast(&m, Vec3::new(0.2, 0.1, 5.0), -Vec3::Z).unwrap();
        assert!((t - 5.0).abs() < 1e-5 && n.z > 0.9);
        assert!(raycast(&m, Vec3::new(3., 0., 5.), -Vec3::Z).is_none());
        let (q, n) = closest(&m, Vec3::new(0.2, 0.1, 3.0));
        assert!(q.distance(Vec3::new(0.2, 0.1, 0.0)) < 1e-5 && n.z.abs() > 0.9);
        let (q, _) = closest(&m, Vec3::new(4.0, 0.0, 1.0)); // off the edge -> clamps to it
        assert!(q.distance(Vec3::new(1.0, 0.0, 0.0)) < 1e-5);
    }

    #[test]
    fn sample_is_even() {
        let s = [Vec3::ZERO, Vec3::X, Vec3::new(1., 1., 0.)]; // L-shape, length 2
        let p = sample(&s, 4);
        assert!(p[1].distance(Vec3::new(0.5, 0., 0.)) < 1e-5 && p[3].distance(Vec3::new(1., 0.5, 0.)) < 1e-5);
    }

    #[test]
    fn four_strokes_make_unit_square_grid() {
        let (a, b, c, d) = (Vec3::ZERO, Vec3::X, Vec3::new(1., 1., 0.), Vec3::Y);
        let edges = [(a, b), (c, d), (c, b), (a, d)]; // scrambled order + directions
        let line = |i: usize, n: usize| (0..=n).map(|k| edges[i].0.lerp(edges[i].1, k as f32 / n as f32)).collect();
        let g = four_grid(&edges, line, 3, 2);
        for j in 0..=2 {
            for i in 0..=3 {
                assert!(g[j][i].distance(Vec3::new(i as f32 / 3., j as f32 / 2., 0.)) < 1e-5);
            }
        }
    }

    #[test]
    fn two_strokes_bridge_with_align() {
        let a: Vec<Vec3> = (0..4).map(|i| Vec3::new(i as f32, 0., 0.)).collect();
        let b = align(&a, (0..4).map(|i| Vec3::new(3. - i as f32, 2., 0.)).collect());
        let g = bridge(&a, &b, 2);
        assert!(g[1][2].distance(Vec3::new(2., 1., 0.)) < 1e-5);
    }

    #[test]
    fn weld_shares_edge_and_fixes_winding() {
        let m = quad_mesh();
        let g1 = vec![vec![Vec3::new(-1., -1., 0.), Vec3::new(0., -1., 0.)], vec![Vec3::new(-1., 1., 0.), Vec3::new(0., 1., 0.)]];
        let g2 = vec![vec![Vec3::new(0., -1., 0.), Vec3::new(1., -1., 0.)], vec![Vec3::new(0., 1., 0.), Vec3::new(1., 1., 0.)]];
        let obj = to_obj(&[snap(&m, &g1), snap(&m, &g2)], 1e-3);
        assert_eq!(obj.lines().filter(|l| l.starts_with("v ")).count(), 6);
        assert_eq!(obj.lines().filter(|l| l.starts_with("f ")).count(), 2);
    }

    #[test]
    fn auto_patch_finds_the_four_cells_of_a_3x3_net() {
        let m = quad_mesh();
        let line = |a: Vec3, b: Vec3| -> Vec<Vec3> { (0..=40).map(|k| a.lerp(b, k as f32 / 40.0)).collect() };
        let mut st: Vec<Vec<Vec3>> = Vec::new();
        for c in [-0.5f32, 0.0, 0.5] {
            st.push(line(Vec3::new(-0.8, c, 0.0), Vec3::new(0.8, c, 0.0)));
            st.push(line(Vec3::new(c, -0.8, 0.0), Vec3::new(c, 0.8, 0.0)));
        }
        let refs: Vec<&[Vec3]> = st.iter().map(|s| s.as_slice()).collect();
        let a = auto_patches(&m, &refs, 0.05, 0.25);
        assert_eq!(a.patches.len(), 4);
        let obj = to_obj(&a.patches, 1e-3);
        assert_eq!(obj.lines().filter(|l| l.starts_with("v ")).count(), 25); // 5x5 welded grid
        assert_eq!(obj.lines().filter(|l| l.starts_with("f ")).count(), 16);
    }

    /// the "U + arc + centre line" case: two strokes meet twice and one wraps around a corner
    #[test]
    fn arc_u_and_centre_line_make_two_quads() {
        let m = quad_mesh();
        let line = |pts: &[(f32, f32)]| -> Vec<Vec3> {
            pts.windows(2).flat_map(|w| (0..30).map(move |k| Vec3::new(w[0].0, w[0].1, 0.).lerp(Vec3::new(w[1].0, w[1].1, 0.), k as f32 / 30.0))).collect()
        };
        let st = [
            line(&[(-0.8, 0.2), (0.8, 0.2)]),                                  // arc on top
            line(&[(-0.8, 0.2), (-0.8, -0.6), (0.8, -0.6), (0.8, 0.2)]),       // U: two sides in one stroke
            line(&[(0.0, 0.5), (0.0, -0.9)]),                                  // centre line
            line(&[(-0.9, 0.5), (0.9, 0.5)]),                                  // closes the top
            line(&[(-0.8, 0.5), (-0.8, 0.2)]),
            line(&[(0.8, 0.5), (0.8, 0.2)]),
        ];
        let refs: Vec<&[Vec3]> = st.iter().map(|s| s.as_slice()).collect();
        let a = auto_patches(&m, &refs, 0.03, 0.3);
        assert!(a.patches.len() >= 2, "got {} patches, {} skipped", a.patches.len(), a.skipped);
    }

    #[test]
    fn mirror_lands_on_the_surface() {
        let m = quad_mesh();
        let p = mirror_on_surface(&m, Vec3::new(0.3, 0.2, 0.01), 0, 0.0, 0.01);
        assert!((p.x + 0.3).abs() < 1e-5 && (p.z - 0.01).abs() < 1e-5);
    }
}

// ---------------------------------------------------------------- smart shapes

fn principal(cov: [[f32; 3]; 3], skip: Option<Vec3>) -> Vec3 {
    let mut v = Vec3::new(0.577, 0.5, 0.64);
    for _ in 0..60 {
        let mut w = Vec3::new(
            cov[0][0] * v.x + cov[0][1] * v.y + cov[0][2] * v.z,
            cov[1][0] * v.x + cov[1][1] * v.y + cov[1][2] * v.z,
            cov[2][0] * v.x + cov[2][1] * v.y + cov[2][2] * v.z,
        );
        if let Some(s) = skip {
            w -= s * s.dot(w); // deflate: stay orthogonal to the first axis
        }
        v = w.normalize_or(v);
    }
    v
}

/// Solve the n x n system a x = b (Gaussian elimination, partial pivoting).
fn solve<const N: usize>(mut a: [[f64; N]; N], mut b: [f64; N]) -> Option<[f64; N]> {
    for c in 0..N {
        let p = (c..N).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if a[p][c].abs() < 1e-12 {
            return None;
        }
        a.swap(c, p);
        b.swap(c, p);
        for r in c + 1..N {
            let f = a[r][c] / a[c][c];
            for k in c..N {
                a[r][k] -= f * a[c][k];
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = [0.0; N];
    for r in (0..N).rev() {
        x[r] = (b[r] - (r + 1..N).map(|k| a[r][k] * x[k]).sum::<f64>()) / a[r][r];
    }
    Some(x)
}

/// A hand-drawn arc/loop that is (nearly) a circle or ellipse -> clean closed ring (first == last).
/// None if it is not planar, not elliptic, not wrapped far enough around, or too wobbly.
pub fn fit_ring(pts: &[Vec3], n: usize) -> Option<Vec<Vec3>> {
    if pts.len() < 8 {
        return None;
    }
    let c = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
    let mut cov = [[0f32; 3]; 3];
    for p in pts {
        let d = *p - c;
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += d[i] * d[j];
            }
        }
    }
    let e1 = principal(cov, None);
    let e2 = principal(cov, Some(e1));
    let nrm = e1.cross(e2);
    let scale = (pts.iter().map(|p| (*p - c).length_squared()).sum::<f32>() / pts.len() as f32).sqrt();
    if scale < 1e-9 {
        return None;
    }
    let flat = (pts.iter().map(|p| (*p - c).dot(nrm).powi(2)).sum::<f32>() / pts.len() as f32).sqrt();
    if flat > 0.06 * scale {
        return None; // not planar (S-curve, spiral, ...)
    }
    let q: Vec<(f64, f64)> = pts.iter().map(|p| (((*p - c).dot(e1) / scale) as f64, ((*p - c).dot(e2) / scale) as f64)).collect();
    // conic A x^2 + B xy + C y^2 + D x + E y = 1 by least squares
    let (mut m, mut r) = ([[0f64; 5]; 5], [0f64; 5]);
    for &(x, y) in &q {
        let f = [x * x, x * y, y * y, x, y];
        for i in 0..5 {
            for j in 0..5 {
                m[i][j] += f[i] * f[j];
            }
            r[i] += f[i];
        }
    }
    let [a, b, cc, d, e] = solve(m, r)?;
    let det = 4.0 * a * cc - b * b;
    if det <= 1e-9 || a <= 0.0 {
        return None; // hyperbola / parabola
    }
    // centre, then the quadratic form on centred coordinates is M x.x = k
    let (x0, y0) = ((b * e - 2.0 * cc * d) / det, (b * d - 2.0 * a * e) / det);
    let k = 1.0 + a * x0 * x0 + b * x0 * y0 + cc * y0 * y0;
    if k <= 0.0 {
        return None;
    }
    // eigen-decompose [[a, b/2], [b/2, cc]] in closed form
    let th = 0.5 * b.atan2(a - cc);
    let (ct, st) = (th.cos(), th.sin());
    let l1 = a * ct * ct + b * ct * st + cc * st * st;
    let l2 = a * st * st - b * ct * st + cc * ct * ct;
    if l1 <= 0.0 || l2 <= 0.0 {
        return None;
    }
    let (r1, r2) = ((k / l1).sqrt(), (k / l2).sqrt());
    if r1.max(r2) > 4.0 * r1.min(r2) {
        return None; // needle, not a ring
    }
    // residual (relative to mean radius) and how far the stroke wraps around the centre
    let mean_r = 0.5 * (r1 + r2);
    let mut err = 0.0;
    let mut ang: Vec<f64> = Vec::with_capacity(q.len());
    for &(x, y) in &q {
        let (u, v) = ((x - x0) * ct + (y - y0) * st, -(x - x0) * st + (y - y0) * ct);
        err += ((u / r1).powi(2) + (v / r2).powi(2)).sqrt().ln().powi(2);
        ang.push(v.atan2(u));
    }
    if (err / q.len() as f64).sqrt() > 0.07 {
        return None;
    }
    let _ = mean_r;
    let mut sorted = ang.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let mut gap = sorted[0] + std::f64::consts::TAU - sorted[sorted.len() - 1];
    for w in sorted.windows(2) {
        gap = gap.max(w[1] - w[0]);
    }
    if std::f64::consts::TAU - gap < 0.6 * std::f64::consts::TAU {
        return None; // just an arc: leave it alone
    }
    // keep the drawn direction and starting angle
    let dir = if ang.windows(2).map(|w| { let mut d = w[1] - w[0]; while d > std::f64::consts::PI { d -= std::f64::consts::TAU } while d < -std::f64::consts::PI { d += std::f64::consts::TAU } d }).sum::<f64>() >= 0.0 { 1.0 } else { -1.0 };
    let t0 = ang[0];
    Some(
        (0..=n)
            .map(|i| {
                let t = t0 + dir * std::f64::consts::TAU * (i % n) as f64 / n as f64;
                let (u, v) = (r1 * t.cos(), r2 * t.sin());
                let (x, y) = (x0 + u * ct - v * st, y0 + u * st + v * ct);
                c + (e1 * x as f32 + e2 * y as f32) * scale
            })
            .collect(),
    )
}

pub fn is_closed(s: &[Vec3]) -> bool {
    let len: f32 = s.windows(2).map(|w| w[0].distance(w[1])).sum();
    s.len() > 3 && s[0].distance(s[s.len() - 1]) < 1e-3 * len.max(1e-9)
}

/// Like `align` for two closed rings: pick start point and direction that untwist the bridge.
pub fn align_ring(a: &[Vec3], b: &[Vec3]) -> Vec<Vec3> {
    let n = a.len() - 1;
    let mut best = (f32::MAX, 0usize, 1isize);
    for dir in [1isize, -1] {
        for k in 0..n {
            let cost: f32 = (0..n).map(|i| a[i].distance_squared(b[(k as isize + dir * i as isize).rem_euclid(n as isize) as usize])).sum();
            if cost < best.0 {
                best = (cost, k, dir);
            }
        }
    }
    let mut out: Vec<Vec3> = (0..n).map(|i| b[(best.1 as isize + best.2 * i as isize).rem_euclid(n as isize) as usize]).collect();
    out.push(out[0]);
    out
}

#[cfg(test)]
mod ring_tests {
    use super::*;

    /// noisy 270 deg arc of a tilted ellipse (a=2,b=1.4) -> closed ring that matches it
    fn arc(sweep: f32) -> Vec<Vec3> {
        let (u, v) = (Vec3::new(1., 0.2, 0.).normalize(), Vec3::new(-0.2, 1., 0.5).normalize());
        (0..60)
            .map(|i| {
                let t = 0.4 + sweep * i as f32 / 59.0;
                let wob = 1.0 + 0.02 * (i as f32 * 1.7).sin();
                Vec3::new(5., 1., 2.) + (u * t.cos() * 2.0 + v * t.sin() * 1.4) * wob
            })
            .collect()
    }

    #[test]
    fn arc_becomes_ring() {
        let r = fit_ring(&arc(4.7), 48).expect("recognised");
        assert_eq!(r.len(), 49);
        assert!(is_closed(&r));
        let c = r.iter().copied().sum::<Vec3>() / r.len() as f32;
        assert!(c.distance(Vec3::new(5., 1., 2.)) < 0.1, "{c}");
        let rad: Vec<f32> = r.iter().map(|p| p.distance(c)).collect();
        assert!(rad.iter().cloned().fold(0., f32::max) > 1.9 && rad.iter().cloned().fold(9., f32::min) < 1.5);
    }

    #[test]
    fn short_arc_and_line_stay() {
        assert!(fit_ring(&arc(2.0), 48).is_none());
        let line: Vec<Vec3> = (0..30).map(|i| Vec3::new(i as f32, 0.1 * (i as f32).sin(), 0.)).collect();
        assert!(fit_ring(&line, 48).is_none());
    }

    #[test]
    fn rings_untwist() {
        let a = fit_ring(&arc(6.2), 16).unwrap();
        let mut b: Vec<Vec3> = a[..16].iter().map(|p| *p + Vec3::Z).collect();
        b.rotate_left(5);
        b.push(b[0]); // closed again, but started elsewhere
        let al = align_ring(&a, &b);
        assert!(a.iter().zip(&al).all(|(p, q)| (p.distance(*q) - 1.0).abs() < 1e-3));
    }
}

#[cfg(test)]
mod bvh_tests {
    use super::*;

    fn sphere(n: usize, m: usize) -> Mesh {
        let mut pos = vec![];
        for i in 0..=n {
            let t = std::f32::consts::PI * i as f32 / n as f32;
            for j in 0..m {
                let p = std::f32::consts::TAU * j as f32 / m as f32;
                pos.push(Vec3::new(t.sin() * p.cos(), t.cos(), t.sin() * p.sin()) * (1.0 + 0.1 * (3.0 * p).sin()));
            }
        }
        let mut idx = vec![];
        for i in 0..n {
            for j in 0..m {
                let (a, b) = ((i * m + j) as u32, (i * m + (j + 1) % m) as u32);
                idx.extend([a, b, b + m as u32, a, b + m as u32, a + m as u32]);
            }
        }
        Mesh::new(pos, idx)
    }

    #[test]
    fn bvh_matches_brute_force() {
        let m = sphere(24, 32);
        for k in 0..60 {
            let f = k as f32;
            let p = Vec3::new((f * 0.7).sin() * 2.0, (f * 1.3).cos() * 1.8, (f * 0.37).sin() * 2.2);
            let brute = m.idx.chunks_exact(3).map(|t| closest_on_tri(p, m.pos[t[0] as usize], m.pos[t[1] as usize], m.pos[t[2] as usize])).min_by(|a, b| a.distance(p).total_cmp(&b.distance(p))).unwrap();
            assert!((closest(&m, p).0.distance(p) - brute.distance(p)).abs() < 1e-5);
            let d = (Vec3::ZERO - p + Vec3::new(0.1 * f.sin(), 0.05, 0.0)).normalize();
            let bt = m.idx.chunks_exact(3).filter_map(|t| ray_tri(p, d, m.pos[t[0] as usize], m.pos[t[1] as usize], m.pos[t[2] as usize])).min_by(|a, b| a.total_cmp(b));
            assert_eq!(raycast(&m, p, d).map(|h| h.0), bt);
        }
    }
}

/// Closest point to p on the polylines `others`, if within r.
pub fn nearest_on_strokes(p: Vec3, others: &[&[Vec3]], r: f32) -> Option<Vec3> {
    let mut best: Option<(f32, Vec3)> = None;
    for s in others {
        for w in s.windows(2) {
            let ab = w[1] - w[0];
            let t = if ab.length_squared() > 0.0 { ((p - w[0]).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
            let q = w[0] + ab * t;
            let d = q.distance(p);
            if d < r && best.map_or(true, |b| d < b.0) {
                best = Some((d, q));
            }
        }
    }
    best.map(|b| b.1)
}

#[cfg(test)]
mod snap_tests {
    use super::*;
    #[test]
    fn end_finds_nearby_stroke() {
        let wall = [Vec3::new(0., -5., 0.), Vec3::new(0., 5., 0.)];
        let q = nearest_on_strokes(Vec3::new(-0.3, 1.0, 0.), &[&wall], 0.5).unwrap();
        assert!(q.distance(Vec3::new(0., 1., 0.)) < 1e-6);
        assert!(nearest_on_strokes(Vec3::new(-2., 1.0, 0.), &[&wall], 0.5).is_none());
    }
}

// ---------------------------------------------------------------- ring projection
/// Put an ideal planar ring onto the surface the stroke was drawn on, or None if that does not work.
/// Each ring point is dropped along the surface normal of the nearest drawn sample (the same layer the
/// user saw: it cannot jump from an eyelid onto the eyeball behind it), then the result is verified:
/// points must not travel far, neighbours must not jump apart, and the ring must stay near the stroke.
pub fn snap_ring(m: &Mesh, ideal: &[Vec3], drawn: &[Vec3], eye: Vec3, lift: f32) -> Option<Vec<Vec3>> {
    let n = ideal.len() - 1;
    let c = ideal[..n].iter().copied().sum::<Vec3>() / n as f32;
    let rad = ideal[..n].iter().map(|p| p.distance(c)).sum::<f32>() / n as f32;
    let up: Vec<Vec3> = drawn
        .iter()
        .map(|&s| {
            let (q, nn) = closest(m, s);
            if nn.dot(eye - q) >= 0.0 { nn } else { -nn }
        })
        .collect();
    let mut out: Vec<Vec3> = Vec::with_capacity(ideal.len());
    for (k, &p) in ideal.iter().enumerate() {
        if k == n {
            out.push(out[0]); // exact closure
            break;
        }
        let j = (0..drawn.len()).min_by(|&a, &b| drawn[a].distance_squared(p).total_cmp(&drawn[b].distance_squared(p)))?;
        let (o, d) = (p + up[j] * rad * 0.5, -up[j]);
        let (t, nn) = raycast(m, o, d)?;
        let q = o + d * t;
        if q.distance(p) > 0.18 * rad {
            return None; // ellipse does not lie on this surface
        }
        out.push(q + nn * lift);
    }
    // continuity: the ring must be as smooth as the ideal one
    let step = ideal.windows(2).map(|w| w[0].distance(w[1])).fold(0.0, f32::max);
    if out.windows(2).any(|w| w[0].distance(w[1]) > 2.5 * step + 0.05 * rad) {
        return None;
    }
    // faithfulness: every drawn sample stays near the ring
    let ring: [&[Vec3]; 1] = [&out];
    let dev: Vec<f32> = drawn.iter().map(|&s| nearest_on_strokes(s, &ring, f32::MAX).map_or(f32::MAX, |q| q.distance(s))).collect();
    let (mean, max) = (dev.iter().sum::<f32>() / dev.len() as f32, dev.iter().cloned().fold(0.0, f32::max));
    (mean < 0.06 * rad && max < 0.2 * rad).then_some(out)
}

// ---------------------------------------------------------------- welded view of the patches (relax brush)
pub struct Weld {
    pub verts: Vec<Vec3>,
    pub nrm: Vec<Vec3>,
    ids: Vec<Vec<u32>>, // per patch, per grid point -> welded vertex
    nbr: Vec<Vec<u32>>,
    border: Vec<bool>,
}

impl Weld {
    pub fn build(patches: &[Patch], tol: f32) -> Weld {
        let tol = tol.max(1e-9);
        let cell = |p: Vec3| ((p.x / tol).floor() as i32, (p.y / tol).floor() as i32, (p.z / tol).floor() as i32);
        let (mut verts, mut nrm): (Vec<Vec3>, Vec<Vec3>) = (vec![], vec![]);
        let mut grid: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        let mut ids = Vec::with_capacity(patches.len());
        for pa in patches {
            ids.push(
                pa.pts
                    .iter()
                    .zip(&pa.nrm)
                    .map(|(&p, &nn)| {
                        let (cx, cy, cz) = cell(p);
                        for dx in -1..=1 {
                            for dy in -1..=1 {
                                for dz in -1..=1 {
                                    for &id in grid.get(&(cx + dx, cy + dy, cz + dz)).into_iter().flatten() {
                                        if verts[id as usize].distance(p) <= tol {
                                            return id;
                                        }
                                    }
                                }
                            }
                        }
                        verts.push(p);
                        nrm.push(nn);
                        grid.entry((cx, cy, cz)).or_default().push(verts.len() as u32 - 1);
                        verts.len() as u32 - 1
                    })
                    .collect::<Vec<u32>>(),
            );
        }
        let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
        for (pa, id) in patches.iter().zip(&ids) {
            let w = pa.nu + 1;
            for j in 0..=pa.nv {
                for i in 0..=pa.nu {
                    let a = id[j * w + i];
                    for (ok, b) in [(i < pa.nu, id.get(j * w + i + 1)), (j < pa.nv, id.get((j + 1) * w + i))] {
                        if let (true, Some(&b)) = (ok, b) {
                            if a != b {
                                *edges.entry((a.min(b), a.max(b))).or_default() += 1;
                            }
                        }
                    }
                }
            }
        }
        // an edge on a patch rim is used by one cell only; interior edges by two
        let (mut nbr, mut border) = (vec![vec![]; verts.len()], vec![false; verts.len()]);
        let mut use_count: HashMap<(u32, u32), u32> = HashMap::new();
        for (pa, id) in patches.iter().zip(&ids) {
            let w = pa.nu + 1;
            for j in 0..pa.nv {
                for i in 0..pa.nu {
                    let q = [id[j * w + i], id[j * w + i + 1], id[(j + 1) * w + i + 1], id[(j + 1) * w + i]];
                    for k in 0..4 {
                        let (a, b) = (q[k], q[(k + 1) % 4]);
                        if a != b {
                            *use_count.entry((a.min(b), a.max(b))).or_default() += 1;
                        }
                    }
                }
            }
        }
        for (&(a, b), _) in &edges {
            nbr[a as usize].push(b);
            nbr[b as usize].push(a);
        }
        for (&(a, b), &c) in &use_count {
            if c == 1 {
                border[a as usize] = true;
                border[b as usize] = true;
            }
        }
        Weld { verts, nrm, ids, nbr, border }
    }

    /// One relax pass around `center`: vertices slide toward the mean of their neighbours (tangentially),
    /// weighted by a smooth falloff, and are put back on the surface. Returns whether anything moved.
    pub fn relax(&mut self, m: &Mesh, center: Vec3, radius: f32, strength: f32, pin_border: bool) -> bool {
        let src = self.verts.clone(); // Jacobi: every vertex reads the old positions
        let mut moved = false;
        for v in 0..src.len() {
            let d = src[v].distance(center);
            if d >= radius || self.nbr[v].is_empty() || (pin_border && self.border[v]) {
                continue;
            }
            let f = 1.0 - d / radius;
            let avg = self.nbr[v].iter().map(|&k| src[k as usize]).sum::<Vec3>() / self.nbr[v].len() as f32;
            let mut delta = avg - src[v];
            delta -= self.nrm[v] * delta.dot(self.nrm[v]); // no shrinking along the normal
            let (q, nn) = closest(m, src[v] + delta * (f * f * strength));
            self.verts[v] = q;
            self.nrm[v] = nn;
            moved = true;
        }
        moved
    }

    pub fn write_back(&self, patches: &mut [Patch]) {
        for (pa, id) in patches.iter_mut().zip(&self.ids) {
            for (k, &v) in id.iter().enumerate() {
                pa.pts[k] = self.verts[v as usize];
                pa.nrm[k] = self.nrm[v as usize];
            }
        }
    }
}

#[cfg(test)]
mod brush_tests {
    use super::*;

    fn flat() -> Mesh {
        Mesh::new(vec![Vec3::new(-3., -3., 0.), Vec3::new(3., -3., 0.), Vec3::new(3., 3., 0.), Vec3::new(-3., 3., 0.)], vec![0, 1, 2, 0, 2, 3])
    }

    /// two 2x2 patches side by side sharing a column; one interior vertex is pushed off
    fn patches(bad: Vec3) -> Vec<Patch> {
        let mk = |x0: f32, wobble: Option<Vec3>| {
            let mut pts: Vec<Vec3> = (0..=2).flat_map(|j| (0..=2).map(move |i| Vec3::new(x0 + i as f32, j as f32, 0.))).collect();
            if let Some(b) = wobble {
                pts[4] = b;
            }
            Patch { nu: 2, nv: 2, nrm: vec![Vec3::Z; 9], pts }
        };
        vec![mk(-2., Some(bad)), mk(0., None)]
    }

    #[test]
    fn weld_shares_the_seam_and_relax_pulls_the_bump_back() {
        let m = flat();
        let mut ps = patches(Vec3::new(-0.6, 1.5, 0.)); // centre should be (-1, 1)
        let mut w = Weld::build(&ps, 1e-3);
        assert_eq!(w.verts.len(), 15); // 18 grid points, 3 shared on the seam
        for _ in 0..30 {
            w.relax(&m, Vec3::new(-1., 1., 0.), 3.0, 0.8, true);
        }
        w.write_back(&mut ps);
        assert!(ps[0].pts[4].distance(Vec3::new(-1., 1., 0.)) < 0.01, "{}", ps[0].pts[4]);
        assert_eq!(ps[0].pts[0], Vec3::new(-2., 0., 0.)); // rim pinned
        assert_eq!(ps[0].pts[2], ps[1].pts[0]); // seam still welded
    }

    #[test]
    fn rim_vertices_move_only_when_unpinned() {
        let m = flat();
        let mut w = Weld::build(&patches(Vec3::new(-1., 1., 0.)), 1e-3);
        let before = w.verts.clone();
        w.relax(&m, Vec3::new(0., 1., 0.), 5.0, 1.0, true);
        let rim_moved = (0..w.verts.len()).any(|v| w.border[v] && w.verts[v] != before[v]);
        assert!(!rim_moved);
        w.relax(&m, Vec3::new(0., 1., 0.), 5.0, 1.0, false);
        assert!((0..w.verts.len()).any(|v| w.border[v] && w.verts[v] != before[v]));
    }
}

#[cfg(test)]
mod ring_snap_tests {
    use super::*;

    fn sphere() -> Mesh {
        let (n, mm) = (40usize, 64usize);
        let mut pos = vec![];
        for i in 0..=n {
            let t = std::f32::consts::PI * i as f32 / n as f32;
            for j in 0..mm {
                let p = std::f32::consts::TAU * j as f32 / mm as f32;
                pos.push(Vec3::new(t.sin() * p.cos(), t.cos(), t.sin() * p.sin()));
            }
        }
        let mut idx = vec![];
        for i in 0..n {
            for j in 0..mm {
                let (a, b) = ((i * mm + j) as u32, (i * mm + (j + 1) % mm) as u32);
                idx.extend([a, b, b + mm as u32, a, b + mm as u32, a + mm as u32]);
            }
        }
        Mesh::new(pos, idx)
    }

    #[test]
    fn ring_around_a_sphere_lands_on_it_and_a_floating_ring_is_refused() {
        let m = sphere();
        let eye = Vec3::new(0., 0.4, 6.);
        let drawn: Vec<Vec3> = (0..50).map(|i| { let t = 0.3 + 5.2 * i as f32 / 49.0; Vec3::new(0.7 * t.cos(), 0.7, 0.7 * t.sin()) * 1.0 }).map(|p| { let (q, _) = closest(&m, p); q }).collect();
        let ideal = fit_ring(&drawn, 48).expect("fits");
        let ring = snap_ring(&m, &ideal, &drawn, eye, 0.001).expect("projects");
        assert!(is_closed(&ring));
        assert!(ring.iter().all(|p| (p.length() - 1.0).abs() < 0.01));
        let far: Vec<Vec3> = ideal.iter().map(|p| *p * 1.0 + Vec3::new(0., 0.8, 0.)).collect();
        assert!(snap_ring(&m, &far, &drawn, eye, 0.001).is_none());
    }
}
