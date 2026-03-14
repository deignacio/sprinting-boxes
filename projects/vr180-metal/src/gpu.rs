use anyhow::{anyhow, Result};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::{
    kCFTypeDictionaryKeyCallBacks, kCFTypeDictionaryValueCallBacks, CFDictionary, CFRetained,
};
use objc2_core_video::{
    kCVPixelBufferIOSurfacePropertiesKey, kCVPixelFormatType_32BGRA, kCVReturnSuccess,
    CVMetalTextureCache, CVMetalTextureGetTexture, CVPixelBuffer, CVPixelBufferCreate,
};
use objc2_foundation::{NSString, NSURL};
use objc2_metal::{
    MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLComputeCommandEncoder,
    MTLComputePipelineState, MTLDevice, MTLLibrary, MTLPixelFormat, MTLSize, MTLTexture,
    MTLTextureDescriptor, MTLTextureUsage,
};
use std::ffi::c_void;
use std::ptr;
use std::ptr::NonNull;

/// Row-major 3×3 f32 rotation matrix passed to the Metal shader.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RotationMatrix {
    pub cols: [[f32; 3]; 3],
}

impl RotationMatrix {
    /// Build from yaw (Z), pitch (Y), roll (X) in radians — ZYX convention.
    pub fn from_yaw_pitch_roll(yaw: f32, pitch: f32, roll: f32) -> Self {
        let (sy, cy) = (yaw.sin(), yaw.cos());
        let (sp, cp) = (pitch.sin(), pitch.cos());
        let (sr, cr) = (roll.sin(), roll.cos());

        // Row-major storage: row0, row1, row2
        // Metal `float3x3` is column-major, so we pass transposed — the shader
        // does `rotation * dir` which reads columns as rows from our layout.
        Self {
            cols: [
                [cy * cp, cy * sp * sr - sy * cr, cy * sp * cr + sy * sr],
                [sy * cp, sy * sp * sr + cy * cr, sy * sp * cr - cy * sr],
                [-sp, cp * sr, cp * cr],
            ],
        }
    }
}

pub struct GpuOutputBuffer {
    pub texture: Retained<ProtocolObject<dyn MTLTexture>>,
    pub pixel_buffer: CFRetained<CVPixelBuffer>,
}

pub struct GpuContext {
    pub _device: Retained<ProtocolObject<dyn MTLDevice>>,
    pub queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    pub pipeline: Retained<ProtocolObject<dyn MTLComputePipelineState>>,
    texture_cache: CFRetained<CVMetalTextureCache>,
    pub output_buffers: Vec<GpuOutputBuffer>,
    pub current_buffer_idx: usize,
    pub out_width: u32,
    pub out_height: u32,
}

// CVMetalTextureCache and CVPixelBuffer are send-safe for our single-threaded use.
unsafe impl Send for GpuContext {}

