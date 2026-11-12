use std::ops::Range;

use zgx_gpu::viewport::ViewportFrame;
use zgx_gpu::wgpu;

use super::scene3d::Vertex;

const SAMPLES: u32 = 4;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SLOT: u64 = 256;

// gpui's metal3d.rs shader: colours stay in the gamma-encoded space the stream uses.
const SHADER: &str = r#"
struct Uniforms { mvp: mat4x4<f32>, model: mat4x4<f32>, live: f32, live_rot: u32, retained: u32, retained_rot: u32 }
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var live_tex: texture_2d<f32>;
@group(0) @binding(2) var kept_tex: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
struct VIn { @location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec3<f32>, @location(3) uv: vec2<f32>, @location(4) screen: f32 }
struct VOut { @builtin(position) position: vec4<f32>, @location(0) normal: vec3<f32>, @location(1) color: vec3<f32>, @location(2) uv: vec2<f32>, @location(3) screen: f32 }
@vertex fn scene_vertex(v: VIn) -> VOut {
  var o: VOut;
  o.position = u.mvp * vec4(v.position, 1.0);
  o.normal = (u.model * vec4(v.normal, 0.0)).xyz;
  o.color = v.color;
  o.uv = v.uv;
  o.screen = v.screen;
  return o;
}
fn display_point(turns: u32, p: vec2<f32>) -> vec2<f32> {
  switch turns {
    case 1u: { return vec2(1.0 - p.y, p.x); }
    case 2u: { return vec2(1.0 - p.x, 1.0 - p.y); }
    case 3u: { return vec2(p.y, 1.0 - p.x); }
    default: { return p; }
  }
}
@fragment fn scene_fragment(i: VOut) -> @location(0) vec4<f32> {
  let live = textureSample(live_tex, samp, display_point(u.live_rot, i.uv)).rgb;
  let kept = textureSample(kept_tex, samp, display_point(u.retained_rot, i.uv)).rgb;
  if (i.screen > 0.5) {
    if (abs(i.screen - u.live) >= 0.5) {
      if (u.retained == 0u || abs(i.screen - f32(u.retained)) >= 0.5) { return vec4(0.0314, 0.0392, 0.0627, 1.0); }
      return vec4(kept, 1.0);
    }
    return vec4(live, 1.0);
  }
  let n = normalize(i.normal);
  let key = normalize(vec3(-3.0, 4.0, 5.0));
  let light = 0.42 + 0.5 * max(dot(n, key), 0.0) + 0.3 * max(dot(n, normalize(vec3(3.0, 1.0, -3.0))), 0.0)
    + 0.15 * max(dot(n, normalize(vec3(-2.0, -2.0, -4.0))), 0.0);
  let shine = pow(max(dot(reflect(-key, n), vec3(0.0, 0.0, 1.0)), 0.0), 24.0);
  return vec4(clamp(i.color * light + 0.25 * shine, vec3(0.0), vec3(1.0)), 1.0);
}
"#;

pub struct Draw {
    pub range: Range<usize>,
    pub mvp: [[f32; 4]; 4],
    pub rotation: [[f32; 4]; 4],
}

pub struct Picture<'a> {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
}

pub struct Job<'a> {
    pub live: Picture<'a>,
    pub mesh: (u64, &'a [Vertex]),
    pub draws: &'a [Draw],
    pub live_panel: u8,
    pub live_rotation: u8,
    pub retained: Option<(Picture<'a>, u8, u8)>,
}

struct Texture {
    id: u64,
    size: (u32, u32),
    view: wgpu::TextureView,
}

pub struct Gpu {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    mesh: Option<(u64, usize, wgpu::Buffer)>,
    live: Option<Texture>,
    kept: Option<Texture>,
    blank: wgpu::TextureView,
    targets: Option<((u32, u32), wgpu::TextureFormat, wgpu::TextureView, wgpu::TextureView)>,
}

fn vertex_bytes(vertices: &[Vertex]) -> &[u8] {
    // SAFETY: Vertex is repr(C) and all f32 (twelve of them, 48 bytes), so it has no padding.
    unsafe { std::slice::from_raw_parts(vertices.as_ptr().cast::<u8>(), std::mem::size_of_val(vertices)) }
}

fn upload(device: &wgpu::Device, queue: &wgpu::Queue, kept: &mut Option<Texture>, picture: &Picture<'_>) {
    let size = (picture.width, picture.height);
    if kept.as_ref().is_some_and(|t| t.id == picture.id) {
        return;
    }
    let extent = wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 };
    if kept.as_ref().is_none_or(|t| t.size != size) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("device screen"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        *kept = Some(Texture { id: 0, size, view: texture.create_view(&Default::default()) });
    }
    let target = kept.as_mut().expect("made above");
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: target.view.texture(),
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        picture.rgba,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size.0 * 4), rows_per_image: Some(size.1) },
        extent,
    );
    target.id = picture.id;
}

