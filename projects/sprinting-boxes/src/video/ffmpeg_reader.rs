use super::VideoReader;
use anyhow::{anyhow, Context, Result};
use opencv::{core, prelude::*};
use std::path::Path;

// Re-export the raw FFI types we need
use ffmpeg_next::ffi;

// ---------------------------------------------------------------------------
// HwDeviceCtx — RAII wrapper for AVBufferRef* (hardware device context)
// ---------------------------------------------------------------------------

/// Owns an `AVBufferRef*` for a hardware device context.
/// Calls `av_buffer_unref` on drop.
struct HwDeviceCtx {
    ptr: *mut ffi::AVBufferRef,
}

impl HwDeviceCtx {
    /// Attempt to create a VideoToolbox hardware device context.
    /// Returns `None` if creation fails (e.g. unsupported platform).
    fn new_videotoolbox() -> Option<Self> {
        let mut ptr: *mut ffi::AVBufferRef = std::ptr::null_mut();
        let ret = unsafe {
            ffi::av_hwdevice_ctx_create(
                &mut ptr,
                ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
            )
        };
        if ret < 0 || ptr.is_null() {
            None
        } else {
            Some(Self { ptr })
        }
    }

    /// Return a new `av_buffer_ref` of this context (increments refcount).
    /// The caller becomes the owner of the returned ref.
    fn buf_ref(&self) -> *mut ffi::AVBufferRef {
        unsafe { ffi::av_buffer_ref(self.ptr) }
    }
}

impl Drop for HwDeviceCtx {
    fn drop(&mut self) {
        unsafe {
            ffi::av_buffer_unref(&mut self.ptr);
        }
    }
}

// ---------------------------------------------------------------------------
// FfmpegReader
// ---------------------------------------------------------------------------

/// Video reader backed by FFmpeg via ffmpeg-next.
/// Attempts GPU-accelerated decoding via VideoToolbox on macOS;
/// falls back to CPU decoding transparently.
pub struct FfmpegReader {
    input_ctx: ffmpeg_next::format::context::Input,
    decoder: ffmpeg_next::codec::decoder::Video,
    video_stream_index: usize,
    /// Lazily created on first frame (source format is only known then).
    scaler: Option<ffmpeg_next::software::scaling::Context>,
    width: u32,
    height: u32,
    source_fps: f64,
    sample_rate: f64,
    total_frames: usize,
    frames_decoded: usize,
    // Hardware acceleration state
    _hw_device_ctx: Option<HwDeviceCtx>,
    /// The pixel format that indicates "this frame is in GPU memory".
    hw_pix_fmt: Option<ffi::AVPixelFormat>,
    _using_hw: bool,
    /// Persistent frame object to avoid allocations in the skip loop.
    reuse_frame: ffmpeg_next::util::frame::Video,
    /// Persistent packet object to avoid allocations.
    reuse_packet: ffmpeg_next::codec::packet::Packet,
    /// Whether we've sent EOF to the decoder.
    eof_sent: bool,
}

// SAFETY: FfmpegReader is only ever used from the single reader thread in the pipeline.
// The raw pointers inside ffmpeg-next types are not shared across threads.
unsafe impl Send for FfmpegReader {}

impl FfmpegReader {
    pub fn new(path: &str, sample_rate: f64) -> Result<Self> {
        ffmpeg_next::init().context("Failed to initialize FFmpeg")?;

        let source = Path::new(path);
        if !source.exists() {
            return Err(anyhow!("Video file not found: {}", path));
        }

        let input_ctx = ffmpeg_next::format::input(&source).context("Failed to open video file")?;

        let video_stream = input_ctx
            .streams()
            .best(ffmpeg_next::media::Type::Video)
            .ok_or_else(|| anyhow!("No video stream found in {}", path))?;

        let video_stream_index = video_stream.index();

        // --- Determine FPS & frame count before we move decoder_ctx ---
        let rational_fps = video_stream.avg_frame_rate();
        let source_fps = if rational_fps.denominator() > 0 {
            rational_fps.numerator() as f64 / rational_fps.denominator() as f64
        } else {
            tracing::warn!("FfmpegReader: could not determine FPS, defaulting to 30.0");
            30.0
        };

        let total_frames = video_stream.frames() as usize;
        let duration_secs = input_ctx.duration() as f64 / ffi::AV_TIME_BASE as f64;

        let calculated_total_frames = if total_frames == 0 {
            (duration_secs * source_fps).round() as usize
        } else {
            total_frames
        };

        tracing::info!(
            "FfmpegReader: opened {}, duration={:.2}s, fps={:.2}, stream_frames={}, estimated_total={}",
            path,
            duration_secs,
            source_fps,
            total_frames,
            calculated_total_frames
        );

        // --- Set up decoder context ---
        let mut decoder_ctx =
            ffmpeg_next::codec::context::Context::from_parameters(video_stream.parameters())
                .context("Failed to create decoder context")?;

        // --- Try hardware acceleration ---
        let (hw_device_ctx, hw_pix_fmt, _using_hw) = Self::try_setup_hw_accel(&mut decoder_ctx);

        let decoder = decoder_ctx
            .decoder()
            .video()
            .context("Failed to open video decoder")?;

        let width = decoder.width();
        let height = decoder.height();

        if _using_hw {
            tracing::info!(
                "FfmpegReader: using VideoToolbox hardware decoding ({}x{})",
                width,
                height
            );
        } else {
            tracing::info!(
                "FfmpegReader: using CPU software decoding ({}x{})",
                width,
                height
            );
        }

        Ok(Self {
            input_ctx,
            decoder,
            video_stream_index,
            scaler: None, // created lazily on first frame
            width,
            height,
            source_fps,
            sample_rate,
            total_frames: calculated_total_frames,
            frames_decoded: 0,
            _hw_device_ctx: hw_device_ctx,
            hw_pix_fmt,
            _using_hw,
            reuse_frame: ffmpeg_next::util::frame::Video::empty(),
            reuse_packet: ffmpeg_next::codec::packet::Packet::empty(),
            eof_sent: false,
        })
    }

