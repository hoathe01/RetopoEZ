//! RetopoEZ: load OBJ, draw strokes on the surface, pick 2/4 strokes -> snapped quad grid, export OBJ.
//! LMB drag draw | LMB click select stroke | RMB or Alt+LMB orbit | MMB pan | wheel zoom | Ctrl+Z undo | Del delete.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod geom;
mod gpu;
mod theme;
use gpu::{Frame, Geo, Gpu, Seg, Vertex};
use geom::{Mesh, Patch};
use eframe::egui;
use eframe::egui_wgpu;
use glam::{Mat4, Vec3};
use std::{fs, hash::{Hash, Hasher}, net::TcpListener, path::PathBuf, sync::{atomic::{AtomicBool, Ordering}, Arc}, time::Duration};

static FOCUS: AtomicBool = AtomicBool::new(false);
static OPEN: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None); // file a second launch asked us to open
const LOCK_ADDR: &str = "127.0.0.1:47831"; // bound = a viewer is already running

// ------------------------------------------------------------------ App
struct Stroke {
    pts: Vec<Vec3>,
    id: u32,
    partner: Option<u32>, // mirrored twin when symmetry was on
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Grid,
    Auto,
    Mirror,
    Strokes,
    Brush,
    Export,
}

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Draw,
    Brush,
    Edit,
}

#[derive(Clone, Copy, PartialEq)]
enum BrushKind {
    Relax,
    Grab,
}

/// Pie menu slices, clockwise from the top.
const PIE: [&str; 8] = ["PENCIL", "BRUSH", "GRID", "LIVE", "MIRROR", "EXPORT", "UNDO", "EDIT"];

const LOW_DEFAULT: [f32; 3] = [0.03, 0.14, 0.42]; // deep sea blue

/// Drawing over selected faces: the region's outline plus the new strokes are re-quadded, the rest stays.
struct Reflow {
    orig: Vec<Patch>, // patches before the first stroke (cancel / undo back to it)
    base: Vec<Patch>, // everything outside the region (cells of touched patches split into 1x1)
    bnd: Vec<Vec<Vec3>>,
    strokes: Vec<Vec<Vec3>>,
}

struct App {
    path: String,
    status: String,
    mesh: Option<Mesh>,
    sig: Option<(u32, u32)>, // mesh identity: vertex count + diag bits (keys the saved session)
    upload: Option<Arc<(Vec<Vertex>, Vec<u32>)>>,
    strokes: Vec<Stroke>,
    next_id: u32,
    cur: Vec<Vec3>,
    line: Option<(egui::Pos2, egui::Pos2)>, // Shift-drag: straight line, screen space until release
    selected: Vec<u32>,
    patches: Vec<Patch>,
    live_patches: Vec<Patch>, // smart auto, recomputed whenever strokes change
    live: bool,
    dirty_auto: bool,
    snap_ends: bool, // stroke ends jump onto a nearby stroke
    snap_r: f32,     // 16 px in world units, set by the viewport
    cur_m: Vec<Vec3>, // live mirror of the stroke being drawn
    low_a: f32,
    low_lift: f32, // per-mille of mesh size the low-poly floats toward the camera
    patch_log: Vec<usize>, // patches added per action, for Undo
    sym: bool,
    axis: usize,
    plane: f32,
    snap: f32, // auto patch: stroke touch distance, per-mille of mesh size
    edge: f32, // auto patch: target quad edge, percent of mesh size
    u: u32,
    v: u32,
    weld: f32, // per-mille of mesh size
    smart: bool, // recognise circles/ellipses
    low: [f32; 3], // colour of the retopo (low) mesh
    tab: Tab,
    book: bool,
    tool: Tool,
    pie: Option<egui::Pos2>, // Space held: radial menu centred here
    beauty: f32,             // stroke clean-up strength, 0 = off
    fsel: std::collections::HashSet<usize>, // Edit tool: selected faces (index into `faces`)
    fsel_n: usize,                          // face count the selection belongs to
    faces: Vec<[Vec3; 4]>,                  // every quad of every patch, rebuilt when the patches change
    faces_key: (usize, usize, u32),
    bk: BrushKind,
    reflow: Option<Reflow>,
    brush_hard: f32,
    grab: Option<(Vec<f32>, Vec<f32>, f32)>, // Grab brush: weights, mirrored weights, world units per pixel
    tab_prev: Tool,
    tab_t: Option<std::time::Instant>,
    brush: Option<geom::Weld>, // welded view of the patches while a relax stroke is in progress
    relax_undo: Vec<Vec<(Vec<Vec3>, Vec<Vec3>)>>,
    brush_px: f32,
    brush_strength: f32,
    pin_border: bool,
    patch_rev: u32,
    geo_fp: Option<u64>,
    saved_fp: Option<u64>,
    saved: Vec<u8>,
    last_save: std::time::Instant,
    mtime: Option<std::time::SystemTime>,
    last_load: std::time::Instant,
    auto_note: String,
    target: Vec3,
    yaw: f32,
    pitch: f32,
    dist: f32,
}

/// Where the viewer remembers the last opened file.
fn config_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join("RetopoEZ"))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/retopoez")))
        .unwrap_or_else(std::env::temp_dir)
}

fn exchange_dir() -> PathBuf {
    std::env::temp_dir().join("retopoez") // same folder retopoez.py uses in Maya
}

// ---- session file (binary, little endian): strokes, patches and settings survive closing the viewer
fn put_u(b: &mut Vec<u8>, x: u32) {
    b.extend(x.to_le_bytes());
}
fn put_f(b: &mut Vec<u8>, x: f32) {
    b.extend(x.to_le_bytes());
}
fn put_v(b: &mut Vec<u8>, v: &[Vec3]) {
    put_u(b, v.len() as u32);
    for p in v {
        p.to_array().iter().for_each(|&c| put_f(b, c));
    }
}

