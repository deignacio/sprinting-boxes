use anyhow::{anyhow, Result};
use objc2_core_foundation::{
    kCFTypeDictionaryKeyCallBacks, kCFTypeDictionaryValueCallBacks, CFDictionary, CFNumber,
    CFNumberType, CFRetained,
};
use objc2_core_media::{CMSampleBuffer, CMVideoFormatDescription};
use objc2_core_video::{
    kCVPixelBufferIOSurfacePropertiesKey, kCVPixelBufferPixelFormatTypeKey,
    kCVPixelFormatType_32BGRA, CVImageBuffer, CVPixelBuffer,
};
use objc2_video_toolbox::{
    kVTVideoDecoderSpecification_RequireHardwareAcceleratedVideoDecoder, VTDecodeFrameFlags,
    VTDecompressionOutputCallbackRecord, VTDecompressionSession,
};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

/// Decoded frame: IOSurface-backed CVPixelBuffer ready for Metal.
pub struct DecodedFrame {
    pub pixel_buffer: CFRetained<CVPixelBuffer>,
    pub pts: objc2_core_media::CMTime,
}

unsafe impl Send for DecodedFrame {}

struct CallbackState {
    frames: VecDeque<DecodedFrame>,
}

/// VideoToolbox HEVC decoder wrapping VTDecompressionSession.
pub struct Decoder {
    session: *mut VTDecompressionSession,
    state: Arc<Mutex<CallbackState>>,
    pub _width: u32,
    pub _height: u32,
}

unsafe impl Send for Decoder {}

unsafe extern "C-unwind" fn decompress_callback(
    decompress_output_ref_con: *mut c_void,
    _source_frame_ref_con: *mut c_void,
    status: i32,
    _info_flags: objc2_video_toolbox::VTDecodeInfoFlags,
    image_buffer: *mut CVImageBuffer,
    _presentation_time_stamp: objc2_core_media::CMTime,
    _presentation_duration: objc2_core_media::CMTime,
) {
    if status != 0 || image_buffer.is_null() {
        return;
    }
    let state = unsafe { &*(decompress_output_ref_con as *const Mutex<CallbackState>) };
    let mut guard = state.lock().unwrap();
    let ptr = NonNull::new(image_buffer as *mut CVPixelBuffer).unwrap();
    let retained_buffer = unsafe { CFRetained::retain(ptr) };
    guard.frames.push_back(DecodedFrame {
        pixel_buffer: retained_buffer,
        pts: _presentation_time_stamp,
    });
}

impl Decoder {
    pub fn new(
        format_desc: *mut CMVideoFormatDescription,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let state = Arc::new(Mutex::new(CallbackState {
            frames: VecDeque::new(),
        }));
        let state_ptr = Arc::as_ptr(&state) as *mut c_void;

        let callback = VTDecompressionOutputCallbackRecord {
            decompressionOutputCallback: Some(decompress_callback),
            decompressionOutputRefCon: state_ptr,
        };

        let format_desc_ref: &CMVideoFormatDescription = unsafe {
            format_desc
                .as_ref()
                .ok_or_else(|| anyhow!("null format_desc"))?
        };

        // Request BGRA output so decoded CVPixelBuffers match MTLPixelFormat::BGRA8Unorm
        // used by the Metal texture cache. Without this, VT outputs YCbCr which cannot
        // be wrapped as a BGRA texture and causes a segfault in CVMetalTextureGetTexture.
        let dest_attrs: objc2_core_foundation::CFRetained<CFDictionary> = unsafe {
            let pixel_format_num = CFNumber::new(
                None,
                CFNumberType::SInt32Type,
                &(kCVPixelFormatType_32BGRA as i32) as *const i32 as *const c_void,
            )
            .expect("CFNumber create failed");

            use objc2_core_foundation::CFType;
            let key_fmt = kCVPixelBufferPixelFormatTypeKey as *const _ as *const c_void;
            let val_fmt_cf: &CFType = pixel_format_num.as_ref();
            let val_fmt = val_fmt_cf as *const CFType as *const c_void;

            let key_io = kCVPixelBufferIOSurfacePropertiesKey as *const _ as *const c_void;
            let val_io = CFDictionary::new(
                None,
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                &raw const kCFTypeDictionaryKeyCallBacks,
                &raw const kCFTypeDictionaryValueCallBacks,
            )
            .expect("Empty CFDictionaryCreate failed");
            let val_io_cf: &CFType = val_io.as_ref();
            let val_io_ptr = val_io_cf as *const CFType as *const c_void;

            let mut keys = [key_fmt, key_io];
            let mut vals = [val_fmt, val_io_ptr];
            CFDictionary::new(
                None,
                keys.as_mut_ptr(),
                vals.as_mut_ptr(),
                2,
                &raw const kCFTypeDictionaryKeyCallBacks,
                &raw const kCFTypeDictionaryValueCallBacks,
            )
            .expect("CFDictionaryCreate failed")
        };

        let decoder_spec: objc2_core_foundation::CFRetained<CFDictionary> = unsafe {
            use objc2_core_foundation::{kCFBooleanTrue, CFType};
            let key = kVTVideoDecoderSpecification_RequireHardwareAcceleratedVideoDecoder
                as *const _ as *const c_void;
            let val_cf: &CFType = kCFBooleanTrue.expect("kCFBooleanTrue").as_ref();
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

        let mut session: *mut VTDecompressionSession = ptr::null_mut();
        let rv = unsafe {
            VTDecompressionSession::create(
                None, // allocator
                format_desc_ref,
                Some(&decoder_spec), // video decoder specification: enable hardware acceleration
                Some(&dest_attrs),   // destination image buffer attributes: request BGRA output
                &callback,
                NonNull::new(&mut session).unwrap(),
            )
        };
        if rv != 0 {
            return Err(anyhow!("VTDecompressionSessionCreate failed: {rv}"));
        }

        Ok(Self {
            session,
            state,
            _width: width,
            _height: height,
        })
    }

    /// Submit a CMSampleBuffer for decoding. Decoded frames accumulate in the callback queue.
    pub fn decode(&self, sample_buffer: &CMSampleBuffer) -> Result<()> {
        let session_ref: &VTDecompressionSession = unsafe {
            self.session
                .as_ref()
                .ok_or_else(|| anyhow!("null session"))?
        };
        let rv = unsafe {
            session_ref.decode_frame(
                sample_buffer,
                VTDecodeFrameFlags::Frame_EnableAsynchronousDecompression,
                ptr::null_mut(), // source frame ref con
                ptr::null_mut(), // info flags out
            )
        };
        if rv != 0 {
            return Err(anyhow!("VTDecompressionSessionDecodeFrame failed: {rv}"));
        }
        Ok(())
    }

    /// Drain all frames that have been decoded so far.
    pub fn drain_frames(&self) -> Vec<DecodedFrame> {
        let mut guard = self.state.lock().unwrap();
        guard.frames.drain(..).collect()
    }

    /// Block until all pending async VT decode callbacks have fired.
    /// Call this after submitting a batch of packets, then drain_frames().
    pub fn wait_for_async(&self) -> Result<()> {
        let session_ref: &VTDecompressionSession = unsafe {
            self.session
                .as_ref()
                .ok_or_else(|| anyhow!("null session"))?
        };
        unsafe { session_ref.wait_for_asynchronous_frames() };
        Ok(())
    }
}