impl Gpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("device 3D"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("device 3D uniforms"),
            size: SLOT * 2,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("device 3D"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(144),
                    },
                    count: None,
                },
                texture_entry(1),
                texture_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("device 3D"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("device 3D"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("device 3D"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            })),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("scene_vertex"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x2, 4 => Float32],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("scene_fragment"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: SAMPLES, ..Default::default() },
            multiview_mask: None,
            cache: None,
        });
        let mut blank = None;
        upload(device, queue, &mut blank, &Picture { id: u64::MAX, width: 1, height: 1, rgba: &[8, 10, 16, 255] });
        Self {
            pipeline,
            layout,
            sampler,
            uniforms,
            mesh: None,
            live: None,
            kept: None,
            blank: blank.expect("uploaded").view,
            targets: None,
        }
    }

    pub fn render(&mut self, frame: &mut ViewportFrame<'_>, job: &Job<'_>) {
        let (device, queue, size) = (frame.device, frame.queue, frame.size);
        let format = frame.format;
        if self.targets.as_ref().is_none_or(|(at, f, ..)| *at != size || *f != format) {
            let texture = |format, label| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: SAMPLES,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    })
                    .create_view(&Default::default())
            };
            self.targets = Some((size, format, texture(format, "device 3D colour"), texture(DEPTH, "device 3D depth")));
        }
        if self.mesh.as_ref().is_none_or(|(id, ..)| *id != job.mesh.0) {
            use wgpu::util::DeviceExt;
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("device 3D mesh"),
                contents: vertex_bytes(job.mesh.1),
                usage: wgpu::BufferUsages::VERTEX,
            });
            self.mesh = Some((job.mesh.0, job.mesh.1.len(), buffer));
        }
        upload(device, queue, &mut self.live, &job.live);
        if let Some((kept, ..)) = &job.retained {
            upload(device, queue, &mut self.kept, kept);
        }
        let (retained, retained_rot) = job.retained.as_ref().map_or((0, 0), |(_, panel, rot)| (u32::from(*panel), u32::from(*rot)));
        let mut slots = vec![0u8; (SLOT * 2) as usize];
        for (n, draw) in job.draws.iter().take(2).enumerate() {
            let mut floats = [0f32; 36];
            for c in 0..4 {
                floats[c * 4..c * 4 + 4].copy_from_slice(&draw.mvp[c]);
                floats[16 + c * 4..16 + c * 4 + 4].copy_from_slice(&draw.rotation[c]);
            }
            floats[32] = f32::from(job.live_panel);
            floats[33] = f32::from_bits(u32::from(job.live_rotation));
            floats[34] = f32::from_bits(retained);
            floats[35] = f32::from_bits(retained_rot);
            let at = n * SLOT as usize;
            for (i, f) in floats.iter().enumerate() {
                slots[at + i * 4..at + i * 4 + 4].copy_from_slice(&f.to_ne_bytes());
            }
        }
        queue.write_buffer(&self.uniforms, 0, &slots);
        let live = &self.live.as_ref().expect("uploaded").view;
        let kept = self.kept.as_ref().filter(|_| job.retained.is_some()).map_or(&self.blank, |t| &t.view);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("device 3D"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.uniforms,
                        offset: 0,
                        size: wgpu::BufferSize::new(144),
                    }),
                },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(live) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(kept) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        let (_, _, colour, depth) = self.targets.as_ref().expect("made above");
        let (_, count, mesh) = self.mesh.as_ref().expect("made above");
        let mut pass = frame.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("device 3D"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: colour,
                depth_slice: None,
                resolve_target: Some(frame.target),
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Discard },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, mesh.slice(..));
        for (n, draw) in job.draws.iter().take(2).enumerate() {
            let end = draw.range.end.min(*count);
            if draw.range.start >= end {
                continue;
            }
            pass.set_bind_group(0, &bind, &[(n as u64 * SLOT) as u32]);
            pass.draw(draw.range.start as u32..end as u32, 0..1);
        }
    }
}