struct Rd<'a>(&'a [u8]);
impl Rd<'_> {
    fn u(&mut self) -> Option<u32> {
        let (h, t) = self.0.split_first_chunk::<4>()?;
        self.0 = t;
        Some(u32::from_le_bytes(*h))
    }
    fn f(&mut self) -> Option<f32> {
        self.u().map(f32::from_bits)
    }
    fn v(&mut self) -> Option<Vec<Vec3>> {
        let n = self.u()? as usize;
        (n <= self.0.len() / 12).then_some(())?; // corrupt length
        (0..n).map(|_| Some(Vec3::new(self.f()?, self.f()?, self.f()?))).collect()
    }
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, lock: TcpListener) -> Self {
        // a later launch pings this socket: bring the window to front instead of opening another
        let ctx = cc.egui_ctx.clone();
        std::thread::spawn(move || {
            for stream in lock.incoming().flatten() {
                // a second launch may send the file it was asked to open (Open with / drag onto the exe)
                let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
                let mut line = String::new();
                let _ = std::io::BufRead::read_line(&mut std::io::BufReader::new(stream), &mut line);
                if !line.trim().is_empty() {
                    *OPEN.lock().unwrap() = Some(line.trim().to_string());
                }
                FOCUS.store(true, Ordering::Relaxed);
                ctx.request_repaint();
            }
        });
        theme::install(&cc.egui_ctx);
        let rs = cc.wgpu_render_state.as_ref().expect("wgpu backend required");
        rs.renderer.write().callback_resources.insert(Gpu::new(rs));
        // file given on the command line, else the one used last time, else the Maya exchange file
        let path = std::env::args()
            .nth(1)
            .or_else(|| fs::read_to_string(config_dir().join("last.txt")).ok().map(|s| s.trim().to_string()).filter(|s| PathBuf::from(s).is_file()))
            .unwrap_or_else(|| exchange_dir().join("mesh.obj").to_string_lossy().into());
        let mut a = App {
            path,
            status: String::new(),
            mesh: None,
            sig: None,
            upload: None,
            strokes: vec![],
            next_id: 0,
            cur: vec![],
            line: None,
            selected: vec![],
            patches: vec![],
            live_patches: vec![],
            live: false,
            dirty_auto: false,
            snap_ends: true,
            snap_r: 0.0,
            cur_m: vec![],
            low_a: 0.75,
            low_lift: 10.0,
            patch_log: vec![],
            sym: false,
            axis: 0,
            plane: 0.0,
            snap: 10.0,
            edge: 3.0,
            u: 8,
            v: 4,
            weld: 0.5,
            smart: true,
            low: LOW_DEFAULT,
            tab: Tab::Strokes,
            book: false, // viewport only; P opens the toolbar and panel
            tool: Tool::Draw,
            pie: None,
            beauty: 0.6,
            fsel: Default::default(),
            fsel_n: 0,
            faces: vec![],
            faces_key: (usize::MAX, 0, 0),
            bk: BrushKind::Relax,
            reflow: None,
            brush_hard: 0.3,
            grab: None,
            tab_prev: Tool::Draw,
            tab_t: None,
            brush: None,
            relax_undo: vec![],
            brush_px: 70.0,
            brush_strength: 0.4,
            pin_border: true,
            patch_rev: 0,
            geo_fp: None,
            saved_fp: None,
            saved: vec![],
            last_save: std::time::Instant::now(),
            mtime: None,
            last_load: std::time::Instant::now(),
            auto_note: String::new(),
            target: Vec3::ZERO,
            yaw: 0.6,
            pitch: 0.4,
            dist: 3.0,
        };
        a.load();
        a
    }

    fn open_path(&mut self, p: String) {
        self.path = p;
        self.load();
    }

    fn session_path(&self) -> Option<PathBuf> {
        let (n, d) = self.sig?;
        Some(PathBuf::from(&self.path).with_file_name(format!("session_{n}_{d:08x}.rtz")))
    }

    fn session_bytes(&self) -> Vec<u8> {
        let mut b = b"RTZ3".to_vec();
        put_u(&mut b, self.next_id);
        for x in [self.sym as u32, self.axis as u32, self.u, self.v, self.smart as u32, self.live as u32, self.snap_ends as u32] {
            put_u(&mut b, x);
        }
        for x in [self.plane, self.weld, self.snap, self.edge, self.low[0], self.low[1], self.low[2], self.low_a] {
            put_f(&mut b, x);
        }
        put_u(&mut b, self.strokes.len() as u32);
        for s in &self.strokes {
            put_u(&mut b, s.id);
            put_u(&mut b, s.partner.unwrap_or(u32::MAX));
            put_v(&mut b, &s.pts);
        }
        put_u(&mut b, self.patches.len() as u32);
        for p in &self.patches {
            put_u(&mut b, p.nu as u32);
            put_u(&mut b, p.nv as u32);
            put_v(&mut b, &p.pts);
            put_v(&mut b, &p.nrm);
        }
        put_u(&mut b, self.patch_log.len() as u32);
        for &n in &self.patch_log {
            put_u(&mut b, n as u32);
        }
        b
    }

    fn autosave(&mut self) {
        let (Some(path), true) = (self.session_path(), self.mesh.is_some()) else { return };
        let fp = self.session_fp();
        if self.saved_fp == Some(fp) {
            return; // nothing changed: skip serialising
        }
        self.saved_fp = Some(fp);
        let b = self.session_bytes();
        if b != self.saved && fs::write(path, &b).is_ok() {
            self.saved = b;
        }
    }

    fn restore(&mut self) -> Option<()> {
        let bytes = fs::read(self.session_path()?).ok()?;
        let mut r = Rd(bytes.strip_prefix(b"RTZ3")?);
        let next_id = r.u()?;
        let [sym, axis, u, v, smart, live, snap_ends] = [r.u()?, r.u()?, r.u()?, r.u()?, r.u()?, r.u()?, r.u()?];
        let [plane, weld, snap, edge, l0, l1, l2, la] = [r.f()?, r.f()?, r.f()?, r.f()?, r.f()?, r.f()?, r.f()?, r.f()?];
        let mut strokes = vec![];
        for _ in 0..r.u()? {
            let (id, p) = (r.u()?, r.u()?);
            strokes.push(Stroke { id, partner: (p != u32::MAX).then_some(p), pts: r.v()? });
        }
        let mut patches = vec![];
        for _ in 0..r.u()? {
            let (nu, nv) = (r.u()? as usize, r.u()? as usize);
            let (pts, nrm) = (r.v()?, r.v()?);
            (pts.len() == (nu + 1) * (nv + 1) && nrm.len() == pts.len()).then_some(())?;
            patches.push(Patch { nu, nv, pts, nrm });
        }
        let log = (0..r.u()?).map(|_| r.u().map(|x| x as usize)).collect::<Option<Vec<_>>>()?;
        (self.next_id, self.sym, self.axis, self.u, self.v, self.smart) = (next_id, sym != 0, (axis as usize).min(2), u.clamp(1, 128), v.clamp(1, 128), smart != 0);
        (self.plane, self.weld, self.snap, self.edge, self.low, self.low_a) = (plane, weld, snap, edge, [l0, l1, l2], la);
        (self.live, self.snap_ends, self.dirty_auto) = (live != 0, snap_ends != 0, live != 0);
        self.status = format!("session restored: {} strokes, {} patches", strokes.len(), patches.len());
        (self.strokes, self.patches, self.patch_log) = (strokes, patches, log);
        self.saved = bytes;
        Some(())
    }

    fn load(&mut self) {
        let p = PathBuf::from(&self.path);
        self.mtime = fs::metadata(&p).and_then(|m| m.modified()).ok();
        self.last_load = std::time::Instant::now();
        match geom::load_obj(&p) {
            Ok(m) => {
                let sig = (m.pos.len() as u32, m.diag.to_bits());
                if self.path != exchange_dir().join("mesh.obj").to_string_lossy() {
                    let _ = fs::create_dir_all(config_dir());
                    let _ = fs::write(config_dir().join("last.txt"), &self.path);
                }
                self.upload = Some(Arc::new((gpu::gpu_vertices(&m), m.idx.clone())));
                if self.sig == Some(sig) {
                    self.mesh = Some(m); // Maya re-exported the same mesh: keep the work
                    return;
                }
                self.autosave(); // the previous mesh's session goes to its own file
                self.status = format!("{} verts, {} tris", m.pos.len(), m.idx.len() / 3);
                self.target = m.center;
                self.dist = m.diag * 1.2;
                (self.strokes, self.selected, self.patches, self.patch_log, self.live_patches) = Default::default();
                self.next_id = 0;
                self.sig = Some(sig);
                self.mesh = Some(m);
                self.saved.clear();
                self.saved_fp = None;
                self.restore();
            }
            Err(e) => self.status = format!("load failed: {e}"),
        }
    }

    fn grid(&mut self, four: bool) {
        let Some(m) = self.mesh.as_ref() else { return };
        let need = if four { 4 } else { 2 };
        if self.selected.len() != need {
            self.status = format!("click {need} strokes to select them (selected: {})", self.selected.len());
            return;
        }
        let (u, v) = (self.u as usize, self.v as usize);
        let build = |ids: &[u32]| -> Option<Patch> {
            let st: Vec<&Vec<Vec3>> = ids
                .iter()
                .map(|id| self.strokes.iter().find(|s| s.id == *id).map(|s| &s.pts))
                .collect::<Option<_>>()?;
            let grid = if four {
                let ends: Vec<(Vec3, Vec3)> = st.iter().map(|s| (s[0], *s.last().unwrap())).collect();
                geom::four_grid(&ends, |i, n| geom::sample(st[i], n), u, v)
            } else {
                let a = geom::sample(st[0], u);
                let b = geom::sample(st[1], u);
                let b = if st.iter().all(|s| geom::is_closed(s)) { geom::align_ring(&a, &b) } else { geom::align(&a, b) };
                geom::bridge(&a, &b, v)
            };
            Some(geom::snap(m, &grid))
        };
        let mut added = 0;
        if let Some(p) = build(&self.selected) {
            self.patches.push(p);
            added += 1;
        }
        if self.sym {
            let twins: Option<Vec<u32>> = self
                .selected
                .iter()
                .map(|id| self.strokes.iter().find(|s| s.id == *id).and_then(|s| s.partner))
                .collect();
            if let Some(t) = twins {
                let (mut a, mut b) = (t.clone(), self.selected.clone());
                a.sort_unstable();
                b.sort_unstable();
                if a != b {
                    if let Some(p) = build(&t) {
                        self.patches.push(p);
                        added += 1;
                    }
                }
            }
        }
        self.patch_log.push(added);
        self.status = format!("{added} patch(es) added");
    }

    fn auto(&mut self) {
        if self.live {
            // freeze: the live result becomes ordinary patches
            let n = self.live_patches.len();
            self.patches.append(&mut self.live_patches);
            self.patch_log.push(n);
            self.live = false;
            self.status = format!("live auto frozen into {n} patches");
            return;
        }
        let Some(m) = self.mesh.as_ref() else { return };
        let use_sel = self.selected.len() >= 2;
        let st: Vec<&[Vec3]> = self
            .strokes
            .iter()
            .filter(|s| !use_sel || self.selected.contains(&s.id))
            .map(|s| s.pts.as_slice())
            .collect();
        if st.len() < 4 {
            self.status = "auto patch needs a net of at least 4 crossing strokes".into();
            return;
        }
        let r = geom::auto_patches(m, &st, m.diag * self.snap * 1e-3, m.diag * self.edge * 0.01);
        let n = r.patches.len();
        self.patches.extend(r.patches);
        self.patch_log.push(n);
        self.auto_note = format!("{n} patches built, {} regions skipped (not 4-sided)", r.skipped);
        self.status = format!("auto patch: {}", self.auto_note);
    }

    fn undo_patch(&mut self) {
        let n = self.patch_log.pop().unwrap_or(1);
        self.patches.truncate(self.patches.len().saturating_sub(n));
    }

    fn undo_stroke(&mut self) {
        self.dirty_auto = true;
        if let Some(s) = self.strokes.pop() {
            if let Some(p) = s.partner {
                if self.strokes.last().map_or(false, |l| l.id == p) {
                    self.strokes.pop();
                }
            }
            self.selected.clear();
        }
    }

    fn push_stroke(&mut self, pts: Vec<Vec3>, live_twin: Option<Vec<Vec3>>) {
        if self.reflow.is_some() || !self.fsel.is_empty() {
            return self.reflow_stroke(pts);
        }
        self.dirty_auto = true;
        let id = self.next_id;
        self.next_id += 2;
        let mut partner = None;
        let mut twin = None;
        if let (true, Some(m)) = (self.sym, &self.mesh) {
            let on_plane = pts.iter().all(|p| (p[self.axis] - self.plane).abs() < m.diag * 0.003);
            if !on_plane {
                let mp = live_twin.filter(|t| t.len() == pts.len()).unwrap_or_else(|| pts.iter().map(|&p| geom::mirror_on_surface(m, p, self.axis, self.plane, m.diag * 2e-4)).collect());
                twin = Some(Stroke { pts: mp, id: id + 1, partner: Some(id) });
                partner = Some(id + 1);
            }
        }
        self.strokes.push(Stroke { pts, id, partner });
        self.strokes.extend(twin);
    }

    /// Hand-drawn loop/arc that is a circle or ellipse -> clean ring lying on the surface.
    /// Refused (None) whenever the ring would not sit on the surface or would stray from the stroke.
    fn ring_from(&self, s: &[Vec3]) -> Result<Vec<Vec3>, &'static str> {
        let m = self.mesh.as_ref().ok_or("no mesh")?;
        let ideal = geom::fit_ring(s, 64).ok_or("")?; // not a circle: nothing to say
        geom::snap_ring(m, &ideal, s, self.eye(), m.diag * 2e-4).ok_or("ring would not sit on the surface here: kept as drawn")
    }

    fn run_live(&mut self) {
        self.dirty_auto = false;
        let Some(m) = self.mesh.as_ref() else { return };
        let st: Vec<&[Vec3]> = self.strokes.iter().map(|s| s.pts.as_slice()).collect();
        if st.len() < 4 {
            self.live_patches.clear();
            self.auto_note = "live: draw a net of 4+ crossing strokes".into();
            return;
        }
        let t = std::time::Instant::now();
        let r = geom::auto_patches(m, &st, m.diag * self.snap * 1e-3, m.diag * self.edge * 0.01);
        self.auto_note = format!("live: {} cells, {} skipped, {} ms", r.patches.len(), r.skipped, t.elapsed().as_millis());
        self.live_patches = r.patches;
        self.patch_rev += 1;
    }

    /// A stroke drawn while faces are selected (or a region is open): re-quad the region around it.
    fn reflow_stroke(&mut self, mut pts: Vec<Vec3>) {
        if self.reflow.is_none() {
            if self.live {
                self.auto(); // freeze live patches so the face indices are patch cells
            }
            self.ensure_faces();
            let orig = self.patches.clone();
            let (mut base, mut cells, mut idx) = (vec![], vec![], 0usize);
            for pa in &orig {
                let (w, n) = (pa.nu + 1, pa.nu * pa.nv);
                if !(0..n).any(|c| self.fsel.contains(&(idx + c))) {
                    base.push(pa.clone());
                } else {
                    for j in 0..pa.nv {
                        for i in 0..pa.nu {
                            let k = j * w + i;
                            let q = [k, k + 1, k + w, k + w + 1];
                            if self.fsel.contains(&(idx + j * pa.nu + i)) {
                                cells.push([pa.pts[k], pa.pts[k + 1], pa.pts[k + w + 1], pa.pts[k + w]]);
                            } else {
                                base.push(Patch { nu: 1, nv: 1, pts: q.map(|x| pa.pts[x]).to_vec(), nrm: q.map(|x| pa.nrm[x]).to_vec() });
                            }
                        }
                    }
                }
                idx += n;
            }
            let tol = self.mesh.as_ref().map_or(1e-4, |m| m.diag * 1e-4);
            let bnd = geom::region_loops(&cells, tol);
            if bnd.is_empty() {
                self.status = "reflow: select faces that touch each other first".into();
                return;
            }
            self.reflow = Some(Reflow { orig, base, bnd, strokes: vec![] });
            self.fsel.clear();
        }
        // ends snap onto the region outline (and onto earlier strokes of this region)
        if let Some(r) = &self.reflow {
            let others: Vec<&[Vec3]> = r.bnd.iter().chain(&r.strokes).map(|s| s.as_slice()).collect();
            if !geom::is_closed(&pts) {
                if let Some(q) = geom::nearest_on_strokes(*pts.last().unwrap(), &others, self.snap_r) {
                    pts.push(q);
                }
                if let Some(q) = geom::nearest_on_strokes(pts[0], &others, self.snap_r) {
                    pts.insert(0, q);
                }
            }
        }
        let twin = match (&self.mesh, self.sym) {
            (Some(m), true) => Some(pts.iter().map(|&p| geom::mirror_on_surface(m, p, self.axis, self.plane, m.diag * 2e-4)).collect::<Vec<_>>()),
            _ => None,
        };
        if let Some(r) = &mut self.reflow {
            r.strokes.push(pts);
            r.strokes.extend(twin);
        }
        self.run_reflow();
    }

    fn run_reflow(&mut self) {
        let (Some(r), Some(m)) = (&self.reflow, &self.mesh) else { return };
        let all: Vec<&[Vec3]> = r.bnd.iter().chain(&r.strokes).map(|s| s.as_slice()).collect();
        let res = geom::auto_patches(m, &all, m.diag * self.snap * 1e-3, m.diag * self.edge * 0.01);
        self.status = format!("reflow: {} cells, {} regions skipped · Enter keeps it, Ctrl+Z steps back, Esc cancels", res.patches.len(), res.skipped);
        self.patches = r.base.iter().cloned().chain(res.patches).collect();
        self.patch_log.clear();
        self.patch_rev += 1;
        self.fsel.clear();
    }

    /// Ctrl+Z / Esc inside a region: drop the last stroke, or restore the original faces.
    fn reflow_back(&mut self, all: bool) -> bool {
        let Some(r) = &mut self.reflow else { return false };
        if !all {
            r.strokes.pop();
            if !r.strokes.is_empty() {
                self.run_reflow();
                return true;
            }
        }
        if let Some(r) = self.reflow.take() {
            self.patches = r.orig;
            self.patch_rev += 1;
            self.status = "reflow cancelled".into();
        }
        true
    }

    /// Stroke ends within 16 px of another stroke jump onto it, so nets close without exact aim.
    fn connect_ends(&self, pts: &mut Vec<Vec3>) -> bool {
        if !self.snap_ends || self.strokes.is_empty() || geom::is_closed(pts) {
            return false;
        }
        let others: Vec<&[Vec3]> = self.strokes.iter().map(|s| s.pts.as_slice()).collect();
        let mut changed = false;
        if let Some(q) = geom::nearest_on_strokes(*pts.last().unwrap(), &others, self.snap_r) {
            pts.push(q);
            changed = true;
        }
        if let Some(q) = geom::nearest_on_strokes(pts[0], &others, self.snap_r) {
            pts.insert(0, q);
            changed = true;
        }
        changed
    }

    fn export(&mut self) {
        let Some(m) = &self.mesh else { return };
        // Maya's exchange file keeps its fixed name; any other mesh gets <name>_retopo.obj beside it
        let src = PathBuf::from(&self.path);
        let stem = src.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
        let out = src.with_file_name(if stem == "mesh" { "retopo.obj".to_string() } else { format!("{stem}_retopo.obj") });
        let obj = geom::to_obj(self.patches.iter().chain(&self.live_patches), m.diag * self.weld * 1e-3);
        self.status = match fs::write(&out, obj) {
            Ok(_) => format!("exported {} patches -> {}", self.patches.len() + self.live_patches.len(), out.display()),
            Err(e) => format!("export failed: {e}"),
        };
    }

    fn delete_selected(&mut self) {
        self.dirty_auto = true;
        self.strokes.retain(|s| !self.selected.contains(&s.id));
        self.selected.clear();
    }

    fn eye(&self) -> Vec3 {
        self.target + self.dist * Vec3::new(self.pitch.cos() * self.yaw.sin(), self.pitch.sin(), self.pitch.cos() * self.yaw.cos())
    }
}

