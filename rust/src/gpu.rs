//! wgpu side: high mesh, low-poly faces and instanced thick lines. Static geometry lives in GPU
//! buffers and is only re-uploaded when `Geo` changes; per frame only a small uniform is written.
use eframe::egui_wgpu::{self, CallbackResources};
use eframe::wgpu::{self, util::DeviceExt};
use eframe::egui;
use glam::{Mat4, Vec3};
use std::sync::Arc;

const SHADER: &str = include_str!("shader.wgsl");

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub nrm: [f32; 3],
}

/// One line segment; the vertex shader expands it to a quad `w` pixels wide.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Seg {
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub col: [f32; 3],
    pub w: f32,
}

impl Seg {
    pub fn new(a: Vec3, b: Vec3, col: [f32; 3], w: f32) -> Seg {
        Seg { a: a.to_array(), b: b.to_array(), col, w }
    }
}

/// Everything that only changes when strokes / patches / colours change.
#[derive(Default)]
pub struct Geo {
    pub segs: Vec<Seg>,  // strokes, symmetry plane
    pub psegs: Vec<Seg>, // low-poly edges (floated over the high mesh)
    pub faces: Vec<Vertex>,
}

pub fn gpu_vertices(m: &crate::geom::Mesh) -> Vec<Vertex> {
    let mut n = vec![Vec3::ZERO; m.pos.len()];
    for t in m.idx.chunks_exact(3) {
        let (a, b, c) = (m.pos[t[0] as usize], m.pos[t[1] as usize], m.pos[t[2] as usize]);
        let f = (b - a).cross(c - a); // area-weighted
        for &i in t {
            n[i as usize] += f;
        }
    }
    m.pos.iter().zip(&n).map(|(p, n)| Vertex { pos: p.to_array(), nrm: n.normalize_or_zero().to_array() }).collect()
}

type VBuf = Option<(wgpu::Buffer, u32)>;

pub struct Gpu {
    mesh_pipe: wgpu::RenderPipeline,
    face_pipe: wgpu::RenderPipeline,
    seg_pipe: wgpu::RenderPipeline,
    seg_low_pipe: wgpu::RenderPipeline,
    ubuf: wgpu::Buffer,
    bind: wgpu::BindGroup,
    mesh: Option<(wgpu::Buffer, wgpu::Buffer, u32)>,
    faces: VBuf,
    segs: VBuf,
    psegs: VBuf,
    dsegs: VBuf, // the stroke being drawn: rebuilt every frame, tiny
    ssegs: VBuf, // selected faces (lifted like the low-poly edges): rebuilt every frame while any are selected
}

impl Gpu {
    pub fn new(rs: &egui_wgpu::RenderState) -> Self {
        let dev = &rs.device;
        let module = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("retopoez"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let ubuf = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("u"),
            size: 176,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let bind = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }],
        });
        let layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let vattrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let vbl = wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &vattrs };
        let sattrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32];
        let sbl = wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Seg>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &sattrs };
        let mk = |vs: &str, fs: &str, bl: &wgpu::VertexBufferLayout, bias: i32, blend: bool| {
            dev.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(vs),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some(vs), compilation_options: Default::default(), buffers: &[Some(bl.clone())] },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float, // = NativeOptions.depth_buffer 32
                    depth_write_enabled: Some(!blend),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState { constant: bias, slope_scale: if bias > 0 { 1.0 } else { 0.0 }, clamp: 0.0 },
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: rs.target_format,
                        blend: blend.then_some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Gpu {
            mesh_pipe: mk("vs", "fs_mesh", &vbl, 2, false), // mesh pushed back so strokes win
            face_pipe: mk("vs_low", "fs_low", &vbl, 0, true), // low-poly fill, floated over the high mesh
            seg_pipe: mk("vs_seg", "fs_line", &sbl, 0, false),
            seg_low_pipe: mk("vs_seg_low", "fs_line", &sbl, 0, false),
            ubuf,
            bind,
            mesh: None,
            faces: None,
            segs: None,
            psegs: None,
            dsegs: None,
            ssegs: None,
        }
    }
}

