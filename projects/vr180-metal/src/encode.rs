use anyhow::{anyhow, Result};
use objc2_core_foundation::{
    kCFBooleanFalse, CFBoolean, CFNumber, CFNumberType, CFRetained, CFString,
};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::{kCVPixelBufferIOSurfacePropertiesKey, CVPixelBuffer};
use objc2_video_toolbox::{
    kVTCompressionPropertyKey_AverageBitRate, kVTCompressionPropertyKey_ProfileLevel,
    kVTCompressionPropertyKey_RealTime,
    kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder,
    kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder, VTCompressionSession,
};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

pub struct EncodedPacket {
    /// Raw HVCC-format HEVC data (length-prefixed NAL units).
    pub data: Vec<u8>,
    pub pts: i64,
    pub dts: i64,
    pub is_keyframe: bool,
    /// HEVCDecoderConfigurationRecord extradata — set only on the first keyframe.
    pub extradata: Option<Vec<u8>>,
}

struct EncodeState {
    packets: VecDeque<EncodedPacket>,
    /// True after we've captured extradata from the first keyframe.
    extradata_captured: bool,
}

/// VideoToolbox HEVC encoder wrapping VTCompressionSession.
pub struct Encoder {
    session: *mut VTCompressionSession,
    state: Arc<Mutex<EncodeState>>,
    fps_num: i32,
    fps_den: i32,
}

unsafe impl Send for Encoder {}

unsafe extern "C-unwind" fn compress_callback(
    output_callback_ref_con: *mut c_void,
    _source_frame_ref_con: *mut c_void,
    status: i32,
    info_flags: objc2_video_toolbox::VTEncodeInfoFlags,
    sample_buffer: *mut CMSampleBuffer,
) {
    if status != 0 {
        eprintln!("Compress callback error: {}", status);
        return;
    }
    if sample_buffer.is_null() {
        return;
    }
    // kVTEncodeInfo_FrameDropped = 1<<1 = 0x2  (kVTEncodeInfo_Asynchronous = 1<<0 = 0x1)
    if info_flags.0 & 0x2 != 0 {
        return;
    };

    let retained_buf = unsafe { CFRetained::retain(NonNull::new(sample_buffer).unwrap()) };
    let data = unsafe { extract_sample_buffer_data(&retained_buf) };
    match data {
        Ok((bytes, pts, dts, keyframe)) => {
            let state = unsafe { &*(output_callback_ref_con as *const Mutex<EncodeState>) };
            let mut guard = state.lock().unwrap();
            let extradata = if keyframe && !guard.extradata_captured {
                let ed = unsafe { extract_hevc_extradata(&retained_buf) }.ok();
                if ed.is_some() {
                    guard.extradata_captured = true;
                }
                ed
            } else {
                None
            };
            guard.packets.push_back(EncodedPacket {
                data: bytes,
                pts,
                dts,
                is_keyframe: keyframe,
                extradata,
            });
        }
        Err(e) => {
            eprintln!("Failed to extract sample buffer data: {:?}", e);
        }
    }
}

unsafe fn extract_sample_buffer_data(sbuf: &CMSampleBuffer) -> Result<(Vec<u8>, i64, i64, bool)> {
    use objc2_core_media::kCMSampleAttachmentKey_NotSync;

    let pts = unsafe { sbuf.presentation_time_stamp() };
    let dts = unsafe { sbuf.decode_time_stamp() };
    let pts_val = pts.value;
    let dts_val = if dts.value == i64::MIN {
        pts_val
    } else {
        dts.value
    };

    let block_buffer = unsafe { sbuf.data_buffer() }.ok_or_else(|| anyhow!("null block buffer"))?;
    let len = unsafe { block_buffer.data_length() };
    let mut data_ptr: *mut std::ffi::c_char = ptr::null_mut();
    let mut _data_len: usize = 0;
    unsafe {
        block_buffer.data_pointer(0, ptr::null_mut(), &mut _data_len, &mut data_ptr);
    }
    if data_ptr.is_null() {
        return Err(anyhow!("CMBlockBufferGetDataPointer returned null"));
    }
    let bytes = unsafe { std::slice::from_raw_parts(data_ptr as *const u8, len) }.to_vec();

    // Check for keyframe (absence of kCMSampleAttachmentKey_NotSync).
    let attachments = unsafe { sbuf.sample_attachments_array(false) };
    let is_keyframe = if let Some(arr) = attachments {
        // If the array is empty or dict has no NotSync key, it's a keyframe.
        if arr.len() == 0 {
            true
        } else {
            // The attachments array contains CFMutableDictionary objects.
            // We get the first element and check for kCMSampleAttachmentKey_NotSync.
            let dict_ptr = unsafe { arr.value_at_index(0) };
            if dict_ptr.is_null() {
                true
            } else {
                let not_sync_key: &objc2_core_foundation::CFString =
                    &*kCMSampleAttachmentKey_NotSync;
                let contains = unsafe {
                    let dict_ref = &*(dict_ptr as *const objc2_core_foundation::CFDictionary);
                    dict_ref.contains_ptr_key(not_sync_key as *const _ as *const c_void)
                };
                !contains
            }
        }
    } else {
        true
    };

    Ok((bytes, pts_val, dts_val, is_keyframe))
}