/// Distance from point p to segment ab (2D).
fn seg_dist(p: egui::Vec2, a: egui::Vec2, b: egui::Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() > 0.0 { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    (p - (a + ab * t)).length()
}

/// Surface point (lifted a hair) under a screen position.
fn ray_hit(m: &Mesh, vp: Mat4, rect: egui::Rect, p: egui::Pos2) -> Option<Vec3> {
    let nx = (p.x - rect.min.x) / rect.width() * 2.0 - 1.0;
    let ny = 1.0 - (p.y - rect.min.y) / rect.height() * 2.0;
    let inv = vp.inverse();
    let (o, f) = (inv.project_point3(Vec3::new(nx, ny, 0.0)), inv.project_point3(Vec3::new(nx, ny, 1.0)));
    let dir = (f - o).normalize();
    geom::raycast(m, o, dir).map(|(t, n)| o + dir * t + n * m.diag * 2e-4)
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if FOCUS.swap(false, Ordering::Relaxed) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        // a file dropped on the window, or handed over by a second launch
        let dropped = ui.input(|i| i.raw.dropped_files.iter().next().map(|f| f.path().to_path_buf()));
        let asked = OPEN.lock().unwrap().take();
        if let Some(p) = dropped.map(|p| p.to_string_lossy().into_owned()).or(asked) {
            if p.to_lowercase().ends_with(".obj") {
                self.open_path(p);
            } else {
                self.status = "only .obj files can be opened".into();
            }
        }
        // the file changes on disk (e.g. Maya re-exported it) -> reload
        ui.ctx().request_repaint_after(Duration::from_millis(1000));
        let now = fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if now.is_some() && now != self.mtime {
            self.load();
        }
        if self.last_save.elapsed() > Duration::from_secs(2) {
            self.last_save = std::time::Instant::now();
            self.autosave();
        }
        // Space held = radial menu at the cursor; release on a slice to pick it
        let typing = ui.ctx().egui_wants_keyboard_input();
        let (space_down, space_hit, tab_down, tab_hit) =
            ui.input(|i| (i.key_down(egui::Key::Space), i.key_pressed(egui::Key::Space), i.key_down(egui::Key::Tab), i.key_pressed(egui::Key::Tab)));
        if space_down || space_hit || tab_down || tab_hit {
            if let Some(id) = ui.memory(|m| m.focused()) {
                ui.memory_mut(|m| m.surrender_focus(id)); // keys must not walk keyboard focus through the buttons
            }
        }
        if space_hit && !typing && self.pie.is_none() {
            self.pie = ui.input(|i| i.pointer.latest_pos());
        }
        if !space_down {
            if let Some(c) = self.pie.take() {
                if let Some(k) = ui.input(|i| i.pointer.latest_pos()).and_then(|p| Self::pie_pick(c, p)) {
                    self.pie_do(k);
                }
            }
        } else if self.pie.is_some() {
            ui.ctx().request_repaint();
        }
        // Tab: tap toggles Edit <-> pencil; hold = Edit while held, back to the pencil on release
        if tab_hit && !typing && self.tab_t.is_none() {
            self.tab_t = Some(std::time::Instant::now());
            self.tab_prev = self.tool;
            self.tool = Tool::Edit;
        }
        if !tab_down {
            if let Some(t0) = self.tab_t.take() {
                self.tool = if t0.elapsed() < Duration::from_millis(250) && self.tab_prev != Tool::Edit { Tool::Edit } else { Tool::Draw };
            }
        }
        if !typing {
            let key = |k| ui.input(|i| i.key_pressed(k));
            if key(egui::Key::B) || key(egui::Key::Num1) {
                self.tool = if self.tool == Tool::Brush { Tool::Draw } else { Tool::Brush };
            }
            if key(egui::Key::R) {
                (self.tool, self.bk, self.tab) = (Tool::Brush, BrushKind::Relax, Tab::Brush);
            }
            if key(egui::Key::V) {
                (self.tool, self.bk, self.tab) = (Tool::Brush, BrushKind::Grab, Tab::Brush);
            }
            if key(egui::Key::OpenBracket) {
                self.brush_px = (self.brush_px * 0.85).max(8.0);
            }
            if key(egui::Key::CloseBracket) {
                self.brush_px = (self.brush_px / 0.85).min(300.0);
            }
            if key(egui::Key::G) {
                self.grid_by_selection();
            }
            if key(egui::Key::A) {
                self.auto();
            }
            if key(egui::Key::L) {
                self.toggle_live();
            }
            if key(egui::Key::M) {
                self.sym = !self.sym;
            }
            if key(egui::Key::E) {
                self.export();
            }
            if key(egui::Key::P) {
                self.book = !self.book;
            }
            if key(egui::Key::Delete) {
                self.delete_selected();
            }
            if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z)) && !self.reflow_back(false) && !(self.tool == Tool::Brush && self.undo_relax()) && !self.strokes.is_empty() {
                self.undo_stroke();
            }
        }

        if self.book {
            self.rail(ui);
            self.book(ui);
        }
        self.viewport(ui);
        if self.live && self.dirty_auto && self.cur.is_empty() && self.line.is_none() {
            self.run_live();
        }
        self.pie_draw(ui);
    }

    fn on_exit(&mut self) {
        self.autosave();
    }
}