fn buf<T: bytemuck::Pod>(dev: &wgpu::Device, label: &str, v: &[T]) -> VBuf {
    (!v.is_empty()).then(|| {
        let b = dev.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: bytemuck::cast_slice(v), usage: wgpu::BufferUsages::VERTEX });
        (b, v.len() as u32)
    })
}

pub struct Frame {
    pub vp: Mat4,
    pub view: Mat4,
    pub upload: Option<Arc<(Vec<Vertex>, Vec<u32>)>>,
    pub geo: Option<Arc<Geo>>, // Some only when the geometry changed
    pub dsegs: Vec<Seg>,
    pub ssegs: Vec<Seg>,
    pub low: [f32; 4],
    pub eye: [f32; 4],
    pub res: [f32; 2],
}

impl egui_wgpu::CallbackTrait for Frame {
    fn prepare(&self, device: &wgpu::Device, queue: &wgpu::Queue, _sd: &egui_wgpu::ScreenDescriptor, _enc: &mut wgpu::CommandEncoder, res: &mut CallbackResources) -> Vec<wgpu::CommandBuffer> {
        let g: &mut Gpu = res.get_mut().unwrap();
        let mut u = [0f32; 44];
        u[..16].copy_from_slice(&self.vp.to_cols_array());
        u[16..32].copy_from_slice(&self.view.to_cols_array());
        u[32..36].copy_from_slice(&self.low);
        u[36..40].copy_from_slice(&self.eye);
        u[40..42].copy_from_slice(&self.res);
        queue.write_buffer(&g.ubuf, 0, bytemuck::cast_slice(&u));
        if let Some(up) = &self.upload {
            let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("vb"), contents: bytemuck::cast_slice(&up.0), usage: wgpu::BufferUsages::VERTEX });
            let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("ib"), contents: bytemuck::cast_slice(&up.1), usage: wgpu::BufferUsages::INDEX });
            g.mesh = Some((vb, ib, up.1.len() as u32));
        }
        if let Some(geo) = &self.geo {
            g.faces = buf(device, "faces", &geo.faces);
            g.segs = buf(device, "segs", &geo.segs);
            g.psegs = buf(device, "psegs", &geo.psegs);
        }
        g.dsegs = buf(device, "dsegs", &self.dsegs);
        g.ssegs = buf(device, "ssegs", &self.ssegs);
        Vec::new()
    }

    fn paint(&self, _i: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, res: &CallbackResources) {
        let g: &Gpu = res.get().unwrap();
        pass.set_bind_group(0, &g.bind, &[]);
        if let Some((vb, ib, n)) = &g.mesh {
            pass.set_pipeline(&g.mesh_pipe);
            pass.set_vertex_buffer(0, vb.slice(..));
            pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..*n, 0, 0..1);
        }
        if let Some((fb, n)) = &g.faces {
            pass.set_pipeline(&g.face_pipe);
            pass.set_vertex_buffer(0, fb.slice(..));
            pass.draw(0..*n, 0..1);
        }
        pass.set_pipeline(&g.seg_low_pipe);
        for (b, n) in [&g.psegs, &g.ssegs].into_iter().flatten() {
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..6, 0..*n);
        }
        pass.set_pipeline(&g.seg_pipe);
        for (b, n) in [&g.segs, &g.dsegs].into_iter().flatten() {
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..6, 0..*n);
        }
    }
}

#[cfg(test)]
mod tests {
    /// The shader only meets the GPU driver at runtime; validate it here so a typo fails `cargo test`, not the window.
    #[test]
    fn shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(super::SHADER).unwrap_or_else(|e| panic!("{}", e.emit_to_string(super::SHADER)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).expect("valid");
        for entry in ["vs", "vs_low", "vs_seg", "vs_seg_low", "fs_mesh", "fs_low", "fs_line"] {
            assert!(module.entry_points.iter().any(|e| e.name == entry), "{entry}");
        }
    }
}
