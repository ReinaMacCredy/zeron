use std::ffi::c_void;
use std::ops::Range;
use std::ptr::{NonNull, null_mut};
use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_video::{
    CVMetalTexture, CVMetalTextureCache, CVMetalTextureGetTexture, CVPixelBuffer,
    CVPixelBufferCreate, kCVMetalTextureUsage, kCVPixelBufferIOSurfacePropertiesKey,
    kCVPixelBufferMetalCompatibilityKey, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
};
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBuffer, MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue,
    MTLCompareFunction, MTLCreateSystemDefaultDevice, MTLDepthStencilDescriptor,
    MTLDepthStencilState, MTLDevice, MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat,
    MTLPrimitiveType, MTLRegion, MTLRenderCommandEncoder, MTLRenderPassDescriptor,
    MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLResourceOptions, MTLSize,
    MTLStorageMode, MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureType,
    MTLTextureUsage,
};

use super::h264::Surface;
use super::scene3d::{RenderKey, Schedule, Vertex};

type Device = ProtocolObject<dyn MTLDevice>;
type Texture = Retained<ProtocolObject<dyn MTLTexture>>;
type Pipeline = Retained<ProtocolObject<dyn MTLRenderPipelineState>>;

const SAMPLES: usize = 4;
const RING: usize = 3;

// Colors stay in the gamma-encoded space the stream uses; ycc is gpui's full-range BT.601 surface matrix.
const SHADERS: &str = r#"
#include <metal_stdlib>
using namespace metal;
struct Vtx { packed_float3 position; packed_float3 normal; packed_float3 color; packed_float2 uv; float screen; };
struct Uniforms { float4x4 mvp; float4x4 model; uint source; float live; uint live_rot; uint retained; uint retained_rot; };
struct VO { float4 position [[position]]; float3 normal; float3 color; float2 uv; float screen; };
vertex VO scene_vertex(uint id [[vertex_id]], const device Vtx* v [[buffer(0)]], constant Uniforms& u [[buffer(1)]]) {
  Vtx x = v[id];
  VO o;
  o.position = u.mvp * float4(float3(x.position), 1);
  o.normal = (u.model * float4(float3(x.normal), 0)).xyz;
  o.color = float3(x.color);
  o.uv = float2(x.uv);
  o.screen = x.screen;
  return o;
}
constant float4x4 ycc = float4x4(float4(1, 1, 1, 0), float4(0, -0.3441, 1.7720, 0), float4(1.4020, -0.7141, 0, 0), float4(-0.7010, 0.5291, -0.8860, 1));
float2 display_point(uint turns, float2 p) {
  switch (turns) {
    case 1: return float2(1 - p.y, p.x);
    case 2: return float2(1 - p.x, 1 - p.y);
    case 3: return float2(p.y, 1 - p.x);
    default: return p;
  }
}
fragment float4 scene_fragment(VO i [[stage_in]], constant Uniforms& u [[buffer(0)]],
    texture2d<float> luma [[texture(0)]], texture2d<float> chroma [[texture(1)]], texture2d<float> bgra [[texture(2)]],
    texture2d<float> kept_luma [[texture(3)]], texture2d<float> kept_chroma [[texture(4)]]) {
  constexpr sampler s(mag_filter::linear, min_filter::linear, address::clamp_to_edge);
  if (i.screen > 0.5) {
    if (abs(i.screen - u.live) >= 0.5) {
      if (u.retained == 0 || abs(i.screen - float(u.retained)) >= 0.5) { return float4(0.0314, 0.0392, 0.0627, 1); }
      float2 kept = display_point(u.retained_rot, i.uv);
      return ycc * float4(kept_luma.sample(s, kept).r, kept_chroma.sample(s, kept).rg, 1);
    }
    float2 uv = display_point(u.live_rot, i.uv);
    if (u.source == 0) { return ycc * float4(luma.sample(s, uv).r, chroma.sample(s, uv).rg, 1); }
    return float4(bgra.sample(s, uv).rgb, 1);
  }
  float3 n = normalize(i.normal);
  float3 key = normalize(float3(-3, 4, 5));
  float light = 0.42 + 0.5 * max(dot(n, key), 0.0) + 0.3 * max(dot(n, normalize(float3(3, 1, -3))), 0.0)
    + 0.15 * max(dot(n, normalize(float3(-2, -2, -4))), 0.0);
  float shine = pow(max(dot(reflect(-key, n), float3(0, 0, 1)), 0.0), 24.0);
  return float4(saturate(i.color * light + 0.25 * shine), 1);
}
struct FO { float4 position [[position]]; };
vertex FO full_vertex(uint id [[vertex_id]]) {
  float2 p = float2((id << 1) & 2, id & 2);
  FO o;
  o.position = float4(p * 2 - 1, 0, 1);
  return o;
}
fragment float to_luma(FO i [[stage_in]], texture2d<float> rgb [[texture(0)]]) {
  return dot(rgb.read(uint2(i.position.xy)).rgb, float3(0.299, 0.587, 0.114));
}
fragment float2 to_chroma(FO i [[stage_in]], texture2d<float> rgb [[texture(0)]]) {
  uint2 p = uint2(i.position.xy) * 2;
  float3 c = (rgb.read(p).rgb + rgb.read(p + uint2(1, 0)).rgb + rgb.read(p + uint2(0, 1)).rgb + rgb.read(p + uint2(1, 1)).rgb) * 0.25;
  float y = dot(c, float3(0.299, 0.587, 0.114));
  return float2((c.b - y) / 1.772 + 0.5, (c.r - y) / 1.402 + 0.5);
}
"#;

