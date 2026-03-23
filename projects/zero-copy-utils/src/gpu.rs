use anyhow::{anyhow, Result};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::{
    kCFTypeDictionaryKeyCallBacks, kCFTypeDictionaryValueCallBacks, CFDictionary, CFRetained,
};
use objc2_core_video::{
    kCVPixelBufferIOSurfacePropertiesKey, kCVPixelFormatType_32BGRA, kCVReturnSuccess,
    CVMetalTextureCache, CVPixelBuffer, CVPixelBufferCreate,
};
use objc2_foundation::{NSString, NSURL};
use objc2_metal::{
    MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLComputeCommandEncoder,
    MTLComputePipelineState, MTLDevice, MTLLibrary, MTLPixelFormat, MTLSize, MTLTexture,
};
use std::ffi::c_void;
use std::ptr;
use std::ptr::NonNull;

/// Operation to perform on each frame.
pub enum GpuOp {
    /// Pixel-for-pixel copy; src and dst are the same size.
    Passthrough,
    /// Crop a rectangle from the source, with horizontal wrap-around support.
    Crop {
        x_offset: u32,
        y_offset: u32,
        /// Full source frame width — used for modulo wrap at the 360°/0° seam.
        src_width: u32,
    },
}

/// Matches the Metal `CropParams` struct in process.metal (std140 layout).
#[repr(C)]
struct CropParams {
    x_offset: u32,
    y_offset: u32,
    src_width: u32,
}

pub struct GpuOutputBuffer {
    pub texture: Retained<ProtocolObject<dyn MTLTexture>>,
    pub pixel_buffer: CFRetained<CVPixelBuffer>,
}

pub struct GpuContext {
    pub device: Retained<ProtocolObject<dyn MTLDevice>>,
    pub queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    passthrough_pipeline: Retained<ProtocolObject<dyn MTLComputePipelineState>>,
    crop_pipeline: Retained<ProtocolObject<dyn MTLComputePipelineState>>,
    _texture_cache: CFRetained<CVMetalTextureCache>,
    pub output_buffers: Vec<GpuOutputBuffer>,
    pub current_buffer_idx: usize,
    pub out_width: u32,
    pub out_height: u32,
}

// CVMetalTextureCache and CVPixelBuffer are safe to send for our single-threaded use.
unsafe impl Send for GpuContext {}

impl GpuContext {
    pub fn new(out_width: u32, out_height: u32) -> Result<Self> {
        let device = objc2_metal::MTLCreateSystemDefaultDevice()
            .ok_or_else(|| anyhow!("No Metal device found"))?;
        let queue = device
            .newCommandQueue()
            .ok_or_else(|| anyhow!("Failed to create MTLCommandQueue"))?;

        // Load compiled metallib baked in at build time.
        let metallib_path = env!("METALLIB_PATH");
        let library = {
            let url = NSURL::fileURLWithPath(&NSString::from_str(metallib_path));
            device
                .newLibraryWithURL_error(&url)
                .map_err(|e| anyhow!("Failed to load Metal library: {e:?}"))?
        };

        let passthrough_pipeline = make_pipeline(&device, &library, "k_passthrough")?;
        let crop_pipeline = make_pipeline(&device, &library, "k_crop")?;

        // CVMetalTextureCache — used to flush stale GPU texture entries after each frame.
        let mut texture_cache_raw: *mut CVMetalTextureCache = ptr::null_mut();
        let rv = unsafe {
            CVMetalTextureCache::create(
                None,
                None,
                &*device,
                None,
                NonNull::new(&mut texture_cache_raw).unwrap(),
            )
        };
        if rv != kCVReturnSuccess {
            return Err(anyhow!("CVMetalTextureCacheCreate failed: {rv}"));
        }
        let texture_cache =
            unsafe { CFRetained::from_raw(NonNull::new(texture_cache_raw).unwrap()) };

        // Pre-allocate triple-buffered output pool.
        // Each slot: IOSurface-backed CVPixelBuffer + MTLTexture wrapping the same IOSurface.
        // The Metal shader writes into the texture; the encoder reads from the pixel buffer —
        // both access the same GPU memory with no CPU copy.
        let mut output_buffers = Vec::new();
        for _ in 0..3 {
            let (pixel_buffer, texture) =
                alloc_output_slot(&device, out_width, out_height)?;
            output_buffers.push(GpuOutputBuffer { texture, pixel_buffer });
        }

        Ok(Self {
            device,
            queue,
            passthrough_pipeline,
            crop_pipeline,
            _texture_cache: texture_cache,
            output_buffers,
            current_buffer_idx: 0,
            out_width,
            out_height,
        })
    }

