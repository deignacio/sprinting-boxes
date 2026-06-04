use super::VideoReader;
use anyhow::{anyhow, Context, Result};
use opencv::{core, prelude::*};
use std::path::Path;
use std::sync::Once;

// Re-export the raw FFI types we need
use ffmpeg_next::ffi;
// Required to use as_ptr/as_mut_ptr on ffmpeg_next packet and frame types
use ffmpeg_next::packet::Ref as PacketRef;

// Initialize FFmpeg exactly once, globally
static FFMPEG_INIT: Once = Once::new();

/// Ensure FFmpeg is initialized (safe to call multiple times).
pub(crate) fn init_ffmpeg() -> Result<()> {
    let mut result = Ok(());
    FFMPEG_INIT.call_once(|| match ffmpeg_next::init() {
        Ok(_) => {
            // Suppress FFmpeg's verbose swscaler warnings about missing accelerated colorspace conversions
            unsafe {
                ffi::av_log_set_level(ffi::AV_LOG_ERROR);
            }
        }
        Err(e) => {
            result = Err(anyhow::anyhow!("Failed to initialize FFmpeg: {}", e));
        }
    });
    result
}

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

/// Threshold for average keyframe spacing: if keyframes are spaced further apart
/// than this, fall back to sampled reading mode.
const KEYFRAME_SPACING_THRESHOLD_SECS: f64 = 1.5;

/// How FfmpegReader decodes frames.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ReadingMode {
    /// Keyframe-only: AVDISCARD_NONKEY is set, keyframe_pts is populated.
    /// frame_count() == total_keyframes.
    KeyframeOnly,
    /// Sampled: all frames decoded, only frames at multiples of (1/sample_rate) seconds returned.
    /// frame_count() == floor(duration_secs * sample_rate).
    Sampled,
}

/// Video reader backed by FFmpeg via ffmpeg-next.
/// Attempts GPU-accelerated decoding via VideoToolbox on macOS;
/// falls back to CPU decoding transparently.
/// Reads either keyframe-only (dense keyframes) or sampled (sparse keyframes).
pub struct FfmpegReader {
    input_ctx: ffmpeg_next::format::context::Input,
    decoder: ffmpeg_next::codec::decoder::Video,
    video_stream_index: usize,
    /// Lazily created on first frame (source format is only known then).
    scaler: Option<ffmpeg_next::software::scaling::Context>,
    width: u32,
    height: u32,
    source_fps: f64,
    /// Duration of the video in seconds (used for keyframe count estimation).
    duration_secs: f64,
    /// Number of keyframes in the video (populated by index scan in new()).
    total_keyframes: usize,
    /// PTS (in stream time_base units) for each keyframe, indexed by keyframe number.
    keyframe_pts: Vec<i64>,
    /// Stream time_base as a float (num/den) for converting PTS to seconds.
    stream_time_base: f64,
    /// Keyframe index of the next frame to be decoded (0 = first keyframe).
    frames_decoded: usize,
    // Hardware acceleration state
    _hw_device_ctx: Option<HwDeviceCtx>,
    /// The pixel format that indicates "this frame is in GPU memory".
    hw_pix_fmt: Option<ffi::AVPixelFormat>,
    _using_hw: bool,
    /// Persistent packet object to avoid allocations in the decode loop.
    reuse_packet: ffmpeg_next::codec::packet::Packet,
    /// Whether we've sent EOF to the decoder.
    eof_sent: bool,
    /// Reading mode determined at construction time.
    reading_mode: ReadingMode,
    /// Desired output sample rate (frames per second). Only meaningful in Sampled mode.
    sample_rate: f64,
}

// SAFETY: Each FfmpegReader instance is owned and used exclusively by a single thread.
// Multiple instances may run concurrently on different threads, each with their own
// independent file handle, decoder context, and packet/frame buffers.
unsafe impl Send for FfmpegReader {}