#[repr(C)]
struct Uniforms {
    mvp: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    source: u32,
    live: f32,
    live_rotation: u32,
    retained: u32,
    retained_rotation: u32,
    _pad: [u32; 3],
}

pub enum Source<'a> {
    Surface(&'a Surface),
    Bgra {
        bytes: &'a [u8],
        width: usize,
        height: usize,
    },
}

pub struct Draw {
    pub range: Range<usize>,
    pub mvp: [[f32; 4]; 4],
    pub rotation: [[f32; 4]; 4],
}

pub struct RetainedFrame<'a> {
    pub surface: &'a Surface,
    pub panel: u8,
    pub rotation: u8,
    pub frame: usize,
}

pub struct Job<'a> {
    pub source: Source<'a>,
    pub frame: usize,
    pub mesh: (u64, &'a [Vertex]),
    pub draws: &'a [Draw],
    pub live_panel: u8,
    pub live_rotation: u8,
    pub retained: Option<RetainedFrame<'a>>,
    pub size: (usize, usize),
    pub clear: [f32; 3],
}

struct Targets {
    size: (usize, usize),
    msaa: Texture,
    depth: Texture,
    resolve: Texture,
    outputs: Vec<Surface>,
}

struct Shared {
    device: Retained<Device>,
    scene: Pipeline,
    luma: Pipeline,
    chroma: Pipeline,
    depth_state: Retained<ProtocolObject<dyn MTLDepthStencilState>>,
}

static SHARED: OnceLock<Option<Shared>> = OnceLock::new();

pub struct Renderer {
    shared: &'static Shared,
    queue: Retained<ProtocolObject<dyn objc2_metal::MTLCommandQueue>>,
    cache: CFRetained<CVMetalTextureCache>,
    mesh: Option<(u64, Retained<ProtocolObject<dyn MTLBuffer>>)>,
    bgra: Option<(usize, Texture)>,
    targets: Option<Targets>,
    next: usize,
    schedule: Schedule,
    shown: Option<Surface>,
    pub failures: u32,
    pub gpu_ms: f64,
}

fn pixel_buffer(width: usize, height: usize) -> Option<Surface> {
    let io_surface = CFDictionary::<CFString, CFType>::empty();
    // SAFETY: the keys are immutable CoreVideo constants.
    let keys = unsafe {
        [
            kCVPixelBufferIOSurfacePropertiesKey,
            kCVPixelBufferMetalCompatibilityKey,
        ]
    };
    let values: [&CFType; 2] = [io_surface.as_ref(), CFBoolean::new(true).as_ref()];
    let attributes = CFDictionary::from_slices(&keys, &values);
    let mut out: *mut CVPixelBuffer = null_mut();
    // SAFETY: the attribute dictionary and the out pointer outlive the call.
    let status = unsafe {
        CVPixelBufferCreate(
            None,
            width,
            height,
            kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
            Some(attributes.as_opaque()),
            NonNull::from(&mut out),
        )
    };
    let out = NonNull::new(out).filter(|_| status == 0)?;
    // SAFETY: a successful Create returns a +1 reference.
    Some(Surface(unsafe { CFRetained::from_raw(out) }))
}

fn texture(
    device: &Device,
    format: MTLPixelFormat,
    (width, height): (usize, usize),
    samples: usize,
    usage: MTLTextureUsage,
) -> Option<Texture> {
    // SAFETY: the dimensions are non-zero and the format is a plain color or depth format.
    let descriptor = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            format, width, height, false,
        )
    };
    if samples > 1 {
        descriptor.setTextureType(MTLTextureType::Type2DMultisample);
        // SAFETY: four samples are supported by every Metal GPU macOS runs on.
        unsafe { descriptor.setSampleCount(samples) };
    }
    descriptor.setStorageMode(MTLStorageMode::Private);
    descriptor.setUsage(usage);
    device.newTextureWithDescriptor(&descriptor)
}