fn project2d(pts: &[Vec3], r: egui::Rect) -> Vec<egui::Pos2> {
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in pts {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let ext = hi - lo;
    let mut ax = [0usize, 1, 2];
    ax.sort_by(|&a, &b| ext[b].total_cmp(&ext[a]));
    let (a, b) = (ax[0], ax[1]);
    let sc = (r.width() / ext[a].max(1e-6)).min(r.height() / ext[b].max(1e-6));
    let c = r.center();
    pts.iter()
        .map(|p| egui::pos2(c.x + (p[a] - (lo[a] + ext[a] * 0.5)) * sc, c.y - (p[b] - (lo[b] + ext[b] * 0.5)) * sc))
        .collect()
}

fn rail_item(ui: &mut egui::Ui, label: &str, key: &str, ic: theme::Icon, active: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(52.0, 52.0), egui::Sense::click());
    let (bg, fg) = if active {
        (theme::LAMP.gamma_multiply(0.22), theme::LAMP)
    } else if resp.hovered() {
        (theme::LEATHER, theme::TITLE)
    } else {
        (egui::Color32::TRANSPARENT, theme::MUTED)
    };
    let p = ui.painter();
    p.rect_filled(rect, 6.0, bg);
    theme::icon(p, rect.center_top() + egui::vec2(0.0, 18.0), ic, fg);
    p.text(rect.center_bottom() - egui::vec2(0.0, 9.0), egui::Align2::CENTER_CENTER, label, theme::typew(9.5), fg);
    resp.on_hover_text(format!("{label}  ({key})")).clicked()
}

impl App {
    /// Cheap digest of everything the GPU geometry depends on; the buffers are rebuilt only when it changes.
    fn geo_fp(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.strokes.len().hash(&mut h);
        for s in &self.strokes {
            (s.id, s.pts.len()).hash(&mut h);
        }
        self.selected.hash(&mut h);
        (self.patches.len(), self.live_patches.len(), self.patch_rev).hash(&mut h);
        self.low.map(f32::to_bits).hash(&mut h);
        (self.sym, self.axis, self.plane.to_bits()).hash(&mut h);
        h.finish()
    }

    /// Digest of everything the session file stores.
    fn session_fp(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.geo_fp().hash(&mut h);
        (self.u, self.v, self.weld.to_bits(), self.snap.to_bits(), self.edge.to_bits(), self.low_a.to_bits()).hash(&mut h);
        (self.smart, self.live, self.snap_ends, self.next_id).hash(&mut h);
        h.finish()
    }

    fn build_geo(&self) -> Geo {
        let diag = self.mesh.as_ref().map_or(1.0, |m| m.diag);
        let mut g = Geo::default();
        for s in &self.strokes {
            let (col, w) = if self.selected.contains(&s.id) { ([1.0, 0.95, 0.1], 3.4) } else { ([1.0, 0.45, 0.0], 2.2) };
            g.segs.extend(s.pts.windows(2).map(|p| Seg::new(p[0], p[1], col, w)));
        }
        if self.sym {
            let c = self.mesh.as_ref().map_or(Vec3::ZERO, |m| m.center);
            let h = diag * 0.6;
            let (a1, a2) = ((self.axis + 1) % 3, (self.axis + 2) % 3);
            let corner = |x: f32, y: f32| {
                let mut p = c;
                p[self.axis] = self.plane;
                p[a1] += x * h;
                p[a2] += y * h;
                p
            };
            let cs = [corner(-1.0, -1.0), corner(1.0, -1.0), corner(1.0, 1.0), corner(-1.0, 1.0)];
            for k in 0..4 {
                g.segs.push(Seg::new(cs[k], cs[(k + 1) % 4], [0.35, 0.55, 1.0], 1.6));
            }
        }
        let edge = [(self.low[0] + 0.2).min(1.0), (self.low[1] + 0.35).min(1.0), (self.low[2] + 0.45).min(1.0)];
        for pa in self.patches.iter().chain(&self.live_patches) {
            let w = pa.nu + 1;
            for j in 0..=pa.nv {
                for i in 0..=pa.nu {
                    let k = j * w + i;
                    if i < pa.nu {
                        g.psegs.push(Seg::new(pa.pts[k], pa.pts[k + 1], edge, 2.6));
                    }
                    if j < pa.nv {
                        g.psegs.push(Seg::new(pa.pts[k], pa.pts[k + w], edge, 2.6));
                    }
                    if i < pa.nu && j < pa.nv {
                        for q in [k, k + 1, k + w + 1, k, k + w + 1, k + w] {
                            g.faces.push(Vertex { pos: pa.pts[q].to_array(), nrm: pa.nrm[q].to_array() });
                        }
                    }
                }
            }
        }
        g
    }

    /// Cache of every quad (patch order, row-major); selection indices point into it.
    fn ensure_faces(&mut self) {
        let key = (self.patches.len(), self.live_patches.len(), self.patch_rev);
        if self.faces_key == key {
            return;
        }
        self.faces_key = key;
        self.faces.clear();
        for pa in self.patches.iter().chain(&self.live_patches) {
            let w = pa.nu + 1;
            for j in 0..pa.nv {
                for i in 0..pa.nu {
                    let k = j * w + i;
                    self.faces.push([pa.pts[k], pa.pts[k + 1], pa.pts[k + w + 1], pa.pts[k + w]]);
                }
            }
        }
    }