    /// Try to configure VideoToolbox hardware acceleration on the decoder context.
    /// Returns (device_ctx, hw_pix_fmt, success_bool).
    /// On failure, returns (None, None, false) — caller should proceed with CPU decoding.
    /// Attempts to probe the decoder for hardware acceleration support (VideoToolbox on macOS).
    /// If successful, it returns:
    /// - `Some(HwDeviceCtx)`: RAII wrapper for the hardware context.
    /// - `Some(AVPixelFormat)`: The output pixel format the hardware decoder will use (e.g. `AV_PIX_FMT_VIDEOTOOLBOX`).
    /// - `true`: If hardware acceleration is active.
    fn try_setup_hw_accel(
        decoder_ctx: &mut ffmpeg_next::codec::context::Context,
    ) -> (Option<HwDeviceCtx>, Option<ffi::AVPixelFormat>, bool) {
        // Only attempt on macOS
        if !cfg!(target_os = "macos") {
            tracing::debug!("FfmpegReader: not macOS, skipping hw accel");
            return (None, None, false);
        }

        unsafe {
            // from_parameters sets codec_id but NOT the codec pointer.
            // We must look up the codec ourselves.
            let codec_id = (*decoder_ctx.as_ptr()).codec_id;
            tracing::debug!("FfmpegReader: codec_id = {:?}", codec_id);

            let codec_ptr = ffi::avcodec_find_decoder(codec_id);
            if codec_ptr.is_null() {
                tracing::debug!(
                    "FfmpegReader: avcodec_find_decoder returned null for codec_id {:?}",
                    codec_id
                );
                return (None, None, false);
            }

            // Log codec name
            let codec_name = if !(*codec_ptr).name.is_null() {
                std::ffi::CStr::from_ptr((*codec_ptr).name)
                    .to_string_lossy()
                    .into_owned()
            } else {
                "<unknown>".to_string()
            };
            tracing::debug!(
                "FfmpegReader: found codec '{}', probing hw configs",
                codec_name
            );

            // --- VideoToolbox Support Probe ---
            // FFmpeg codecs can support multiple hardware acceleration methods.
            // We iterate through them to see if VideoToolbox (Darwin) is available.
            let mut matched_pix_fmt: Option<ffi::AVPixelFormat> = None;
            let mut idx = 0i32;
            loop {
                let config = ffi::avcodec_get_hw_config(codec_ptr, idx);
                if config.is_null() {
                    break;
                }
                let c = &*config;
                tracing::debug!(
                    "FfmpegReader: hw_config[{}]: device_type={:?}, methods={}, pix_fmt={:?}",
                    idx,
                    c.device_type,
                    c.methods,
                    c.pix_fmt
                );

                // We prefer the HW_DEVICE_CTX method which allows us to manage
                // the hardware device lifecycle via AVBufferRef.
                if c.device_type == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX
                    && (c.methods as u32 & ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as u32) != 0
                {
                    matched_pix_fmt = Some(c.pix_fmt);
                    break;
                }
                idx += 1;
            }

            let hw_pix_fmt = match matched_pix_fmt {
                Some(fmt) => {
                    tracing::debug!("FfmpegReader: VideoToolbox supported, hw_pix_fmt={:?}", fmt);
                    fmt
                }
                None => {
                    tracing::info!(
                        "FfmpegReader: codec '{}' does not support VideoToolbox, using CPU",
                        codec_name
                    );
                    return (None, None, false);
                }
            };

            // Create the hardware device context
            let hw_ctx = match HwDeviceCtx::new_videotoolbox() {
                Some(ctx) => ctx,
                None => {
                    tracing::warn!(
                        "FfmpegReader: failed to create VideoToolbox device, falling back to CPU"
                    );
                    return (None, None, false);
                }
            };
            tracing::debug!("FfmpegReader: VideoToolbox device context created successfully");

            // Attach hw_device_ctx to decoder context (before opening)
            (*decoder_ctx.as_mut_ptr()).hw_device_ctx = hw_ctx.buf_ref();

            (Some(hw_ctx), Some(hw_pix_fmt), true)
        }
    }