    /// Wrap a decoded CVPixelBuffer (IOSurface-backed, BGRA) as a read-only MTLTexture.
    pub fn wrap_input(
        &self,
        pixel_buffer: &CFRetained<CVPixelBuffer>,
        width: usize,
        height: usize,
    ) -> Result<Retained<ProtocolObject<dyn MTLTexture>>> {
        use objc2_core_video::CVPixelBufferGetIOSurface;
        use objc2_metal::MTLTextureDescriptor;

        let iosurface = CVPixelBufferGetIOSurface(Some(pixel_buffer))
            .ok_or_else(|| anyhow!("CVPixelBufferGetIOSurface returned nil for decoded frame — not IOSurface-backed"))?;
        let desc = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm,
                width,
                height,
                false,
            )
        };
        desc.setUsage(objc2_metal::MTLTextureUsage::ShaderRead);
        self.device
            .newTextureWithDescriptor_iosurface_plane(&desc, &iosurface, 0)
            .ok_or_else(|| anyhow!("newTextureWithDescriptor:iosurface:plane: (src) returned nil"))
    }

    /// Run a processing kernel on `src_texture`, writing into the next output slot.
    /// Returns the filled CVPixelBuffer ready for the encoder.
    pub fn process(
        &mut self,
        src_texture: &ProtocolObject<dyn MTLTexture>,
        op: &GpuOp,
    ) -> Result<CFRetained<CVPixelBuffer>> {
        let cmd = self
            .queue
            .commandBuffer()
            .ok_or_else(|| anyhow!("Failed to create MTLCommandBuffer"))?;
        let enc = cmd
            .computeCommandEncoder()
            .ok_or_else(|| anyhow!("Failed to create compute command encoder"))?;

        let out_buf = &self.output_buffers[self.current_buffer_idx];
        let pixel_buffer = out_buf.pixel_buffer.clone();

        let pipeline = match op {
            GpuOp::Passthrough => &self.passthrough_pipeline,
            GpuOp::Crop { .. } => &self.crop_pipeline,
        };

        unsafe {
            enc.setComputePipelineState(pipeline);
            enc.setTexture_atIndex(Some(src_texture), 0);
            enc.setTexture_atIndex(Some(&*out_buf.texture), 1);
        }

        if let GpuOp::Crop { x_offset, y_offset, src_width } = op {
            let params = CropParams {
                x_offset: *x_offset,
                y_offset: *y_offset,
                src_width: *src_width,
            };
            unsafe {
                enc.setBytes_length_atIndex(
                    NonNull::new(&params as *const CropParams as *mut c_void).unwrap(),
                    std::mem::size_of::<CropParams>(),
                    0,
                );
            }
        }

        let w = pipeline.threadExecutionWidth();
        let h = pipeline.maxTotalThreadsPerThreadgroup() / w;
        let threads_per_group = MTLSize { width: w, height: h, depth: 1 };
        let groups = MTLSize {
            width: (self.out_width as usize + w - 1) / w,
            height: (self.out_height as usize + h - 1) / h,
            depth: 1,
        };
        enc.dispatchThreadgroups_threadsPerThreadgroup(groups, threads_per_group);
        enc.endEncoding();

        cmd.commit();
        cmd.waitUntilCompleted();
        if let Some(err) = cmd.error() {
            return Err(anyhow!("Metal command buffer error: {err:?}"));
        }

        // Invalidate stale texture cache entries.
        self._texture_cache.flush(0);

        self.current_buffer_idx = (self.current_buffer_idx + 1) % self.output_buffers.len();
        Ok(pixel_buffer)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_pipeline(
    device: &ProtocolObject<dyn MTLDevice>,
    library: &ProtocolObject<dyn MTLLibrary>,
    fn_name: &str,
) -> Result<Retained<ProtocolObject<dyn MTLComputePipelineState>>> {
    let name = NSString::from_str(fn_name);
    let function = library
        .newFunctionWithName(&name)
        .ok_or_else(|| anyhow!("Metal function '{fn_name}' not found in library"))?;
    device
        .newComputePipelineStateWithFunction_error(&function)
        .map_err(|e| anyhow!("Failed to create compute pipeline '{fn_name}': {e:?}"))
}

fn alloc_output_slot(
    device: &ProtocolObject<dyn MTLDevice>,
    width: u32,
    height: u32,
) -> Result<(CFRetained<CVPixelBuffer>, Retained<ProtocolObject<dyn MTLTexture>>)> {
    use objc2_core_video::CVPixelBufferGetIOSurface;
    use objc2_metal::{MTLTextureDescriptor, MTLTextureUsage};

    // IOSurface-backed CVPixelBuffer.
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
    let pixel_buffer_attrs: CFRetained<CFDictionary> = unsafe {
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
            width as usize,
            height as usize,
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

    // MTLTexture wrapping the same IOSurface with ShaderWrite access.
    let iosurface = CVPixelBufferGetIOSurface(Some(&pixel_buffer)).ok_or_else(|| {
        anyhow!("CVPixelBufferGetIOSurface returned nil (output buffer not IOSurface-backed)")
    })?;
    let desc = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            MTLPixelFormat::BGRA8Unorm,
            width as usize,
            height as usize,
            false,
        )
    };
    desc.setUsage(MTLTextureUsage::ShaderWrite | MTLTextureUsage::ShaderRead);
    let texture = device
        .newTextureWithDescriptor_iosurface_plane(&desc, &iosurface, 0)
        .ok_or_else(|| {
            anyhow!("newTextureWithDescriptor:iosurface:plane: (output) returned nil")
        })?;

    Ok((pixel_buffer, texture))
}
