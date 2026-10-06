use std::ffi::c_void;
use std::ptr::{NonNull, null_mut};

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
use super::scene3d::{Model, RenderKey, Schedule};

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
struct Uniforms { float4x4 mvp; float4x4 model; uint source; };
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
fragment float4 scene_fragment(VO i [[stage_in]], constant Uniforms& u [[buffer(0)]],
    texture2d<float> luma [[texture(0)]], texture2d<float> chroma [[texture(1)]], texture2d<float> bgra [[texture(2)]]) {
  constexpr sampler s(mag_filter::linear, min_filter::linear, address::clamp_to_edge);
  if (i.screen > 0.5) {
    if (u.source == 0) { return ycc * float4(luma.sample(s, i.uv).r, chroma.sample(s, i.uv).rg, 1); }
    return float4(bgra.sample(s, i.uv).rgb, 1);
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
    _pad: [u32; 3],
}

pub enum Source<'a> {
    Surface(&'a Surface),
    Bgra {
        id: usize,
        bytes: &'a [u8],
        width: usize,
        height: usize,
    },
}

impl Source<'_> {
    fn id(&self) -> usize {
        match self {
            Source::Surface(surface) => CFRetained::as_ptr(&surface.0).as_ptr() as usize,
            Source::Bgra { id, .. } => *id,
        }
    }
}

pub struct Job<'a> {
    pub source: Source<'a>,
    pub model: &'a Model,
    pub mvp: [[f32; 4]; 4],
    pub rotation: [[f32; 4]; 4],
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

pub struct Renderer {
    device: Retained<Device>,
    queue: Retained<ProtocolObject<dyn objc2_metal::MTLCommandQueue>>,
    scene: Pipeline,
    luma: Pipeline,
    chroma: Pipeline,
    depth_state: Retained<ProtocolObject<dyn MTLDepthStencilState>>,
    cache: CFRetained<CVMetalTextureCache>,
    mesh: Option<(u64, Retained<ProtocolObject<dyn MTLBuffer>>, usize)>,
    bgra: Option<(usize, Texture)>,
    targets: Option<Targets>,
    next: usize,
    schedule: Schedule,
    shown: Option<Surface>,
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

impl Renderer {
    pub fn new() -> Option<Self> {
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
                &device,
                Some(attributes.as_opaque()),
                NonNull::from(&mut cache),
            )
        };
        let cache = NonNull::new(cache).filter(|_| status == 0)?;
        Some(Self {
            queue: device.newCommandQueue()?,
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
            // SAFETY: a successful Create returns a +1 reference.
            cache: unsafe { CFRetained::from_raw(cache) },
            device,
            mesh: None,
            bgra: None,
            targets: None,
            next: 0,
            schedule: Schedule::default(),
            shown: None,
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
                &self.device,
                MTLPixelFormat::BGRA8Unorm,
                size,
                SAMPLES,
                render,
            )?,
            depth: texture(
                &self.device,
                MTLPixelFormat::Depth32Float,
                size,
                SAMPLES,
                render,
            )?,
            resolve: texture(
                &self.device,
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

    fn ensure_mesh(&mut self, model: &Model) -> Option<()> {
        if self.mesh.as_ref().is_some_and(|(id, ..)| *id == model.id) {
            return Some(());
        }
        let bytes = std::mem::size_of_val(model.vertices.as_slice());
        let pointer = NonNull::new(model.vertices.as_ptr().cast_mut().cast::<c_void>())?;
        // SAFETY: `pointer` addresses `bytes` readable bytes; Metal copies them before returning.
        let buffer = unsafe {
            self.device.newBufferWithBytes_length_options(
                pointer,
                bytes,
                MTLResourceOptions::StorageModeShared,
            )
        }?;
        self.mesh = Some((model.id, buffer, model.vertices.len()));
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
        let texture = self.device.newTextureWithDescriptor(&descriptor)?;
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
        let key = RenderKey {
            source: job.source.id(),
            model: job.model.id,
            mvp: job.mvp,
            size,
            clear: job.clear,
        };
        if !self.schedule.due(key)
            && let Some(shown) = &self.shown
        {
            return Some(shown.clone());
        }
        let rendered = self.draw(&job, size);
        if rendered.is_none() {
            self.schedule.forget();
        }
        self.shown = rendered.clone();
        rendered
    }

    fn draw(&mut self, job: &Job, size: (usize, usize)) -> Option<Surface> {
        self.ensure_targets(size)?;
        self.ensure_mesh(job.model)?;
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
                id,
                bytes,
                width,
                height,
            } => {
                let texture = self.upload(*id, bytes, (*width, *height))?;
                (1, [texture.clone(), texture.clone(), texture])
            }
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
        let (_, vertices, count) = self.mesh.as_ref()?;
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
        encoder.setRenderPipelineState(&self.scene);
        encoder.setDepthStencilState(Some(&self.depth_state));
        let uniforms = Uniforms {
            mvp: job.mvp,
            model: job.rotation,
            source,
            _pad: [0; 3],
        };
        // SAFETY: the vertex buffer holds `count` Vertex records matching the shader's Vtx layout,
        // and the uniform bytes outlive the encode call that copies them.
        unsafe {
            encoder.setVertexBuffer_offset_atIndex(Some(vertices), 0, 0);
            let bytes = NonNull::from(&uniforms).cast::<c_void>();
            encoder.setVertexBytes_length_atIndex(bytes, size_of::<Uniforms>(), 1);
            encoder.setFragmentBytes_length_atIndex(bytes, size_of::<Uniforms>(), 0);
            for (index, texture) in screen.iter().enumerate() {
                encoder.setFragmentTexture_atIndex(Some(texture), index);
            }
            encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, *count);
        }
        encoder.endEncoding();
        for (target, pipeline) in [(&y_plane, &self.luma), (&c_plane, &self.chroma)] {
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
    use crate::device_panel::scene3d::{Camera, IOS_PHONE, Quat, Vec3};
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

    fn filled(width: usize, height: usize, (y, cb, cr): (u8, u8, u8)) -> Surface {
        let surface = pixel_buffer(width, height).unwrap();
        planes(
            &surface,
            true,
            |luma, luma_stride, chroma, chroma_stride| {
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
            },
        );
        surface
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

    fn job<'a>(source: Source<'a>, model: &'a Model, pose: Quat, size: (usize, usize)) -> Job<'a> {
        let rotation = model.rotation(pose);
        let camera = Camera::fit(model.bounds(None), rotation, size.0 as f64 / size.1 as f64);
        let (mvp, rotation) = camera.matrices(rotation);
        Job {
            source,
            model,
            mvp,
            rotation,
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
            .render(job(Source::Surface(&source), &model, Quat::IDENTITY, size))
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
            .render(job(Source::Surface(&source), &model, Quat::IDENTITY, size))
            .unwrap();
        assert_eq!(
            again.buffer() as *const _,
            first.buffer() as *const _,
            "an unchanged key reuses the buffer"
        );
        let turned = Quat::axis_angle(Vec3::new(0.0, 1.0, 0.0), 0.5);
        let next = renderer
            .render(job(Source::Surface(&source), &model, turned, size))
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
                    id: 7,
                    bytes: &bgra,
                    width: 40,
                    height: 80,
                },
                &model,
                Quat::IDENTITY,
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
                    .render(job(Source::Surface(&source), &model, pose, size))
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
}