    /// Set a hint for the decoder on which frames can be skipped during decoding.
    /// Discard::NonReference is used during the skip loop to save GPU/CPU cycles.
    fn set_skip_frame_hint(&mut self, discard: ffmpeg_next::codec::discard::Discard) {
        unsafe {
            (*self.decoder.as_mut_ptr()).skip_frame = discard.into();
        }
    }

    /// Internal logic to retrieve the next decoded frame from the stream.
    /// This is the core decoding loop used by both owned and reuse paths.
    fn decode_loop(&mut self, target_frame: &mut ffmpeg_next::util::frame::Video) -> Result<()> {
        loop {
            // 1. Try to receive a decoded frame
            match self.decoder.receive_frame(target_frame) {
                Ok(()) => return Ok(()),
                Err(ffmpeg_next::Error::Other { errno: ffi::EAGAIN }) => {
                    if self.eof_sent {
                        return Err(anyhow!("End of stream"));
                    }
                    // Continue to feeding packets
                }
                Err(ffmpeg_next::Error::Eof) => {
                    return Err(anyhow!("End of stream"));
                }
                Err(e) => return Err(anyhow!("Decoder error: {}", e)),
            }

            // 2. Feed packets until we find a video packet OR reach EOF
            if !self.eof_sent {
                let mut found_packet = false;
                while self.reuse_packet.read(&mut self.input_ctx).is_ok() {
                    if self.reuse_packet.stream() == self.video_stream_index {
                        self.decoder
                            .send_packet(&self.reuse_packet)
                            .context("Failed to send packet to decoder")?;
                        found_packet = true;
                        break;
                    }
                }

                if !found_packet {
                    // EOF reached in input file — notify decoder to flush
                    self.decoder
                        .send_eof()
                        .context("Failed to send EOF to decoder")?;
                    self.eof_sent = true;
                    // Loop back to try receive_frame one last time(s)
                }
            } else {
                // If EOF already sent and receive_frame returned EAGAIN, we are truly done
                return Err(anyhow!("End of stream"));
            }
        }
    }

    /// Receive the next raw frame into the persistent `reuse_frame`.
    fn receive_into_reuse(&mut self) -> Result<()> {
        // We use a temporary swap to satisfy the borrow checker:
        // we can't call self.decode_loop(&mut self.reuse_frame).
        let mut frame = ffmpeg_next::util::frame::Video::empty();
        std::mem::swap(&mut frame, &mut self.reuse_frame);
        let res = self.decode_loop(&mut frame);
        std::mem::swap(&mut frame, &mut self.reuse_frame);
        res
    }

    /// Receive the next raw frame from the decoder as an owned object.
    fn receive_next_raw_owned(&mut self) -> Result<ffmpeg_next::util::frame::Video> {
        let mut frame = ffmpeg_next::util::frame::Video::empty();
        self.decode_loop(&mut frame)?;
        Ok(frame)
    }
    fn get_or_create_scaler(
        &mut self,
        src_format: ffmpeg_next::format::Pixel,
    ) -> Result<&mut ffmpeg_next::software::scaling::Context> {
        if self.scaler.is_none() {
            let scaler = ffmpeg_next::software::scaling::Context::get(
                src_format,
                self.width,
                self.height,
                ffmpeg_next::format::Pixel::BGR24,
                self.width,
                self.height,
                ffmpeg_next::software::scaling::Flags::BILINEAR,
            )
            .context("Failed to create scaler")?;
            self.scaler = Some(scaler);
        }
        Ok(self.scaler.as_mut().unwrap())
    }

    /// Process a decoded frame: transfer from GPU if needed, and scale/convert to BGR24.
    /// Processes a decoded frame by:
    /// 1. Transferring it from GPU to CPU memory if hardware acceleration is active.
    /// 2. Converting it to the target BGR format if needed.
    /// If hardware transfer fails, it logs a warning and continues with the GPU frame (which will likely fail later).
    fn process_decoded_frame(
        &mut self,
        frame: ffmpeg_next::util::frame::Video,
    ) -> Result<ffmpeg_next::util::frame::Video> {
        let sw_frame = if self.is_hw_frame(&frame) {
            self.transfer_hw_frame(&frame)?
        } else {
            frame
        };

        let scaler = self.get_or_create_scaler(sw_frame.format())?;
        let mut processed_frame = ffmpeg_next::util::frame::Video::empty();
        scaler
            .run(&sw_frame, &mut processed_frame)
            .context("Scaler failed")?;

        Ok(processed_frame)
    }