/// Extract a HEVCDecoderConfigurationRecord from the first keyframe's CMFormatDescription.
/// This is the `extradata` / `hvcC` box content the MP4 muxer needs.
///
/// Uses kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms to get the raw
/// hvcC bytes that CoreMedia builds from the VPS/SPS/PPS in the format description.
unsafe fn extract_hevc_extradata(sbuf: &CMSampleBuffer) -> Result<Vec<u8>> {
    use objc2_core_foundation::{CFData, CFDictionary, CFString};
    use objc2_core_media::{
        kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms, CMFormatDescription,
    };

    let fmt_desc_retained = unsafe { sbuf.format_description() }
        .ok_or_else(|| anyhow!("no format description on encoded sample buffer"))?;
    let fmt_desc: &CMFormatDescription = &fmt_desc_retained;

    // kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms is a CFDictionary
    // mapping box type strings (e.g. "hvcC") to CFData containing the raw box payload.
    let ext_key = unsafe { &*kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms };
    let ext = fmt_desc
        .extension(ext_key)
        .ok_or_else(|| anyhow!("no SampleDescriptionExtensionAtoms on format description"))?;

    // Cast the CFTypeRef to CFDictionary.
    let atoms_dict = unsafe {
        let raw = objc2_core_foundation::CFRetained::as_ptr(&ext).as_ptr();
        &*(raw as *const CFDictionary)
    };

    // Look up "hvcC" key.
    let hvc_key = CFString::from_str("hvcC");
    let hvc_val = unsafe {
        use objc2_core_foundation::CFType;
        let key_cf: &CFType = hvc_key.as_ref();
        atoms_dict.value(key_cf as *const CFType as *const c_void)
    };
    if hvc_val.is_null() {
        return Err(anyhow!("hvcC key not found in SampleDescriptionExtensionAtoms"));
    }

    // The value is a CFData containing the raw HEVCDecoderConfigurationRecord.
    let hvc_data = unsafe { &*(hvc_val as *const CFData) };
    let len = hvc_data.length();
    let ptr = hvc_data.byte_ptr();
    if ptr.is_null() || len == 0 {
        return Err(anyhow!("hvcC CFData is empty"));
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) }.to_vec();
    Ok(bytes)
}

/// Create a CFNumber from an i32 value.
unsafe fn cf_number_i32(val: i32) -> objc2_core_foundation::CFRetained<CFNumber> {
    unsafe {
        CFNumber::new(
            None,
            CFNumberType::SInt32Type,
            &val as *const i32 as *const c_void,
        )
        .expect("CFNumberCreate should not fail")
    }
}