impl FfmpegReader {
    pub fn new(path: &str, sample_rate: f64) -> Result<Self> {
        init_ffmpeg()?;

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

        // Capture stream time_base for PTS → seconds conversion
        let time_base = video_stream.time_base();
        let stream_time_base = if time_base.1 > 0 {
            time_base.0 as f64 / time_base.1 as f64
        } else {
            1.0 / 90000.0 // fallback: common 90kHz clock
        };

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

        // --- Set decoder to skip all non-keyframe frames (tentatively) ---
        // This will be overridden to AVDISCARD_NONE if sampled mode is needed.
        unsafe {
            (*decoder_ctx.as_mut_ptr()).skip_frame = ffi::AVDiscard::AVDISCARD_NONKEY;
        }

        let decoder = decoder_ctx
            .decoder()
            .video()
            .context("Failed to open video decoder")?;

        let width = decoder.width();
        let height = decoder.height();

        let mut reader = Self {
            input_ctx,
            decoder,
            video_stream_index,
            scaler: None,
            width,
            height,
            source_fps,
            duration_secs,
            total_keyframes: 0,
            keyframe_pts: Vec::new(),
            stream_time_base,
            frames_decoded: 0,
            _hw_device_ctx: hw_device_ctx,
            hw_pix_fmt,
            _using_hw,
            reuse_packet: ffmpeg_next::codec::packet::Packet::empty(),
            eof_sent: false,
            reading_mode: ReadingMode::KeyframeOnly, // temporary, will be set below
            sample_rate,
        };

        // Pre-scan all keyframe timestamps (reads packet headers only — no decoding).
        // This also determines whether to use keyframe-only or sampled mode.
        let reading_mode = reader.scan_keyframes()?;
        reader.reading_mode = reading_mode;

        // If sampled mode, override the decoder's skip_frame setting to AVDISCARD_NONE.
        if reading_mode == ReadingMode::Sampled {
            unsafe {
                (*reader.decoder.as_mut_ptr()).skip_frame = ffi::AVDiscard::AVDISCARD_NONE;
            }
        }

        let mode_label = match reading_mode {
            ReadingMode::KeyframeOnly => "keyframe-only",
            ReadingMode::Sampled => "sampled",
        };

        if _using_hw {
            tracing::info!(
                "FfmpegReader: using VideoToolbox hardware decoding ({}x{}), {} mode",
                width,
                height,
                mode_label
            );
        } else {
            tracing::info!(
                "FfmpegReader: using CPU software decoding ({}x{}), {} mode",
                width,
                height,
                mode_label
            );
        }

        Ok(reader)
    }

    /// Collect keyframe timestamps from the container's in-memory index.
    ///
    /// MP4 (and most seekable containers) populate `AVStream.index_entries` from the
    /// moov box during `avformat_open_input` — no bytes of video payload are read.
    /// Falls back to a duration-based estimate if the index is absent (live streams, etc.).
    /// Returns the reading mode to use based on keyframe density.
    fn scan_keyframes(&mut self) -> Result<ReadingMode> {
        let stream = self
            .input_ctx
            .stream(self.video_stream_index)
            .ok_or_else(|| anyhow!("video stream not found"))?;

        // Cast *const → *mut: avformat_index_get_entry needs *mut but only reads.
        // Safe because we hold &mut self (exclusive access) and the function is read-only.
        let stream_ptr = unsafe { stream.as_ptr() } as *mut ffi::AVStream;

        let n = unsafe { ffi::avformat_index_get_entries_count(stream_ptr) };

        let mode = if n > 0 {
            for i in 0..n {
                let entry = unsafe { ffi::avformat_index_get_entry(stream_ptr, i) };
                if entry.is_null() {
                    continue;
                }
                let entry_ref = unsafe { &*entry };
                if entry_ref.flags() & ffi::AVINDEX_KEYFRAME != 0 {
                    self.keyframe_pts.push(entry_ref.timestamp);
                }
            }
            self.total_keyframes = self.keyframe_pts.len();
            tracing::info!(
                "FfmpegReader: index scan complete — {} keyframes from {} index entries",
                self.total_keyframes,
                n,
            );

            // Decide mode based on average keyframe spacing
            if self.total_keyframes > 0 {
                let avg_spacing = self.duration_secs / self.total_keyframes as f64;
                if avg_spacing > KEYFRAME_SPACING_THRESHOLD_SECS {
                    tracing::info!(
                        "FfmpegReader: keyframes spaced {:.2}s apart (>{:.2}s threshold), using sampled mode",
                        avg_spacing,
                        KEYFRAME_SPACING_THRESHOLD_SECS
                    );
                    ReadingMode::Sampled
                } else {
                    tracing::info!(
                        "FfmpegReader: keyframes spaced {:.2}s apart, using keyframe-only mode",
                        avg_spacing
                    );
                    ReadingMode::KeyframeOnly
                }
            } else {
                ReadingMode::KeyframeOnly
            }
        } else {
            // No index (e.g. raw stream). Estimate 1 keyframe/second and use sampled mode.
            self.total_keyframes = self.duration_secs.ceil() as usize;
            tracing::warn!(
                "FfmpegReader: no container index found; using sampled mode with estimated {} keyframes from duration",
                self.total_keyframes
            );
            ReadingMode::Sampled
        };

        // Drop the Stream borrow before we need to use input_ctx again.
        let _ = stream;
        Ok(mode)
    }