    /// Nearest face under the cursor that is not hidden by the high mesh.
    fn pick_face(&self, vp: Mat4, rect: egui::Rect, p: egui::Pos2, eye: Vec3) -> Option<usize> {
        let scr = |q: Vec3| {
            let c = vp * q.extend(1.0);
            egui::pos2(rect.min.x + (c.x / c.w * 0.5 + 0.5) * rect.width(), rect.min.y + (0.5 - c.y / c.w * 0.5) * rect.height())
        };
        let tri = |a: egui::Pos2, b: egui::Pos2, c: egui::Pos2| {
            let s = |p1: egui::Pos2, p2: egui::Pos2, p3: egui::Pos2| (p1.x - p3.x) * (p2.y - p3.y) - (p2.x - p3.x) * (p1.y - p3.y);
            let (d1, d2, d3) = (s(p, a, b), s(p, b, c), s(p, c, a));
            !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
        };
        let mut hits: Vec<(f32, usize)> = (0..self.faces.len())
            .filter(|&i| {
                let q = self.faces[i].map(scr);
                tri(q[0], q[1], q[2]) || tri(q[0], q[2], q[3])
            })
            .map(|i| (((self.faces[i][0] + self.faces[i][2]) * 0.5 - eye).length(), i))
            .collect();
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        hits.into_iter().map(|h| h.1).find(|&i| {
            let c = (self.faces[i][0] + self.faces[i][2]) * 0.5;
            let d = c - eye;
            let l = d.length().max(1e-9);
            self.mesh.as_ref().map_or(true, |m| geom::raycast(m, eye, d / l).map_or(true, |(t, _)| t > l - m.diag * 0.01))
        })
    }

    /// The face on the other side of the symmetry plane.
    fn mirror_face(&self, i: usize) -> Option<usize> {
        let m = self.mesh.as_ref()?;
        let mc = geom::mirror_on_surface(m, (self.faces[i][0] + self.faces[i][2]) * 0.5, self.axis, self.plane, 0.0);
        let (j, d) = (0..self.faces.len()).map(|j| (j, ((self.faces[j][0] + self.faces[j][2]) * 0.5).distance(mc))).min_by(|a, b| a.1.total_cmp(&b.1))?;
        (d < m.diag * 0.03).then_some(j)
    }

    fn begin_brush(&mut self) {
        if self.live {
            self.auto(); // freeze the live result: brushing changes it, a rebuild would undo that
        }
        let Some(m) = &self.mesh else { return };
        if self.patches.is_empty() {
            self.status = "brush: no patches yet, build a grid or freeze live auto first".into();
            return;
        }
        let tol = (m.diag * self.weld * 1e-3).max(1e-7);
        self.relax_undo.push(self.patches.iter().map(|p| (p.pts.clone(), p.nrm.clone())).collect());
        if self.relax_undo.len() > 8 {
            self.relax_undo.remove(0);
        }
        self.brush = Some(geom::Weld::build(&self.patches, tol));
    }

    fn undo_relax(&mut self) -> bool {
        while let Some(s) = self.relax_undo.pop() {
            if s.len() == self.patches.len() {
                for (p, (pts, nrm)) in self.patches.iter_mut().zip(s) {
                    (p.pts, p.nrm) = (pts, nrm);
                }
                self.patch_rev += 1;
                self.status = "brush undone".into();
                return true;
            }
        }
        false
    }

    fn pie_pick(c: egui::Pos2, p: egui::Pos2) -> Option<usize> {
        let d = p - c;
        if d.length() < 34.0 {
            return None;
        }
        let a = d.x.atan2(-d.y).rem_euclid(std::f32::consts::TAU); // 0 = up, clockwise
        Some(((a + std::f32::consts::FRAC_PI_8) / std::f32::consts::FRAC_PI_4) as usize % 8)
    }

    fn pie_do(&mut self, k: usize) {
        match k {
            0 => self.tool = Tool::Draw,
            7 => self.tool = Tool::Edit,
            1 => (self.tool, self.tab) = (Tool::Brush, Tab::Brush),
            2 => self.grid_by_selection(),
            3 => self.toggle_live(),
            4 => self.sym = !self.sym,
            5 => self.export(),
            6 => self.undo_patch(),
            _ => {}
        }
    }