impl GpuContext {
    pub fn new(src_width: u32, _src_height: u32) -> Result<Self> {
        let out_size = src_width / 2;

        // --- Metal device & command queue ---
        let device = objc2_metal::MTLCreateSystemDefaultDevice()
            .ok_or_else(|| anyhow!("No Metal device found"))?;
        let queue = device
            .newCommandQueue()
            .ok_or_else(|| anyhow!("Failed to create MTLCommandQueue"))?;

        // --- Load metallib from the path baked in at compile time ---
        let metallib_path = env!("METALLIB_PATH");
        let library = {
            let url = NSURL::fileURLWithPath(&NSString::from_str(metallib_path));
            device
                .newLibraryWithURL_error(&url)
                .map_err(|e| anyhow!("Failed to load Metal library: {e:?}"))?
        };

        let fn_name = NSString::from_str("reproject");
        let function = library
            .newFunctionWithName(&fn_name)
            .ok_or_else(|| anyhow!("Metal function 'reproject' not found in library"))?;

        let pipeline = {
            device
                .newComputePipelineStateWithFunction_error(&function)
                .map_err(|e| anyhow!("Failed to create compute pipeline: {e:?}"))?
        };

        // --- CVMetalTextureCache (for wrapping decoded CVPixelBuffers as MTLTextures) ---
        let mut texture_cache: *mut CVMetalTextureCache = ptr::null_mut();
        let rv = unsafe {
            CVMetalTextureCache::create(
                None, // allocator — kCFAllocatorDefault
                None, // cache attributes
                &*device,
                None, // texture attributes
                NonNull::new(&mut texture_cache).unwrap(),
            )
        };
        if rv != kCVReturnSuccess {
            return Err(anyhow!("CVMetalTextureCacheCreate failed: {rv}"));
        }
        let texture_cache = unsafe { CFRetained::from_raw(NonNull::new(texture_cache).unwrap()) };

        // --- Preallocate a pool of output buffers for triple-buffering ---
        let mut output_buffers = Vec::new();
        for _ in 0..3 {
            let desc = unsafe {
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    MTLPixelFormat::BGRA8Unorm,
                    out_size as usize,
                    out_size as usize,
                    false,
                )
            };
            desc.setUsage(MTLTextureUsage::ShaderWrite | MTLTextureUsage::ShaderRead);
            let texture = device
                .newTextureWithDescriptor(&desc)
                .ok_or_else(|| anyhow!("Failed to allocate output MTLTexture"))?;

            let iosurface_props = unsafe {
                CFDictionary::new(
                    None,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    0,
                    &raw const kCFTypeDictionaryKeyCallBacks,
                    &raw const kCFTypeDictionaryValueCallBacks,
                )
                .expect("Empty CFDictionaryCreate failed")
            };
            let pixel_buffer_attrs: objc2_core_foundation::CFRetained<CFDictionary> = unsafe {
                use objc2_core_foundation::CFType;
                let key = kCVPixelBufferIOSurfacePropertiesKey as *const _ as *const c_void;
                let val_cf: &CFType = iosurface_props.as_ref();
                let val = val_cf as *const CFType as *const c_void;
                let mut keys = [key];
                let mut vals = [val];
                CFDictionary::new(
                    None,
                    keys.as_mut_ptr(),
                    vals.as_mut_ptr(),
                    1,
                    &raw const kCFTypeDictionaryKeyCallBacks,
                    &raw const kCFTypeDictionaryValueCallBacks,
                )
                .expect("CFDictionaryCreate failed")
            };

            let mut out_pixel_buffer: *mut CVPixelBuffer = ptr::null_mut();
            let rv = unsafe {
                CVPixelBufferCreate(
                    None,
                    out_size as usize,
                    out_size as usize,
                    kCVPixelFormatType_32BGRA,
                    Some(&pixel_buffer_attrs),
                    NonNull::new(&mut out_pixel_buffer).unwrap(),
                )
            };
            if rv != kCVReturnSuccess {
                return Err(anyhow!("CVPixelBufferCreate failed: {rv}"));
            }
            let pixel_buffer =
                unsafe { CFRetained::from_raw(NonNull::new(out_pixel_buffer).unwrap()) };

            output_buffers.push(GpuOutputBuffer {
                texture,
                pixel_buffer,
            });
        }