    /// Try to configure VideoToolbox hardware acceleration on the decoder context.
    /// Returns (device_ctx, hw_pix_fmt, success_bool).
    /// On failure, returns (None, None, false) — caller should proceed with CPU decoding.
    fn try_setup_hw_accel(
        decoder_ctx: &mut ffmpeg_next::codec::context::Context,
    ) -> (Option<HwDeviceCtx>, Option<ffi::AVPixelFormat>, bool) {
        // Only attempt on macOS
        if !cfg!(target_os = "macos") {
            tracing::debug!("FfmpegReader: not macOS, skipping hw accel");
            return (None, None, false);
        }

        unsafe {
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

            (*decoder_ctx.as_mut_ptr()).hw_device_ctx = hw_ctx.buf_ref();

            (Some(hw_ctx), Some(hw_pix_fmt), true)
        }
    }

    /// Internal logic to retrieve the next decoded keyframe from the stream.
    /// Only keyframe packets are sent to the decoder; non-keyframe packets are skipped.
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

            // 2. Feed packets until we find one to send OR reach EOF
            if !self.eof_sent {
                let mut found_packet = false;
                while self.reuse_packet.read(&mut self.input_ctx).is_ok() {
                    if self.reuse_packet.stream() == self.video_stream_index {
                        // Determine whether to skip this packet based on reading mode
                        let is_key = unsafe {
                            ((*self.reuse_packet.as_ptr()).flags & ffi::AV_PKT_FLAG_KEY) != 0
                        };
                        let should_skip = match self.reading_mode {
                            ReadingMode::KeyframeOnly => !is_key,
                            ReadingMode::Sampled => false, // decode all frames in sampled mode
                        };

                        if should_skip {
                            // CRITICAL: Unref packet buffer before continuing to avoid av_malloc leak
                            unsafe {
                                ffi::av_packet_unref(self.reuse_packet.as_ptr() as *mut _);
                            }
                            continue;
                        }

                        self.decoder
                            .send_packet(&self.reuse_packet)
                            .context("Failed to send packet to decoder")?;
                        // Unref the packet after sending — avcodec_send_packet makes its own copy
                        unsafe {
                            ffi::av_packet_unref(self.reuse_packet.as_ptr() as *mut _);
                        }
                        found_packet = true;
                        break;
                    } else {
                        // Non-video stream packet (audio, subtitles, etc.) — must unref to avoid leak
                        unsafe {
                            ffi::av_packet_unref(self.reuse_packet.as_ptr() as *mut _);
                        }
                    }
                }

                if !found_packet {
                    self.decoder
                        .send_eof()
                        .context("Failed to send EOF to decoder")?;
                    self.eof_sent = true;
                }
            } else {
                return Err(anyhow!("End of stream"));
            }
        }
    }

    /// Receive the next keyframe from the decoder as an owned object.
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
    /// Returns the total number of keyframes in the video.
    /// Exact when the container index was available; estimated otherwise.
    fn frame_count(&self) -> Result<usize> {
        match self.reading_mode {
            ReadingMode::KeyframeOnly => Ok(self.total_keyframes.max(1)),
            ReadingMode::Sampled => {
                let count = (self.duration_secs * self.sample_rate).floor() as usize;
                Ok(count.max(1))
            }
        }
    }

    fn source_fps(&self) -> Result<f64> {
        Ok(self.source_fps)
    }

    /// Seek to the Nth keyframe (0-indexed).
    /// Uses the exact PTS from the container index when available; falls back to 1-sec/keyframe
    /// time estimation when the index was absent.
    fn seek_to_frame(&mut self, frame_num: usize) -> Result<()> {
        let time_secs = match self.reading_mode {
            ReadingMode::KeyframeOnly => {
                if let Some(&pts) = self.keyframe_pts.get(frame_num) {
                    pts as f64 * self.stream_time_base
                } else if !self.keyframe_pts.is_empty() {
                    // Beyond known keyframes — clamp to last known
                    let last = *self.keyframe_pts.last().unwrap();
                    last as f64 * self.stream_time_base
                } else {
                    // No index: assume 1 keyframe/second
                    frame_num as f64
                }
            }
            ReadingMode::Sampled => {
                // In sampled mode, frame_num is a sample index, not a keyframe index
                frame_num as f64 / self.sample_rate
            }
        };
        self.seek_to_time(time_secs)?;
        self.frames_decoded = frame_num;
        Ok(())
    }

    /// Read the Nth keyframe (unit_id = keyframe index, 0-based).
    ///
    /// Since I-frames have no temporal dependencies, any keyframe can be sought to directly
    /// via its exact PTS from the container index. Sequential access skips the seek.
    fn read_unit(&mut self, unit_id: usize) -> Result<core::Mat> {
        match self.reading_mode {
            ReadingMode::KeyframeOnly => {
                if unit_id != self.frames_decoded {
                    self.seek_to_frame(unit_id)?;
                }
                self.read_frame()
            }
            ReadingMode::Sampled => {
                let target_secs = unit_id as f64 / self.sample_rate;
                if unit_id != self.frames_decoded {
                    self.seek_to_time(target_secs)?;
                    self.frames_decoded = unit_id;
                }
                self.read_frame_at_time(target_secs)
            }
        }
    }

    fn read_frame(&mut self) -> Result<core::Mat> {
        let raw_frame = self.receive_next_raw_owned()?;
        let processed_frame = self.process_decoded_frame(raw_frame)?;
        let bgr_mat = bgr_frame_to_mat(&processed_frame)?;
        // In keyframe-only mode, frames_decoded is a keyframe counter. Increment after each read.
        // In sampled mode, frames_decoded is set to unit_id before reading, so don't increment here.
        if self.reading_mode == ReadingMode::KeyframeOnly {
            self.frames_decoded += 1;
        }
        Ok(bgr_mat)
    }
}