    fn pie_draw(&self, ui: &egui::Ui) {
        let Some(c) = self.pie else { return };
        let cur = ui.input(|i| i.pointer.latest_pos());
        let hot = cur.and_then(|p| Self::pie_pick(c, p));
        let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::NULL.with("pie")));
        let on = [self.tool == Tool::Draw, self.tool == Tool::Brush, false, self.live, self.sym, false, false, self.tool == Tool::Edit];
        let (r0, r1, step) = (44.0f32, 128.0f32, std::f32::consts::FRAC_PI_4);
        let at = |r: f32, a: f32| c + egui::vec2(a.sin(), -a.cos()) * r;
        p.circle_filled(c, r1 + 10.0, egui::Color32::from_rgba_unmultiplied(8, 5, 3, 120));
        for k in 0..8 {
            let (a0, a1) = (k as f32 * step - step / 2.0 + 0.03, k as f32 * step + step / 2.0 - 0.03);
            let mut pts: Vec<egui::Pos2> = (0..=8).map(|s| at(r1, a0 + (a1 - a0) * s as f32 / 8.0)).collect();
            pts.extend((0..=8).rev().map(|s| at(r0, a0 + (a1 - a0) * s as f32 / 8.0)));
            let (fill, stroke) = if hot == Some(k) {
                (theme::LAMP, theme::INK_DEEP)
            } else if on[k] {
                (egui::Color32::from_rgb(0x4a, 0x35, 0x22), theme::LAMP)
            } else {
                (egui::Color32::from_rgba_unmultiplied(23, 16, 11, 240), theme::LINE)
            };
            p.add(egui::Shape::convex_polygon(pts, fill, egui::Stroke::new(1.5, stroke)));
            p.text(at((r0 + r1) / 2.0, k as f32 * step), egui::Align2::CENTER_CENTER, PIE[k], theme::typew(12.0), if hot == Some(k) { theme::INK_DEEP } else { theme::CREAM });
        }
        p.circle_filled(c, r0 - 6.0, theme::NOIR);
        p.circle_stroke(c, r0 - 6.0, egui::Stroke::new(1.0, theme::LINE));
        p.text(c, egui::Align2::CENTER_CENTER, hot.map_or("SPACE", |k| PIE[k]), theme::serif(11.0), theme::TITLE);
    }

    fn toggle_live(&mut self) {
        self.live = !self.live;
        self.dirty_auto = true;
        if !self.live {
            self.live_patches.clear();
        }
        self.status = if self.live { "live auto ON: patches follow your strokes".into() } else { "live auto off".into() };
    }

    fn grid_by_selection(&mut self) {
        match self.selected.len() {
            2 => self.grid(false),
            4 => self.grid(true),
            n => self.status = format!("click 2 or 4 strokes first (selected: {n})"),
        }
    }

    fn rail(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("rail")
            .exact_size(64.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(theme::RAIL).stroke(egui::Stroke::new(1.0, theme::LINE)).inner_margin(egui::Margin::symmetric(6, 8)))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                if rail_item(ui, "Pencil", "default", theme::Icon::Pencil, self.tool == Tool::Draw) {
                    self.tool = Tool::Draw;
                }
                if rail_item(ui, "Edit", "Tab", theme::Icon::Edit, self.tool == Tool::Edit) {
                    self.tool = if self.tool == Tool::Edit { Tool::Draw } else { Tool::Edit };
                }
                if rail_item(ui, "Brush", "B / 1", theme::Icon::Relax, self.tool == Tool::Brush) {
                    self.tool = if self.tool == Tool::Brush { Tool::Draw } else { Tool::Brush };
                }
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
                if rail_item(ui, "Grid", "G", theme::Icon::Grid, false) {
                    self.grid_by_selection();
                }
                if rail_item(ui, "Auto", "A", theme::Icon::Auto, false) {
                    self.auto();
                }
                if rail_item(ui, "Live", "L", theme::Icon::Live, self.live) {
                    self.toggle_live();
                }
                if rail_item(ui, "Mirror", "M", theme::Icon::Mirror, self.sym) {
                    self.sym = !self.sym;
                }
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
                if rail_item(ui, "Undo", "Ctrl+Z", theme::Icon::Undo, false) {
                    self.undo_patch();
                }
                if rail_item(ui, "Export", "E", theme::Icon::Export, false) {
                    self.export();
                }
            });
    }

    /// Right-hand panel: flat tabs on top, one page below.
    fn book(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("book")
            .exact_size(310.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(theme::PANEL).stroke(egui::Stroke::new(1.0, theme::LINE)).inner_margin(egui::Margin::same(12)))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                let tabs = [(Tab::Strokes, "Strokes"), (Tab::Grid, "Grid"), (Tab::Auto, "Auto"), (Tab::Mirror, "Mirror"), (Tab::Brush, "Brush"), (Tab::Export, "Export")];
                let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::hover());
                let w = row.width() / tabs.len() as f32;
                for (k, (tab, name)) in tabs.iter().enumerate() {
                    let r = egui::Rect::from_min_size(row.min + egui::vec2(w * k as f32, 0.0), egui::vec2(w, 28.0));
                    let resp = ui.interact(r, ui.scope_id().with(("tab", k)), egui::Sense::click());
                    let on = self.tab == *tab;
                    let p = ui.painter();
                    p.text(r.center(), egui::Align2::CENTER_CENTER, *name, theme::typew(12.0), if on { theme::TITLE } else if resp.hovered() { theme::CREAM } else { theme::MUTED });
                    if on {
                        p.line_segment([r.left_bottom() + egui::vec2(6.0, 0.0), r.right_bottom() - egui::vec2(6.0, 0.0)], egui::Stroke::new(2.0, theme::LAMP));
                    }
                    if resp.clicked() {
                        self.tab = *tab;
                    }
                }
                ui.separator();
                theme::paper_visuals(ui);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    match self.tab {
                        Tab::Strokes => self.page_strokes(ui),
                        Tab::Grid => self.page_grid(ui),
                        Tab::Auto => self.page_auto(ui),
                        Tab::Mirror => self.page_mirror(ui),
                        Tab::Brush => self.page_brush(ui),
                        Tab::Export => self.page_export(ui),
                    }
                });
            });
    }

    fn heading(ui: &mut egui::Ui, title: &str, note: &str) {
        theme::label(ui, title, egui::FontId::proportional(16.0), theme::TITLE);
        theme::label(ui, note, theme::typew(11.5), theme::MUTED);
        ui.add_space(2.0);
    }

    fn page_strokes(&mut self, ui: &mut egui::Ui) {
        Self::heading(ui, "Evidence", &format!("Collected: {} strokes · {} picked", self.strokes.len(), self.selected.len()));
        ui.checkbox(&mut self.smart, egui::RichText::new("Recognise circles & ellipses").font(theme::typew(12.0)));
        ui.add(egui::Slider::new(&mut self.beauty, 0.0..=1.0).text("SMOOTH STROKES"));
        ui.checkbox(&mut self.snap_ends, egui::RichText::new("Snap stroke ends to nearby strokes").font(theme::typew(12.0)));
        theme::label(ui, "Hold Shift while dragging for a straight line.", theme::hand(17.0), theme::INK_SOFT);
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 8.0) / 2.0;
            if theme::button(ui, "DELETE PICKED", theme::Kind::Ink, egui::vec2(w, 32.0)).clicked() {
                self.delete_selected();
            }
            if theme::button(ui, "CLEAR ALL", theme::Kind::Ghost, egui::vec2(w, 32.0)).clicked() {
                (self.strokes, self.selected, self.patches, self.patch_log) = Default::default();
                self.dirty_auto = true;
            }
        });
        ui.add_space(4.0);
        let mut toggled = None;
        for (n, s) in self.strokes.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.0), egui::Sense::click());
            let sel = self.selected.contains(&s.id);
            let p = ui.painter();
            p.rect_filled(rect, 3.0, if sel { theme::LAMP } else if resp.hovered() { theme::PAPER_LIGHT } else { theme::PAPER_LIGHT.gamma_multiply(0.55) });
            p.rect_stroke(rect, 3.0, egui::Stroke::new(1.0, if sel { theme::LAMP } else { theme::INK_SOFT.gamma_multiply(0.5) }), egui::StrokeKind::Inside);
            p.circle_filled(rect.left_center() + egui::vec2(12.0, 0.0), 4.0, theme::RED);
            p.text(rect.left_center() + egui::vec2(26.0, -7.0), egui::Align2::LEFT_CENTER, format!("Stroke {:02}", n + 1), theme::typew(12.0), if sel { theme::INK_DEEP } else { theme::INK });
            let twin = s.partner.and_then(|pid| self.strokes.iter().position(|x| x.id == pid));
            let tag = match (geom::is_closed(&s.pts), twin) {
                (true, Some(k)) => format!("ring · twin {:02}", k + 1),
                (true, None) => "ring".into(),
                (false, Some(k)) => format!("twin {:02}", k + 1),
                _ => format!("{} pts", s.pts.len()),
            };
            p.text(rect.left_center() + egui::vec2(26.0, 8.0), egui::Align2::LEFT_CENTER, tag, theme::mono(9.5), theme::INK_SOFT);
            let pr = egui::Rect::from_center_size(rect.right_center() - egui::vec2(36.0, 0.0), egui::vec2(60.0, 26.0));
            p.add(egui::Shape::line(project2d(&s.pts, pr), egui::Stroke::new(1.8, if sel { theme::INK } else { theme::ORANGE_DARK })));
            if resp.clicked() {
                toggled = Some(s.id);
            }
            ui.add_space(5.0);
        }
        if let Some(id) = toggled {
            match self.selected.iter().position(|&x| x == id) {
                Some(k) => {
                    self.selected.remove(k);
                }
                None => self.selected.push(id),
            }
        }
    }

    fn page_grid(&mut self, ui: &mut egui::Ui) {
        Self::heading(ui, "Manual Grid", "Pick 2 strokes to bridge, or 4 to fill a patch.");
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 8.0) / 2.0;
            if theme::button(ui, "GRID FROM 2", theme::Kind::Ink, egui::vec2(w, 38.0)).clicked() {
                self.grid(false);
            }
            if theme::button(ui, "GRID FROM 4", theme::Kind::Ghost, egui::vec2(w, 38.0)).clicked() {
                self.grid(true);
            }
        });
        ui.horizontal(|ui| {
            theme::label(ui, "U SPANS", theme::typew(11.0), theme::INK_SOFT);
            ui.add(egui::DragValue::new(&mut self.u).range(1..=128));
            theme::label(ui, "V SPANS", theme::typew(11.0), theme::INK_SOFT);
            ui.add(egui::DragValue::new(&mut self.v).range(1..=128));
        });
        theme::label(ui, "Two rings (limbs, fingers) bridge into a tube.", theme::hand(17.0), theme::INK_SOFT);
        theme::label(ui, format!("{} selected", self.selected.len()), theme::mono(11.0), theme::INK_SOFT);
    }

    fn page_auto(&mut self, ui: &mut egui::Ui) {
        Self::heading(ui, "Smart Auto", "Cross a net of strokes; every 4-sided cell becomes a grid.");
        let (txt, kind) = if self.live { ("LIVE · ON", theme::Kind::Amber) } else { ("LIVE · OFF", theme::Kind::Ghost) };
        if theme::button(ui, txt, kind, egui::vec2(ui.available_width(), 44.0)).clicked() {
            self.toggle_live();
        }
        theme::label(ui, "Live re-builds the patches after every stroke, stroke ends snap to their neighbours, and mirrored strokes come with their twins.", theme::hand(17.0), theme::INK_SOFT);
        let c1 = ui.add(egui::Slider::new(&mut self.edge, 0.2..=30.0).suffix(" %").text("EDGE SIZE")).changed();
        let c2 = ui.add(egui::Slider::new(&mut self.snap, 1.0..=50.0).suffix(" ‰").text("TOUCH")).changed();
        self.dirty_auto |= c1 || c2;
        ui.checkbox(&mut self.snap_ends, egui::RichText::new("Snap stroke ends").font(theme::typew(12.0)));
        let label = if self.live { "FREEZE TO PATCHES" } else { "RUN ONCE" };
        if theme::button(ui, label, theme::Kind::Ink, egui::vec2(ui.available_width(), 38.0)).clicked() {
            self.auto();
        }
        if !self.auto_note.is_empty() {
            theme::label(ui, &self.auto_note, theme::hand(19.0), theme::INK);
        }
    }

    fn page_mirror(&mut self, ui: &mut egui::Ui) {
        Self::heading(ui, "Symmetry", "New strokes and grids are mirrored onto the other side.");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.sym, egui::RichText::new("ON").font(theme::typew(12.0)));
            for (i, n) in ["X", "Y", "Z"].iter().enumerate() {
                ui.selectable_value(&mut self.axis, i, *n);
            }
        });
        ui.horizontal(|ui| {
            theme::label(ui, "PLANE", theme::typew(11.0), theme::INK_SOFT);
            ui.add(egui::DragValue::new(&mut self.plane).speed(0.01));
            if ui.link(egui::RichText::new("plane = center").font(theme::typew(11.0)).color(theme::BLUE)).clicked() {
                if let Some(m) = &self.mesh {
                    self.plane = m.center[self.axis];
                }
            }
        });
    }

    fn page_brush(&mut self, ui: &mut egui::Ui) {
        Self::heading(ui, "Brush", "B or 1 toggles it. Drag over the low mesh.");
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 8.0) / 2.0;
            for (k, name) in [(BrushKind::Relax, "Relax  R"), (BrushKind::Grab, "Grab  V")] {
                let on = self.tool == Tool::Brush && self.bk == k;
                if theme::button(ui, name, if on { theme::Kind::Amber } else { theme::Kind::Ghost }, egui::vec2(w, 32.0)).clicked() {
                    (self.tool, self.bk) = (Tool::Brush, k);
                }
            }
        });
        ui.add(egui::Slider::new(&mut self.brush_px, 8.0..=300.0).suffix(" px").text("Size  [ ]"));
        ui.add(egui::Slider::new(&mut self.brush_hard, 0.0..=0.95).text("Hardness"));
        ui.add_enabled(self.bk == BrushKind::Relax, egui::Slider::new(&mut self.brush_strength, 0.05..=1.0).text("Strength"));
        ui.checkbox(&mut self.pin_border, "Keep patch borders in place");
        theme::label(ui, "Ctrl + right-drag: sideways = size, up/down = hardness. Symmetry applies. Live auto freezes into patches when you brush. Ctrl+Z undoes a brush stroke.", theme::hand(17.0), theme::INK_SOFT);
    }

    fn page_export(&mut self, ui: &mut egui::Ui) {
        Self::heading(ui, "Export", "Weld the patches and send the quads back to Maya.");
        ui.add(egui::Slider::new(&mut self.weld, 0.0..=20.0).suffix(" ‰").text("MERGE BELOW"));
        ui.horizontal(|ui| {
            theme::label(ui, "LOW-POLY COLOUR", theme::typew(11.0), theme::INK_SOFT);
            ui.color_edit_button_rgb(&mut self.low);
            if ui.link(egui::RichText::new("reset").font(theme::typew(11.0)).color(theme::BLUE)).clicked() {
                self.low = LOW_DEFAULT;
            }
        });
        ui.add(egui::Slider::new(&mut self.low_a, 0.1..=1.0).text("FILL OPACITY"));
        ui.add(egui::Slider::new(&mut self.low_lift, 0.0..=60.0).suffix(" ‰").text("LIFT OVER HIGH"));
        ui.add_space(8.0);
        if theme::button(ui, "EXPORT TO MAYA", theme::Kind::Red, egui::vec2(ui.available_width(), 52.0)).clicked() {
            self.export();
        }
        if theme::button(ui, "UNDO PATCH", theme::Kind::Ghost, egui::vec2(ui.available_width(), 34.0)).clicked() {
            self.undo_patch();
        }
        theme::label(ui, "Saved next to the mesh as <name>_retopo.obj (Maya: Import retopo mesh).", theme::hand(17.0), theme::INK_SOFT);
    }

    fn viewport(&mut self, ui: &mut egui::Ui) {
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        ui.painter().rect_filled(rect, 0.0, theme::VIEWPORT);
        let diag = self.mesh.as_ref().map_or(1.0, |m| m.diag);
        let (alt, shift) = ui.input(|i| (i.modifiers.alt, i.modifiers.shift));
        let busy = self.pie.is_some();
        let d = resp.drag_delta();

        // camera
        let ctrl = ui.input(|i| i.modifiers.command);
        if self.tool == Tool::Brush && ctrl && resp.dragged_by(egui::PointerButton::Secondary) {
            // Substance-style: Ctrl+RMB drag, sideways = size, up/down = hardness
            self.brush_px = (self.brush_px + d.x).clamp(8.0, 300.0);
            self.brush_hard = (self.brush_hard - d.y * 0.004).clamp(0.0, 0.95);
        } else if !busy && (resp.dragged_by(egui::PointerButton::Secondary) || (alt && resp.dragged_by(egui::PointerButton::Primary))) {
            self.yaw -= d.x * 0.01;
            self.pitch = (self.pitch + d.y * 0.01).clamp(-1.5, 1.5);
        }
        let aspect = (rect.width() / rect.height().max(1.0)).max(0.01);
        let fov = 45f32.to_radians();
        let view = Mat4::look_at_rh(self.eye(), self.target, Vec3::Y);
        let proj = Mat4::perspective_rh(fov, aspect, diag * 0.001, diag * 50.0);
        let vp = proj * view;
        let (right, up) = (view.row(0).truncate(), view.row(1).truncate());
        if !busy && resp.dragged_by(egui::PointerButton::Middle) {
            let k = 2.0 * self.dist * (fov * 0.5).tan() / rect.height().max(1.0);
            self.target += (-d.x * right + d.y * up) * k;
        }
        self.snap_r = 16.0 * 2.0 * self.dist * (fov * 0.5).tan() / rect.height().max(1.0);
        if resp.hovered() && !busy {
            let s = ui.input(|i| i.smooth_scroll_delta.y);
            self.dist = (self.dist * (1.0 - s * 0.002)).clamp(diag * 0.01, diag * 20.0);
        }
        let eye = self.eye();

        // brush (Relax / Grab) over the low mesh; symmetry applies to both
        let brushing = self.tool == Tool::Brush && resp.dragged_by(egui::PointerButton::Primary) && !alt && !busy;
        if brushing {
            if self.brush.is_none() {
                self.begin_brush();
                self.grab = None;
            }
            if let (Some(m), Some(w), Some(p)) = (&self.mesh, self.brush.as_mut(), resp.interact_pointer_pos()) {
                if let Some(hit) = ray_hit(m, vp, rect, p) {
                    let k = 2.0 * (hit - eye).length() * (fov * 0.5).tan() / rect.height().max(1.0); // world units per pixel
                    let mirror = |q: Vec3| geom::mirror_on_surface(m, q, self.axis, self.plane, diag * 2e-4);
                    let mut changed = false;
                    match self.bk {
                        BrushKind::Relax => {
                            let centers = if self.sym { vec![hit, mirror(hit)] } else { vec![hit] };
                            let wt = w.weights(&centers, self.brush_px * k, self.brush_hard, self.pin_border);
                            changed = w.relax(m, &wt, self.brush_strength);
                        }
                        BrushKind::Grab => {
                            // the grabbed set is fixed when the stroke starts and follows the cursor
                            let (w1, w2, k0) = self.grab.get_or_insert_with(|| {
                                let w1 = w.weights(&[hit], self.brush_px * k, self.brush_hard, self.pin_border);
                                let w2 = if self.sym { w.weights(&[mirror(hit)], self.brush_px * k, self.brush_hard, self.pin_border) } else { vec![] };
                                (w1, w2, k)
                            });
                            let delta = (right * d.x - up * d.y) * *k0;
                            let mut dm = delta;
                            dm[self.axis] = -dm[self.axis];
                            let mut parts: Vec<(&[f32], Vec3)> = vec![(w1.as_slice(), delta)];
                            if !w2.is_empty() {
                                parts.push((w2.as_slice(), dm));
                            }
                            if d != egui::Vec2::ZERO {
                                changed = w.grab(m, &parts);
                            }
                        }
                    }
                    if changed {
                        w.write_back(&mut self.patches);
                        self.patch_rev += 1;
                    }
                }
            }
            ui.ctx().request_repaint(); // keep brushing while the pointer rests
        } else {
            self.brush = None;
            self.grab = None;
        }

        // drawing: freehand, or with Shift a straight line (cheap screen-space preview, raycast on release)
        let drawing = self.tool == Tool::Draw && resp.dragged_by(egui::PointerButton::Primary) && !alt && !busy;
        if drawing {
            if let (Some(m), Some(p)) = (&self.mesh, resp.interact_pointer_pos()) {
                if let (true, Some(o)) = (shift, ui.input(|i| i.pointer.press_origin())) {
                    self.line = Some((o, p));
                    self.cur.clear();
                    self.cur_m.clear();
                } else {
                    self.line = None;
                    if let Some(hit) = ray_hit(m, vp, rect, p) {
                        if self.cur.last().map_or(true, |l| l.distance(hit) > diag * 0.003) {
                            self.cur.push(hit);
                            if self.sym {
                                self.cur_m.push(geom::mirror_on_surface(m, hit, self.axis, self.plane, diag * 2e-4)); // realtime mirror
                            }
                        }
                    }
                }
            }
        } else if let Some((a, b)) = self.line.take() {
            let n = ((b - a).length() / 8.0).ceil().clamp(2.0, 80.0) as usize;
            let pts: Vec<Vec3> = match &self.mesh {
                Some(m) if (b - a).length() > 6.0 => (0..=n).filter_map(|k| ray_hit(m, vp, rect, a.lerp(b, k as f32 / n as f32))).collect(),
                _ => vec![],
            };
            self.cur.clear();
            self.cur_m.clear();
            if pts.len() >= 2 {
                let mut pts = pts;
                self.connect_ends(&mut pts);
                self.push_stroke(pts, None);
            }
        } else if !self.cur.is_empty() {
            let (mut s, tw) = (std::mem::take(&mut self.cur), std::mem::take(&mut self.cur_m));
            if s.len() >= 2 {
                let ring = if self.smart { self.ring_from(&s) } else { Err("") };
                if let Ok(r) = ring {
                    self.status = "circle recognised: stroke closed into a ring".into();
                    self.push_stroke(r, None);
                } else {
                    if let Err(why) = ring {
                        if !why.is_empty() {
                            self.status = why.into();
                        }
                    }
                    let mut tw = Some(tw);
                    if self.beauty > 0.0 {
                        if let Some(m) = &self.mesh {
                            s = geom::beautify(m, &s, self.beauty, m.diag * 2e-4);
                            tw = None; // the mirrored twin is rebuilt from the cleaned stroke
                        }
                    }
                    let moved = self.connect_ends(&mut s);
                    self.push_stroke(s, if moved { None } else { tw });
                }
            }
        }

        // click = pick the stroke under the cursor (toggle); click on nothing = deselect
        if resp.clicked_by(egui::PointerButton::Primary) && !alt && !busy && self.tool == Tool::Draw {
            if let Some(p) = resp.interact_pointer_pos() {
                let scr = |q: Vec3| {
                    let c = vp * q.extend(1.0);
                    egui::vec2(rect.min.x + (c.x / c.w * 0.5 + 0.5) * rect.width(), rect.min.y + (0.5 - c.y / c.w * 0.5) * rect.height())
                };
                let hit = self
                    .strokes
                    .iter()
                    .find(|s| s.pts.windows(2).any(|w| seg_dist(p.to_vec2(), scr(w[0]), scr(w[1])) < 10.0))
                    .map(|s| s.id);
                match hit {
                    Some(i) => match self.selected.iter().position(|&x| x == i) {
                        Some(k) => {
                            self.selected.remove(k);
                        }
                        None => self.selected.push(i),
                    },
                    None => self.selected.clear(),
                }
            }
        }

        // Edit tool: press and sweep over faces to select them (Ctrl = deselect); reacts on press, no drag threshold
        self.ensure_faces();
        if self.faces.len() != self.fsel_n {
            self.fsel_n = self.faces.len();
            self.fsel.clear();
        }
        if self.tool == Tool::Edit && !busy && !alt && resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_down()) {
            self.reflow = None; // choosing new faces keeps the previous region as it is
            let erase = ui.input(|i| i.modifiers.command);
            match resp.interact_pointer_pos().and_then(|p| self.pick_face(vp, rect, p, eye)) {
                Some(i) => {
                    let ids = if self.sym { [Some(i), self.mirror_face(i)] } else { [Some(i), None] };
                    for k in ids.into_iter().flatten() {
                        if erase {
                            self.fsel.remove(&k);
                        } else {
                            self.fsel.insert(k);
                        }
                    }
                    self.status = format!("{} faces selected", self.fsel.len());
                }
                None if ui.input(|i| i.pointer.primary_pressed()) && !erase => self.fsel.clear(), // pressed on nothing
                None => {}
            }
            ui.ctx().request_repaint();
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            self.reflow = None; // keep the new layout
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) && !self.reflow_back(true) {
            self.fsel.clear();
        }

        // geometry buffers are rebuilt only when something changed; the stroke being drawn is tiny and goes every frame
        let fp = self.geo_fp();
        let geo = if self.geo_fp != Some(fp) {
            self.geo_fp = Some(fp);
            Some(Arc::new(self.build_geo()))
        } else {
            None
        };
        let mut dsegs: Vec<Seg> = self.cur.windows(2).map(|w| Seg::new(w[0], w[1], [1.0, 0.45, 0.0], 2.2)).collect();
        if let Some(r) = &self.reflow {
            dsegs.extend(r.strokes.iter().flat_map(|s| s.windows(2).map(|w| Seg::new(w[0], w[1], [1.0, 0.45, 0.0], 2.2))));
            dsegs.extend(r.bnd.iter().flat_map(|s| s.windows(2).map(|w| Seg::new(w[0], w[1], [1.0, 0.82, 0.25], 2.0))));
        }
        dsegs.extend(self.cur_m.windows(2).map(|w| Seg::new(w[0], w[1], [1.0, 0.6, 0.2], 2.2)));
        let gold = [1.0, 0.82, 0.25];
        let ssegs: Vec<Seg> = self
            .fsel
            .iter()
            .filter_map(|&i| self.faces.get(i))
            .flat_map(|c| (0..4).map(move |k| Seg::new(c[k], c[(k + 1) % 4], gold, 3.6)).chain([Seg::new(c[0], c[2], gold, 1.4), Seg::new(c[1], c[3], gold, 1.4)]))
            .collect();
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            Frame {
                vp,
                view,
                upload: self.upload.take(),
                geo,
                dsegs,
                ssegs,
                low: [self.low[0], self.low[1], self.low[2], self.low_a],
                eye: [eye.x, eye.y, eye.z, diag * self.low_lift * 1e-3],
                res: [rect.width(), rect.height()],
            },
        ));

        // HUD
        let p = ui.painter().with_clip_rect(rect);
        if let Some((a, b)) = self.line {
            p.line_segment([a, b], egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 115, 0)));
        }
        if self.tool == Tool::Brush && !busy {
            if let Some(h) = resp.hover_pos().or(resp.interact_pointer_pos()) {
                p.circle_stroke(h, self.brush_px, egui::Stroke::new(1.5, theme::LAMP));
                p.circle_stroke(h, self.brush_px * self.brush_hard, egui::Stroke::new(1.0, theme::LAMP.gamma_multiply(0.45)));
            }
        }
        let chip = |pos: egui::Pos2, align: egui::Align2, t: &str, col: egui::Color32| {
            let g = p.layout_no_wrap(t.to_string(), theme::typew(12.0), col);
            let r = align.anchor_size(pos, g.size()).expand2(egui::vec2(10.0, 5.0));
            p.rect_filled(r, 4.0, egui::Color32::from_rgba_unmultiplied(20, 14, 10, 220));
            p.rect_stroke(r, 4.0, egui::Stroke::new(1.0, theme::LINE), egui::StrokeKind::Inside);
            p.galley(r.min + egui::vec2(10.0, 5.0), g, col);
        };
        if self.mesh.is_some() {
            if self.patches.len() + self.live_patches.len() > 0 {
                let l = self.low.map(|c| ((c * 0.45 + 0.55) * 255.0) as u8); // lightened so it reads on the dark chip
                chip(rect.left_top() + egui::vec2(16.0, 16.0), egui::Align2::LEFT_TOP, &format!("{} PATCHES{}", self.patches.len() + self.live_patches.len(), if self.live { " · LIVE" } else { "" }), egui::Color32::from_rgb(l[0], l[1], l[2]));
            }
            if self.tool == Tool::Edit {
                chip(rect.center_top() + egui::vec2(0.0, 16.0), egui::Align2::CENTER_TOP, &format!("EDIT · {} FACES", self.fsel.len()), theme::LAMP);
            }
            if self.tool == Tool::Brush {
                chip(rect.center_top() + egui::vec2(0.0, 16.0), egui::Align2::CENTER_TOP, if self.bk == BrushKind::Relax { "BRUSH · RELAX" } else { "BRUSH · GRAB" }, theme::LAMP);
            }
            if self.sym {
                chip(rect.right_top() + egui::vec2(-16.0, 16.0), egui::Align2::RIGHT_TOP, &format!("MIRROR · {} = {:.2}", ["X", "Y", "Z"][self.axis], self.plane), egui::Color32::from_rgb(0x8f, 0xb0, 0xff));
            }
        } else {
            let c = rect.center();
            p.text(c - egui::vec2(0.0, 20.0), egui::Align2::CENTER_CENTER, "NO CASE LOADED", theme::serif(30.0), theme::TITLE);
            p.text(c + egui::vec2(0.0, 16.0), egui::Align2::CENTER_CENTER, "Drop an .obj file here, or use Open with on one. (From Maya: Open viewer.)", theme::typew(13.0), theme::MUTED);
        }
        let tool = match self.tool {
            Tool::Draw => "PENCIL",
            Tool::Edit => "EDIT",
            Tool::Brush if self.bk == BrushKind::Relax => "BRUSH · RELAX",
            Tool::Brush => "BRUSH · GRAB",
        };
        let hint = if self.strokes.is_empty() && self.mesh.is_some() { "  ·  Space menu  ·  P panel" } else { "" };
        p.text(rect.left_bottom() + egui::vec2(14.0, -12.0), egui::Align2::LEFT_CENTER, format!("{tool}{hint}  ·  {}", self.status), theme::mono(10.5), theme::MUTED);
    }
}