        Ok(Self {
            _device: device,
            queue,
            pipeline,
            texture_cache,
            output_buffers,
            current_buffer_idx: 0,
            out_width: out_size,
            out_height: out_size,
        })
    }

    /// Access the current output CVPixelBuffer and its corresponding texture.
    pub fn current_output(&self) -> &GpuOutputBuffer {
        &self.output_buffers[self.current_buffer_idx]
    }

    /// Wrap a decoded CVPixelBuffer (IOSurface-backed) as an MTLTexture (zero-copy).
    pub fn wrap_pixel_buffer_as_texture(
        &self,
        pixel_buffer: &CFRetained<CVPixelBuffer>,
        width: usize,
        height: usize,
    ) -> Result<Retained<ProtocolObject<dyn MTLTexture>>> {
        use objc2_core_video::{CVImageBuffer, CVMetalTexture};
        let mut cv_metal_texture: *mut CVMetalTexture = ptr::null_mut();
        let image_buffer: &CVImageBuffer = unsafe {
            (objc2_core_foundation::CFRetained::as_ptr(pixel_buffer).as_ptr()
                as *const CVImageBuffer)
                .as_ref()
                .ok_or_else(|| anyhow!("null pixel_buffer"))?
        };
        let texture_cache_ref: &CVMetalTextureCache = &self.texture_cache;
        let rv = unsafe {
            CVMetalTextureCache::create_texture_from_image(
                None,
                texture_cache_ref,
                image_buffer,
                None,
                MTLPixelFormat::BGRA8Unorm,
                width,
                height,
                0, // plane index
                NonNull::new(&mut cv_metal_texture).unwrap(),
            )
        };
        if rv != kCVReturnSuccess {
            return Err(anyhow!(
                "CVMetalTextureCacheCreateTextureFromImage failed: {rv}"
            ));
        }

        // Wrap in CFRetained so it's released automatically.
        let cv_metal_texture =
            unsafe { CFRetained::from_raw(NonNull::new(cv_metal_texture).unwrap()) };

        // CVMetalTextureGetTexture takes &CVMetalTexture (= &CVImageBuffer).
        // Cast through CVImageBuffer, not CVBuffer, to match the expected type.
        let cv_metal_texture_ref: &CVImageBuffer = unsafe {
            (objc2_core_foundation::CFRetained::as_ptr(&cv_metal_texture).as_ptr()
                as *const CVImageBuffer)
                .as_ref()
                .ok_or_else(|| anyhow!("null cv_metal_texture"))?
        };
        let texture = CVMetalTextureGetTexture(cv_metal_texture_ref)
            .ok_or_else(|| anyhow!("CVMetalTextureGetTexture returned nil"))?;
        Ok(texture)
    }

    /// Run the reprojection compute shader: equirectangular → fisheye.
    /// Returns the CVPixelBuffer that was just reprojected into.
    pub fn reproject(
        &mut self,
        src_texture: &ProtocolObject<dyn MTLTexture>,
        rotation: &RotationMatrix,
    ) -> Result<CFRetained<CVPixelBuffer>> {
        let cmd = self
            .queue
            .commandBuffer()
            .ok_or_else(|| anyhow!("Failed to create MTLCommandBuffer"))?;

        let enc = cmd
            .computeCommandEncoder()
            .ok_or_else(|| anyhow!("Failed to create compute command encoder"))?;

        let out_buf_data = self.current_output();
        let pixel_buffer = out_buf_data.pixel_buffer.clone();

        unsafe {
            enc.setComputePipelineState(&self.pipeline);
            enc.setTexture_atIndex(Some(src_texture), 0);
            enc.setTexture_atIndex(Some(&*out_buf_data.texture), 1);
        }

        // Upload rotation matrix as a tiny buffer.
        let rotation_bytes = unsafe {
            std::slice::from_raw_parts(
                rotation as *const RotationMatrix as *const u8,
                std::mem::size_of::<RotationMatrix>(),
            )
        };
        unsafe {
            enc.setBytes_length_atIndex(
                NonNull::new(rotation_bytes.as_ptr() as *mut c_void).unwrap(),
                rotation_bytes.len(),
                0,
            );
        }

        // Dispatch threadgroups to cover the full output texture.
        let w = self.pipeline.threadExecutionWidth();
        let h = self.pipeline.maxTotalThreadsPerThreadgroup() / w;
        let threads_per_group = MTLSize {
            width: w,
            height: h,
            depth: 1,
        };
        let groups = MTLSize {
            width: (self.out_width as usize + w - 1) / w,
            height: (self.out_height as usize + h - 1) / h,
            depth: 1,
        };
        enc.dispatchThreadgroups_threadsPerThreadgroup(groups, threads_per_group);
        enc.endEncoding();

        cmd.commit();
        cmd.waitUntilCompleted();
        self.rotate_buffer();

        // Flush texture cache to invalidate stale entries.
        let cache_ref = &*self.texture_cache;
        cache_ref.flush(0);

        Ok(pixel_buffer)
    }

    fn rotate_buffer(&mut self) {
        self.current_buffer_idx = (self.current_buffer_idx + 1) % self.output_buffers.len();
    }
}