impl Shared {
    fn compile() -> Option<Self> {
        let device = MTLCreateSystemDefaultDevice()?;
        let library = device
            .newLibraryWithSource_options_error(&NSString::from_str(SHADERS), None)
            .map_err(|error| tracing::warn!(%error, "device 3D shaders did not compile"))
            .ok()?;
        let pipeline = |vertex: &str, fragment: &str, format, depth: bool, samples: usize| {
            let descriptor = MTLRenderPipelineDescriptor::new();
            let vertex = library.newFunctionWithName(&NSString::from_str(vertex))?;
            let fragment = library.newFunctionWithName(&NSString::from_str(fragment))?;
            descriptor.setVertexFunction(Some(&vertex));
            descriptor.setFragmentFunction(Some(&fragment));
            // SAFETY: attachment 0 always exists and the sample count is supported.
            unsafe {
                descriptor
                    .colorAttachments()
                    .objectAtIndexedSubscript(0)
                    .setPixelFormat(format);
                descriptor.setRasterSampleCount(samples);
            }
            if depth {
                descriptor.setDepthAttachmentPixelFormat(MTLPixelFormat::Depth32Float);
            }
            device
                .newRenderPipelineStateWithDescriptor_error(&descriptor)
                .ok()
        };
        let depth = MTLDepthStencilDescriptor::new();
        depth.setDepthCompareFunction(MTLCompareFunction::Less);
        depth.setDepthWriteEnabled(true);
        Some(Self {
            scene: pipeline(
                "scene_vertex",
                "scene_fragment",
                MTLPixelFormat::BGRA8Unorm,
                true,
                SAMPLES,
            )?,
            luma: pipeline("full_vertex", "to_luma", MTLPixelFormat::R8Unorm, false, 1)?,
            chroma: pipeline(
                "full_vertex",
                "to_chroma",
                MTLPixelFormat::RG8Unorm,
                false,
                1,
            )?,
            depth_state: device.newDepthStencilStateWithDescriptor(&depth)?,
            device,
        })
    }
}

impl Renderer {
    pub fn new() -> Option<Self> {
        let shared = SHARED.get_or_init(Shared::compile).as_ref()?;
        let usage = CFNumber::new_i32(
            (MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget).0 as i32,
        );
        // SAFETY: the key is an immutable CoreVideo constant.
        let keys = unsafe { [kCVMetalTextureUsage] };
        let values: [&CFType; 1] = [usage.as_ref()];
        let attributes = CFDictionary::from_slices(&keys, &values);
        let mut cache: *mut CVMetalTextureCache = null_mut();
        // SAFETY: the device, attributes and out pointer outlive the call.
        let status = unsafe {
            CVMetalTextureCache::create(
                None,
                None,
                &shared.device,
                Some(attributes.as_opaque()),
                NonNull::from(&mut cache),
            )
        };
        let cache = NonNull::new(cache).filter(|_| status == 0)?;
        Some(Self {
            queue: shared.device.newCommandQueue()?,
            // SAFETY: a successful Create returns a +1 reference.
            cache: unsafe { CFRetained::from_raw(cache) },
            shared,
            mesh: None,
            bgra: None,
            targets: None,
            next: 0,
            schedule: Schedule::default(),
            shown: None,
            failures: 0,
            gpu_ms: 0.0,
        })
    }

    fn plane(
        &self,
        buffer: &CVPixelBuffer,
        format: MTLPixelFormat,
        (width, height): (usize, usize),
        index: usize,
    ) -> Option<(CFRetained<CVMetalTexture>, Texture)> {
        let mut out: *mut CVMetalTexture = null_mut();
        // SAFETY: the buffer is a live IOSurface-backed NV12 buffer and the plane size matches it.
        let status = unsafe {
            CVMetalTextureCache::create_texture_from_image(
                None,
                &self.cache,
                buffer,
                None,
                format,
                width,
                height,
                index,
                NonNull::from(&mut out),
            )
        };
        let out = NonNull::new(out).filter(|_| status == 0)?;
        // SAFETY: a successful Create returns a +1 reference.
        let wrapper = unsafe { CFRetained::from_raw(out) };
        let texture = CVMetalTextureGetTexture(&wrapper)?;
        Some((wrapper, texture))
    }

    fn ensure_targets(&mut self, size: (usize, usize)) -> Option<()> {
        if self
            .targets
            .as_ref()
            .is_some_and(|targets| targets.size == size)
        {
            return Some(());
        }
        self.targets = None;
        let render = MTLTextureUsage::RenderTarget;
        self.targets = Some(Targets {
            size,
            msaa: texture(
                &self.shared.device,
                MTLPixelFormat::BGRA8Unorm,
                size,
                SAMPLES,
                render,
            )?,
            depth: texture(
                &self.shared.device,
                MTLPixelFormat::Depth32Float,
                size,
                SAMPLES,
                render,
            )?,
            resolve: texture(
                &self.shared.device,
                MTLPixelFormat::BGRA8Unorm,
                size,
                1,
                render | MTLTextureUsage::ShaderRead,
            )?,
            outputs: (0..RING)
                .map(|_| pixel_buffer(size.0, size.1))
                .collect::<Option<_>>()?,
        });
        Some(())
    }