impl Encoder {
    pub fn new(
        width: u32,
        height: u32,
        fps_num: i32,
        fps_den: i32,
        bitrate_bps: i32,
    ) -> Result<Self> {
        let state = Arc::new(Mutex::new(EncodeState {
            packets: VecDeque::new(),
            extradata_captured: false,
        }));
        let state_ptr = Arc::as_ptr(&state) as *mut c_void;
        let encoder_spec: objc2_core_foundation::CFRetained<objc2_core_foundation::CFDictionary> = unsafe {
            use objc2_core_foundation::{
                kCFBooleanTrue, kCFTypeDictionaryKeyCallBacks, kCFTypeDictionaryValueCallBacks,
                CFDictionary, CFType,
            };
            let key = kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder
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

        let source_attrs: objc2_core_foundation::CFRetained<objc2_core_foundation::CFDictionary> = unsafe {
            use objc2_core_foundation::{
                kCFTypeDictionaryKeyCallBacks, kCFTypeDictionaryValueCallBacks, CFDictionary,
                CFType,
            };
            let key = kCVPixelBufferIOSurfacePropertiesKey as *const _ as *const c_void;
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
            let mut keys = [key];
            let mut vals = [val_io_ptr];
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

        let mut session: *mut VTCompressionSession = ptr::null_mut();
        let rv = unsafe {
            VTCompressionSession::create(
                None,
                width as i32,
                height as i32,
                // kCMVideoCodecType_HEVC = 'hvc1' = 0x68766331
                0x68766331u32,
                Some(&encoder_spec), // encoder specification: force hardware acceleration
                Some(&source_attrs), // source image buffer attributes: require IOSurface support
                None,                // compressed data allocator
                Some(compress_callback),
                state_ptr,
                NonNull::new(&mut session).unwrap(),
            )
        };
        if rv != 0 {
            return Err(anyhow!("VTCompressionSessionCreate failed: {rv}"));
        }

        let session_ref: &VTCompressionSession = unsafe { &*session };

        // Set properties using CFType-based API.
        unsafe {
            use objc2_core_foundation::CFType;
            use objc2_video_toolbox::VTSessionSetProperty;

            let bitrate = cf_number_i32(bitrate_bps);
            let bitrate_cf: &CFType = bitrate.as_ref();
            VTSessionSetProperty(
                session_ref.as_ref(),
                kVTCompressionPropertyKey_AverageBitRate,
                Some(bitrate_cf),
            );

            let realtime_val: &CFBoolean =
                objc2_core_foundation::kCFBooleanTrue.expect("kCFBooleanTrue");
            let realtime_cf: &CFType = realtime_val.as_ref();
            VTSessionSetProperty(
                session_ref.as_ref(),
                kVTCompressionPropertyKey_RealTime,
                Some(realtime_cf),
            );

            let profile = CFString::from_str("HEVC_Main_AutoLevel");
            let profile_cf: &CFType = profile.as_ref();
            VTSessionSetProperty(
                session_ref.as_ref(),
                kVTCompressionPropertyKey_ProfileLevel,
                Some(profile_cf),
            );

            // Disable B-frames for maximum throughput
            let reorder_key = CFString::from_str("AllowFrameReordering");
            VTSessionSetProperty(
                session_ref.as_ref(),
                &reorder_key,
                Some(kCFBooleanFalse.expect("kCFBooleanFalse").as_ref()),
            );

            // Force hardware acceleration
            let hw_val: &CFBoolean = objc2_core_foundation::kCFBooleanTrue.expect("kCFBooleanTrue");
            let hw_cf: &CFType = hw_val.as_ref();
            VTSessionSetProperty(
                session_ref.as_ref(),
                kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder,
                Some(hw_cf),
            );

            // High performance tuning
            let efficiency_val: &CFBoolean = kCFBooleanFalse.expect("kCFBooleanFalse");
            let efficiency_key = CFString::from_str("MaximizePowerEfficiency");
            VTSessionSetProperty(
                session_ref.as_ref(),
                &efficiency_key,
                Some(efficiency_val.as_ref()),
            );

            let priority_key = CFString::from_str("Priority");
            let priority_val = cf_number_i32(1);
            VTSessionSetProperty(
                session_ref.as_ref(),
                &priority_key,
                Some(priority_val.as_ref()),
            );

            session_ref.prepare_to_encode_frames();
        }

        Ok(Self {
            session,
            state,
            fps_num,
            fps_den,
        })
    }

    /// Encode one CVPixelBuffer frame with explicit PTS.
    pub fn encode_frame(
        &mut self,
        pixel_buffer: &CFRetained<CVPixelBuffer>,
        pts: CMTime,
    ) -> Result<()> {
        use objc2_core_video::CVImageBuffer;
        let duration = unsafe { CMTime::new(self.fps_den as i64, self.fps_num) };

        let session_ref: &VTCompressionSession = unsafe { &*self.session };
        let image_buffer: &CVImageBuffer = unsafe {
            (objc2_core_foundation::CFRetained::as_ptr(pixel_buffer).as_ptr()
                as *const CVImageBuffer)
                .as_ref()
                .ok_or_else(|| anyhow!("null pixel_buffer"))?
        };
        let rv = unsafe {
            session_ref.encode_frame(
                image_buffer,
                pts,
                duration,
                None,            // frame properties
                ptr::null_mut(), // source frame ref con
                ptr::null_mut(), // info flags out
            )
        };
        if rv != 0 {
            return Err(anyhow!("VTCompressionSessionEncodeFrame failed: {rv}"));
        }
        Ok(())
    }

    /// Flush all pending frames and drain output packets.
    pub fn flush(&self) -> Result<Vec<EncodedPacket>> {
        let session_ref: &VTCompressionSession = unsafe { self.session.as_ref().unwrap() };
        let rv = unsafe { session_ref.complete_frames(objc2_core_media::kCMTimeIndefinite) };
        if rv != 0 {
            eprintln!("VTCompressionSessionCompleteFrames returned {rv}");
        }
        Ok(self.drain_packets())
    }

    /// Drain encoded packets accumulated so far.
    pub fn drain_packets(&self) -> Vec<EncodedPacket> {
        self.state.lock().unwrap().packets.drain(..).collect()
    }
}