    /// Check if a decoded frame is a hardware frame (lives in GPU memory).
    fn is_hw_frame(&self, frame: &ffmpeg_next::util::frame::Video) -> bool {
        match self.hw_pix_fmt {
            Some(hw_fmt) => {
                let frame_fmt = unsafe { (*frame.as_ptr()).format };
                frame_fmt == hw_fmt as i32
            }
            None => false,
        }
    }

    /// Transfer a hardware frame from GPU memory to CPU memory.
    fn transfer_hw_frame(
        &self,
        hw_frame: &ffmpeg_next::util::frame::Video,
    ) -> Result<ffmpeg_next::util::frame::Video> {
        let mut sw_frame = ffmpeg_next::util::frame::Video::empty();
        let ret =
            unsafe { ffi::av_hwframe_transfer_data(sw_frame.as_mut_ptr(), hw_frame.as_ptr(), 0) };
        if ret < 0 {
            return Err(anyhow!(
                "av_hwframe_transfer_data failed (error code {})",
                ret
            ));
        }
        Ok(sw_frame)
    }
}

/// Convert a BGR24 ffmpeg frame to an OpenCV Mat.
/// This performs a deep copy to ensure the Mat owns its data, making it safe
/// to send across channels after the source ffmpeg frame is dropped.
fn bgr_frame_to_mat(frame: &ffmpeg_next::util::frame::Video) -> Result<core::Mat> {
    let width = frame.width() as i32;
    let height = frame.height() as i32;
    let data = frame.data(0);
    let stride = frame.stride(0);

    // We MUST copy the data because 'frame' will be dropped after this call,
    // and the resulting Mat needs to be sent through channels to other workers.
    let mut mat = unsafe { core::Mat::new_rows_cols(height, width, core::CV_8UC3)? };

    for y in 0..height as usize {
        let src_offset = y * stride;
        let src_row = &data[src_offset..src_offset + (width as usize * 3)];
        let dst_ptr = mat.ptr_mut(y as i32)?;
        unsafe {
            std::ptr::copy_nonoverlapping(src_row.as_ptr(), dst_ptr, width as usize * 3);
        }
    }

    Ok(mat)
}

impl VideoReader for FfmpegReader {
    fn frame_count(&self) -> Result<usize> {
        let units =
            (self.total_frames as f64 * self.sample_rate / self.source_fps).floor() as usize;
        Ok(units.max(1))
    }

    fn source_fps(&self) -> Result<f64> {
        Ok(self.source_fps)
    }

    fn seek_to_frame(&mut self, frame_num: usize) -> Result<()> {
        let time_secs = frame_num as f64 / self.source_fps;
        let timestamp = (time_secs * ffi::AV_TIME_BASE as f64) as i64;
        self.input_ctx
            .seek(timestamp, ..timestamp)
            .context("Failed to seek")?;
        self.decoder.flush();
        self.eof_sent = false;
        self.scaler = None; // reset scaler on seek (format might change)
        self.frames_decoded = frame_num;
        Ok(())
    }

    fn read_unit(&mut self, unit_id: usize) -> Result<core::Mat> {
        let target_frame = super::unit_to_frame(unit_id, self.source_fps, self.sample_rate);

        if target_frame < self.frames_decoded {
            // Backward seek required
            self.seek_to_frame(target_frame)?;
        } else if target_frame > self.frames_decoded {
            // Skip forward.
            // optimization: if target is far, seek.
            if target_frame - self.frames_decoded > 50 {
                self.seek_to_frame(target_frame)?;
            } else {
                // Ensure skip hint is set for speed during seeking
                self.set_skip_frame_hint(ffmpeg_next::codec::discard::Discard::NonReference);
                while self.frames_decoded < target_frame {
                    // Fast decode to advance
                    self.receive_into_reuse()?;
                    self.frames_decoded += 1;
                }
            }
        }

        self.read_frame()
    }

    fn read_frame(&mut self) -> Result<core::Mat> {
        // Encode it for real.
        self.set_skip_frame_hint(ffmpeg_next::codec::discard::Discard::Default);
        let raw_frame = self.receive_next_raw_owned()?;
        let processed_frame = self.process_decoded_frame(raw_frame)?;
        let bgr_mat = bgr_frame_to_mat(&processed_frame)?;
        self.frames_decoded += 1;

        Ok(bgr_mat)
    }
}