    fn ensure_mesh(&mut self, (id, vertices): (u64, &[Vertex])) -> Option<()> {
        if self.mesh.as_ref().is_some_and(|(known, _)| *known == id) {
            return Some(());
        }
        let bytes = std::mem::size_of_val(vertices);
        let pointer = NonNull::new(vertices.as_ptr().cast_mut().cast::<c_void>())?;
        // SAFETY: `pointer` addresses `bytes` readable bytes; Metal copies them before returning.
        let buffer = unsafe {
            self.shared.device.newBufferWithBytes_length_options(
                pointer,
                bytes,
                MTLResourceOptions::StorageModeShared,
            )
        }?;
        self.mesh = Some((id, buffer));
        Some(())
    }

    fn upload(
        &mut self,
        id: usize,
        bytes: &[u8],
        (width, height): (usize, usize),
    ) -> Option<Texture> {
        if let Some((known, texture)) = &self.bgra
            && *known == id
        {
            return Some(texture.clone());
        }
        if bytes.len() < width * height * 4 {
            return None;
        }
        // SAFETY: the dimensions are non-zero and BGRA8 is a plain color format.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm,
                width,
                height,
                false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::ShaderRead);
        let texture = self.shared.device.newTextureWithDescriptor(&descriptor)?;
        let region = MTLRegion {
            origin: MTLOrigin { x: 0, y: 0, z: 0 },
            size: MTLSize {
                width,
                height,
                depth: 1,
            },
        };
        // SAFETY: `bytes` holds `height` tightly packed rows of `width` BGRA pixels.
        unsafe {
            texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                region,
                0,
                NonNull::new(bytes.as_ptr().cast_mut().cast::<c_void>())?,
                width * 4,
            )
        };
        self.bgra = Some((id, texture.clone()));
        Some(texture)
    }

    pub fn render(&mut self, job: Job) -> Option<Surface> {
        let size = (job.size.0 & !1, job.size.1 & !1);
        if size.0 < 2 || size.1 < 2 {
            return None;
        }
        let mut mvps = [[[0.0; 4]; 4]; 2];
        for (slot, draw) in mvps.iter_mut().zip(job.draws) {
            *slot = draw.mvp;
        }
        let key = RenderKey {
            source: job.frame,
            model: job.mesh.0,
            mvps,
            size,
            clear: job.clear,
            live_panel: job.live_panel,
            rotation: job.live_rotation,
            retained: job
                .retained
                .as_ref()
                .map(|kept| (kept.frame, kept.panel, kept.rotation)),
        };
        if !self.schedule.due(key)
            && let Some(shown) = &self.shown
        {
            return Some(shown.clone());
        }
        let rendered = self.draw(&job, size);
        if rendered.is_none() {
            self.schedule.forget();
            self.failures += 1;
        } else {
            self.failures = 0;
        }
        self.shown = rendered.clone();
        rendered
    }

    fn draw(&mut self, job: &Job, size: (usize, usize)) -> Option<Surface> {
        self.ensure_targets(size)?;
        self.ensure_mesh(job.mesh)?;
        let mut keep = Vec::new();
        let (source, screen) = match &job.source {
            Source::Surface(surface) => {
                let buffer = surface.buffer();
                let (width, height) = (surface.width() as usize, surface.height() as usize);
                let (luma_cv, luma) =
                    self.plane(buffer, MTLPixelFormat::R8Unorm, (width, height), 0)?;
                let (chroma_cv, chroma) =
                    self.plane(buffer, MTLPixelFormat::RG8Unorm, (width / 2, height / 2), 1)?;
                keep.extend([luma_cv, chroma_cv]);
                (0, [luma.clone(), chroma, luma])
            }
            Source::Bgra {
                bytes,
                width,
                height,
            } => {
                let texture = self.upload(job.frame, bytes, (*width, *height))?;
                (1, [texture.clone(), texture.clone(), texture])
            }
        };
        let kept = match &job.retained {
            Some(kept) => {
                let buffer = kept.surface.buffer();
                let (width, height) = (
                    kept.surface.width() as usize,
                    kept.surface.height() as usize,
                );
                let (luma_cv, luma) =
                    self.plane(buffer, MTLPixelFormat::R8Unorm, (width, height), 0)?;
                let (chroma_cv, chroma) =
                    self.plane(buffer, MTLPixelFormat::RG8Unorm, (width / 2, height / 2), 1)?;
                keep.extend([luma_cv, chroma_cv]);
                [luma, chroma]
            }
            None => [screen[0].clone(), screen[1].clone()],
        };
        let targets = self.targets.as_ref()?;
        let output = targets.outputs[self.next].clone();
        self.next = (self.next + 1) % RING;
        let (y_cv, y_plane) = self.plane(output.buffer(), MTLPixelFormat::R8Unorm, size, 0)?;
        let (c_cv, c_plane) = self.plane(
            output.buffer(),
            MTLPixelFormat::RG8Unorm,
            (size.0 / 2, size.1 / 2),
            1,
        )?;
        keep.extend([y_cv, c_cv]);
        let (_, vertices) = self.mesh.as_ref()?;
        let commands = self.queue.commandBuffer()?;
        let pass = MTLRenderPassDescriptor::new();
        // SAFETY: attachment 0 exists; the textures match the pipeline's formats and sample count.
        unsafe {
            let color = pass.colorAttachments().objectAtIndexedSubscript(0);
            color.setTexture(Some(&targets.msaa));
            color.setResolveTexture(Some(&targets.resolve));
            color.setLoadAction(MTLLoadAction::Clear);
            color.setStoreAction(MTLStoreAction::MultisampleResolve);
            color.setClearColor(MTLClearColor {
                red: f64::from(job.clear[0]),
                green: f64::from(job.clear[1]),
                blue: f64::from(job.clear[2]),
                alpha: 1.0,
            });
            let depth = pass.depthAttachment();
            depth.setTexture(Some(&targets.depth));
            depth.setLoadAction(MTLLoadAction::Clear);
            depth.setStoreAction(MTLStoreAction::DontCare);
            depth.setClearDepth(1.0);
        }
        let encoder = commands.renderCommandEncoderWithDescriptor(&pass)?;
        encoder.setRenderPipelineState(&self.shared.scene);
        encoder.setDepthStencilState(Some(&self.shared.depth_state));
        let count = job.mesh.1.len();
        // SAFETY: the vertex buffer holds `count` Vertex records matching the shader's Vtx layout,
        // each draw range lies inside it, and the uniform bytes outlive the encode call that copies them.
        unsafe {
            encoder.setVertexBuffer_offset_atIndex(Some(vertices), 0, 0);
            for (index, texture) in screen.iter().chain(&kept).enumerate() {
                encoder.setFragmentTexture_atIndex(Some(texture), index);
            }
            for draw in job.draws.iter().filter(|draw| draw.range.end <= count) {
                let uniforms = Uniforms {
                    mvp: draw.mvp,
                    model: draw.rotation,
                    source,
                    live: f32::from(job.live_panel),
                    live_rotation: u32::from(job.live_rotation),
                    retained: job
                        .retained
                        .as_ref()
                        .map_or(0, |kept| u32::from(kept.panel)),
                    retained_rotation: job
                        .retained
                        .as_ref()
                        .map_or(0, |kept| u32::from(kept.rotation)),
                    _pad: [0; 3],
                };
                let bytes = NonNull::from(&uniforms).cast::<c_void>();
                encoder.setVertexBytes_length_atIndex(bytes, size_of::<Uniforms>(), 1);
                encoder.setFragmentBytes_length_atIndex(bytes, size_of::<Uniforms>(), 0);
                encoder.drawPrimitives_vertexStart_vertexCount(
                    MTLPrimitiveType::Triangle,
                    draw.range.start,
                    draw.range.len(),
                );
            }
        }
        encoder.endEncoding();
        for (target, pipeline) in [
            (&y_plane, &self.shared.luma),
            (&c_plane, &self.shared.chroma),
        ] {
            let pass = MTLRenderPassDescriptor::new();
            // SAFETY: attachment 0 exists and the target matches the pipeline's format.
            unsafe {
                let color = pass.colorAttachments().objectAtIndexedSubscript(0);
                color.setTexture(Some(target));
                color.setLoadAction(MTLLoadAction::DontCare);
                color.setStoreAction(MTLStoreAction::Store);
            }
            let encoder = commands.renderCommandEncoderWithDescriptor(&pass)?;
            encoder.setRenderPipelineState(pipeline);
            // SAFETY: the resolve texture outlives the command buffer, which is awaited below.
            unsafe {
                encoder.setFragmentTexture_atIndex(Some(&targets.resolve), 0);
                encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3);
            }
            encoder.endEncoding();
        }
        commands.commit();
        commands.waitUntilCompleted();
        self.gpu_ms = (commands.GPUEndTime() - commands.GPUStartTime()) * 1e3;
        drop(keep);
        self.cache.flush(0);
        Some(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device_panel::scene3d::{Camera, IOS_PHONE, Model, Quat, Vec3};
    use objc2_core_video::{
        CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRowOfPlane,
        CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    };
    use zeron_proto::device_hub::DeviceOrientation;

    fn planes(
        surface: &Surface,
        write: bool,
        mut visit: impl FnMut(*mut u8, usize, *mut u8, usize),
    ) {
        let buffer = surface.buffer();
        let flags = CVPixelBufferLockFlags(if write { 0 } else { 1 });
        // SAFETY: the buffer is locked for the duration of the visit and both planes exist in NV12.
        unsafe {
            CVPixelBufferLockBaseAddress(buffer, flags);
            visit(
                CVPixelBufferGetBaseAddressOfPlane(buffer, 0).cast(),
                CVPixelBufferGetBytesPerRowOfPlane(buffer, 0),
                CVPixelBufferGetBaseAddressOfPlane(buffer, 1).cast(),
                CVPixelBufferGetBytesPerRowOfPlane(buffer, 1),
            );
            CVPixelBufferUnlockBaseAddress(buffer, flags);
        }
    }

    fn filled(width: usize, height: usize, color: (u8, u8, u8)) -> Surface {
        let surface = pixel_buffer(width, height).unwrap();
        fill(&surface, color);
        surface
    }

    fn fill(surface: &Surface, (y, cb, cr): (u8, u8, u8)) {
        let (width, height) = (surface.width() as usize, surface.height() as usize);
        planes(surface, true, |luma, luma_stride, chroma, chroma_stride| {
            for row in 0..height {
                // SAFETY: each row holds at least `width` luma bytes.
                unsafe { std::ptr::write_bytes(luma.add(row * luma_stride), y, width) };
            }
            for row in 0..height / 2 {
                for column in 0..width / 2 {
                    // SAFETY: each chroma row holds `width / 2` interleaved CbCr pairs.
                    unsafe {
                        *chroma.add(row * chroma_stride + column * 2) = cb;
                        *chroma.add(row * chroma_stride + column * 2 + 1) = cr;
                    }
                }
            }
        });
    }

    fn sample(surface: &Surface, x: usize, y: usize) -> (u8, u8, u8) {
        let mut out = (0, 0, 0);
        planes(
            surface,
            false,
            |luma, luma_stride, chroma, chroma_stride| {
                // SAFETY: (x, y) lies inside the buffer.
                out = unsafe {
                    let at = chroma.add((y / 2) * chroma_stride + (x / 2) * 2);
                    (*luma.add(y * luma_stride + x), *at, *at.add(1))
                };
            },
        );
        out
    }

    fn near(actual: (u8, u8, u8), expected: (f32, f32, f32), tolerance: f32) -> bool {
        let d = |a: u8, b: f32| (f32::from(a) - b).abs() <= tolerance;
        d(actual.0, expected.0) && d(actual.1, expected.1) && d(actual.2, expected.2)
    }

    fn draws(model: &Model, pose: Quat, size: (usize, usize)) -> [Draw; 1] {
        let rotation = model.rotation(pose);
        let camera = Camera::fit(model.bounds(None), rotation, size.0 as f64 / size.1 as f64);
        let (mvp, rotation) = camera.matrices(rotation);
        [Draw {
            range: 0..model.vertices.len(),
            mvp,
            rotation,
        }]
    }

    fn job<'a>(
        source: Source<'a>,
        frame: usize,
        model: &'a Model,
        draws: &'a [Draw],
        size: (usize, usize),
    ) -> Job<'a> {
        Job {
            source,
            frame,
            mesh: (model.id, &model.vertices),
            draws,
            live_panel: 1,
            live_rotation: 0,
            retained: None,
            size,
            clear: [0.2, 0.3, 0.4],
        }
    }

    #[test]
    fn renders_the_live_frame_onto_the_screen_of_an_opaque_nv12_buffer() {
        let mut renderer = Renderer::new().expect("a Metal device");
        let source = filled(590, 1280, (150, 90, 170));
        let model = Model::new(&IOS_PHONE, 590.0 / 1280.0, DeviceOrientation::Portrait);
        let size = (800, 1000);
        let first = renderer
            .render(job(
                Source::Surface(&source),
                1,
                &model,
                &draws(&model, Quat::IDENTITY, size),
                size,
            ))
            .unwrap();
        assert_eq!((first.width(), first.height()), (800, 1000));
        assert!(
            near(sample(&first, 400, 500), (150.0, 90.0, 170.0), 2.0),
            "{:?}",
            sample(&first, 400, 500)
        );
        let clear_y = 255.0 * (0.299 * 0.2 + 0.587 * 0.3 + 0.114 * 0.4);
        let clear = (
            clear_y,
            255.0 * ((0.4 - clear_y / 255.0) / 1.772 + 0.5),
            255.0 * ((0.2 - clear_y / 255.0) / 1.402 + 0.5),
        );
        assert!(
            near(sample(&first, 2, 2), clear, 1.5),
            "{:?} vs {clear:?}",
            sample(&first, 2, 2)
        );

        let again = renderer
            .render(job(
                Source::Surface(&source),
                1,
                &model,
                &draws(&model, Quat::IDENTITY, size),
                size,
            ))
            .unwrap();
        assert_eq!(
            again.buffer() as *const _,
            first.buffer() as *const _,
            "an unchanged key reuses the buffer"
        );
        let turned = Quat::axis_angle(Vec3::new(0.0, 1.0, 0.0), 0.5);
        let next = renderer
            .render(job(
                Source::Surface(&source),
                1,
                &model,
                &draws(&model, turned, size),
                size,
            ))
            .unwrap();
        assert_ne!(
            next.buffer() as *const _,
            first.buffer() as *const _,
            "a new pose renders into the next buffer"
        );

        let bgra: Vec<u8> = [64u8, 128, 192, 255].repeat(40 * 80);
        let image = renderer
            .render(job(
                Source::Bgra {
                    bytes: &bgra,
                    width: 40,
                    height: 80,
                },
                2,
                &model,
                &draws(&model, Quat::IDENTITY, size),
                size,
            ))
            .unwrap();
        let (r, g, b) = (192.0 / 255.0, 128.0 / 255.0, 64.0 / 255.0);
        let y = 0.299 * r + 0.587 * g + 0.114 * b;
        let expected = (
            255.0 * y,
            255.0 * ((b - y) / 1.772 + 0.5),
            255.0 * ((r - y) / 1.402 + 0.5),
        );
        assert!(
            near(sample(&image, 400, 500), expected, 2.0),
            "{:?} vs {expected:?}",
            sample(&image, 400, 500)
        );
    }

    #[test]
    fn per_frame_cost_of_a_turning_model() {
        let mut renderer = Renderer::new().expect("a Metal device");
        let source = filled(590, 1280, (150, 90, 170));
        let model = Model::new(&IOS_PHONE, 590.0 / 1280.0, DeviceOrientation::Portrait);
        for size in [(800, 1000), (1600, 2000)] {
            let (mut gpu, mut wall) = (Vec::new(), Vec::new());
            for frame in 0..120 {
                let pose = Quat::axis_angle(Vec3::new(0.0, 1.0, 0.0), frame as f64 * 0.01);
                let started = std::time::Instant::now();
                renderer
                    .render(job(
                        Source::Surface(&source),
                        1,
                        &model,
                        &draws(&model, pose, size),
                        size,
                    ))
                    .unwrap();
                if frame >= 20 {
                    wall.push(started.elapsed().as_secs_f64() * 1e3);
                    gpu.push(renderer.gpu_ms);
                }
            }
            let mean = |values: &[f64]| values.iter().sum::<f64>() / values.len() as f64;
            eprintln!(
                "device 3D render {}x{}: GPU {:.3} ms, wall incl. wait {:.3} ms (means of {})",
                size.0,
                size.1,
                mean(&gpu),
                mean(&wall),
                gpu.len()
            );
            assert!(mean(&wall) < 16.0);
        }
    }

    #[test]
    fn a_decoder_buffer_reused_for_a_new_frame_renders_the_new_picture() {
        let mut renderer = Renderer::new().expect("a Metal device");
        let source = filled(590, 1280, (150, 90, 170));
        let model = Model::new(&IOS_PHONE, 590.0 / 1280.0, DeviceOrientation::Portrait);
        let size = (800, 1000);
        renderer
            .render(job(
                Source::Surface(&source),
                1,
                &model,
                &draws(&model, Quat::IDENTITY, size),
                size,
            ))
            .unwrap();
        fill(&source, (60, 140, 100));
        let next = renderer
            .render(job(
                Source::Surface(&source),
                2,
                &model,
                &draws(&model, Quat::IDENTITY, size),
                size,
            ))
            .unwrap();
        assert!(
            near(sample(&next, 400, 500), (60.0, 140.0, 100.0), 2.0),
            "{:?}",
            sample(&next, 400, 500)
        );
    }

    fn quadrants(width: usize, height: usize, colors: [(u8, u8, u8); 4]) -> Surface {
        let surface = pixel_buffer(width, height).unwrap();
        let color = |x: usize, y: usize| {
            colors[usize::from(x >= width / 2) + 2 * usize::from(y >= height / 2)]
        };
        planes(
            &surface,
            true,
            |luma, luma_stride, chroma, chroma_stride| {
                for y in 0..height {
                    for x in 0..width {
                        // SAFETY: (x, y) lies inside the luma plane.
                        unsafe { *luma.add(y * luma_stride + x) = color(x, y).0 };
                    }
                }
                for y in 0..height / 2 {
                    for x in 0..width / 2 {
                        let (_, cb, cr) = color(x * 2, y * 2);
                        // SAFETY: (x, y) lies inside the half-size chroma plane.
                        unsafe {
                            *chroma.add(y * chroma_stride + x * 2) = cb;
                            *chroma.add(y * chroma_stride + x * 2 + 1) = cr;
                        }
                    }
                }
            },
        );
        surface
    }

    fn quad(left: f32, right: f32, tag: f32) -> [Vertex; 6] {
        let corner = |x: f32, y: f32, u: f32, v: f32| Vertex {
            position: [x, y, 0.5],
            normal: [0.0, 0.0, 1.0],
            color: [0.0; 3],
            uv: [u, v],
            screen: tag,
        };
        let (tl, tr, bl, br) = (
            corner(left, 1.0, 0.0, 0.0),
            corner(right, 1.0, 1.0, 0.0),
            corner(left, -1.0, 0.0, 1.0),
            corner(right, -1.0, 1.0, 1.0),
        );
        [tl, tr, bl, tr, br, bl]
    }

    fn rgb_to_nv12((r, g, b): (f32, f32, f32)) -> (f32, f32, f32) {
        let y = 0.299 * r + 0.587 * g + 0.114 * b;
        (
            255.0 * y,
            255.0 * ((b - y) / 1.772 + 0.5),
            255.0 * ((r - y) / 1.402 + 0.5),
        )
    }

    #[test]
    fn two_draws_over_one_buffer_show_the_live_panel_turned_and_the_other_panel_dark() {
        assert_eq!(size_of::<Uniforms>(), 160, "matches the Metal struct");
        let mut renderer = Renderer::new().expect("a Metal device");
        let colors = [
            (150, 90, 170),
            (100, 160, 110),
            (180, 128, 128),
            (90, 128, 90),
        ];
        let source = quadrants(64, 64, colors);
        let vertices: Vec<Vertex> = quad(-1.0, 0.0, 3.0)
            .into_iter()
            .chain(quad(0.0, 1.0, 1.0))
            .collect();
        let identity = Quat::IDENTITY.matrix();
        let draws = [0..6, 6..12].map(|range| Draw {
            range,
            mvp: identity,
            rotation: identity,
        });
        let size = (200, 100);
        let near = |actual: (u8, u8, u8), expected: (u8, u8, u8)| {
            near(
                actual,
                (
                    f32::from(expected.0),
                    f32::from(expected.1),
                    f32::from(expected.2),
                ),
                2.0,
            )
        };
        for (turns, live_quadrant) in [(0, 0), (1, 1)] {
            let output = renderer
                .render(Job {
                    source: Source::Surface(&source),
                    frame: 1,
                    mesh: (u64::MAX, &vertices),
                    draws: &draws,
                    live_panel: 3,
                    live_rotation: turns,
                    retained: None,
                    size,
                    clear: [1.0, 1.0, 1.0],
                })
                .unwrap();
            let live = sample(&output, 24, 24);
            assert!(
                near(live, colors[live_quadrant]),
                "turns {turns}: {live:?} vs {:?}",
                colors[live_quadrant]
            );
            let dark = rgb_to_nv12((8.0 / 255.0, 10.0 / 255.0, 16.0 / 255.0));
            assert!(
                super::tests::near(sample(&output, 150, 50), dark, 2.0),
                "{:?} vs {dark:?}",
                sample(&output, 150, 50)
            );
        }
    }

    #[test]
    fn the_inactive_display_shows_its_retained_frame_turned_by_its_own_rotation() {
        let mut renderer = Renderer::new().expect("a Metal device");
        let live = filled(64, 64, (150, 90, 170));
        let colors = [
            (150, 90, 170),
            (100, 160, 110),
            (180, 128, 128),
            (90, 128, 90),
        ];
        let kept = quadrants(64, 64, colors);
        let vertices: Vec<Vertex> = quad(-1.0, 0.0, 3.0)
            .into_iter()
            .chain(quad(0.0, 1.0, 1.0))
            .collect();
        let identity = Quat::IDENTITY.matrix();
        let draws = [0..6, 6..12].map(|range| Draw {
            range,
            mvp: identity,
            rotation: identity,
        });
        for (turns, quadrant) in [(0, 0), (2, 3)] {
            let output = renderer
                .render(Job {
                    source: Source::Surface(&live),
                    frame: 1,
                    mesh: (u64::MAX - 1, &vertices),
                    draws: &draws,
                    live_panel: 3,
                    live_rotation: 0,
                    retained: Some(RetainedFrame {
                        surface: &kept,
                        panel: 1,
                        rotation: turns,
                        frame: 7,
                    }),
                    size: (200, 100),
                    clear: [1.0, 1.0, 1.0],
                })
                .unwrap();
            let (y, cb, cr) = colors[quadrant];
            let expected = (f32::from(y), f32::from(cb), f32::from(cr));
            assert!(
                near(sample(&output, 124, 24), expected, 2.0),
                "turns {turns}: {:?} vs {expected:?}",
                sample(&output, 124, 24)
            );
            assert!(near(sample(&output, 24, 24), (150.0, 90.0, 170.0), 2.0));
        }
    }

    #[test]
    fn a_second_renderer_reuses_the_shaders_the_first_one_compiled() {
        let first = Renderer::new().expect("a Metal device");
        let second = Renderer::new().expect("a Metal device");
        assert!(std::ptr::eq(first.shared, second.shared));
        assert!(!std::ptr::eq(&*first.queue, &*second.queue));
    }
}