fn main() -> eframe::Result {
    let Ok(lock) = TcpListener::bind(LOCK_ADDR) else {
        // wake the running one (and hand it the file we were asked to open), then quit
        if let Ok(mut s) = std::net::TcpStream::connect(LOCK_ADDR) {
            if let Some(a) = std::env::args().nth(1) {
                let abs = std::path::absolute(&a).unwrap_or_else(|_| PathBuf::from(a));
                let _ = std::io::Write::write_all(&mut s, format!("{}\n", abs.display()).as_bytes());
            }
        }
        return Ok(());
    };
    let opts = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        depth_buffer: 32,
        viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 750.0]).with_title("RetopoEZ"),
        ..Default::default()
    };
    eframe::run_native("RetopoEZ", opts, Box::new(move |cc| Ok(Box::new(App::new(cc, lock)))))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_vectors_roundtrip_and_reject_garbage() {
        let v = vec![Vec3::new(1., 2., 3.), Vec3::new(-4., 5.5, 6.)];
        let mut b = vec![];
        put_v(&mut b, &v);
        assert_eq!(Rd(&b).v().unwrap(), v);
        assert!(Rd(&b[..b.len() - 1]).v().is_none()); // truncated file
        assert!(Rd(&[255, 255, 255, 255]).v().is_none()); // absurd length
    }
}