impl FfmpegReader {
    pub fn seek_to_time(&mut self, time_secs: f64) -> Result<()> {
        let timestamp = (time_secs * ffi::AV_TIME_BASE as f64) as i64;
        self.input_ctx
            .seek(timestamp, ..timestamp)
            .context("Failed to seek")?;
        self.decoder.flush();
        self.eof_sent = false;
        self.scaler = None;
        Ok(())
    }

    /// Decode and return the frame at the target time (in seconds).
    /// Used in sampled mode to find frames at exact sample times.
    fn read_frame_at_time(&mut self, target_secs: f64) -> Result<core::Mat> {
        let target_pts = target_secs / self.stream_time_base;
        let mut frames_tried = 0;
        const MAX_FRAMES_BEFORE_GIVE_UP: usize = 1000;

        loop {
            let raw_frame = self.receive_next_raw_owned()?;
            let frame_pts = unsafe { (*raw_frame.as_ptr()).pts };

            // Accept frame if PTS is at or past target, or if PTS is unavailable (AV_NOPTS_VALUE)
            let accept_frame = frame_pts == ffi::AV_NOPTS_VALUE || frame_pts >= target_pts as i64;
            if accept_frame {
                let processed_frame = self.process_decoded_frame(raw_frame)?;
                let bgr_mat = bgr_frame_to_mat(&processed_frame)?;
                // Don't increment frames_decoded here — it's already set to unit_id by read_unit()
                return Ok(bgr_mat);
            }

            frames_tried += 1;
            if frames_tried >= MAX_FRAMES_BEFORE_GIVE_UP {
                return Err(anyhow!(
                    "Failed to find frame at {}s after {} decoded frames",
                    target_secs,
                    frames_tried
                ));
            }
        }
    }
}
