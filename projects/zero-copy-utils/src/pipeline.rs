use anyhow::{anyhow, Context, Result};
use ffmpeg::format;
use ffmpeg::media::Type;
use ffmpeg_next as ffmpeg;
use indicatif::{ProgressBar, ProgressStyle};
use objc2_core_foundation::CFRetained;
use std::ffi::c_void;
use std::path::Path;
use std::ptr::NonNull;

use crate::decode::Decoder;
use crate::encode::Encoder;
use crate::gpu::{GpuContext, GpuOp};
use crate::mux::Muxer;

/// High-level operation requested by the caller.
pub enum Operation {
    /// Decode every frame and re-encode it — sanity-checks the full pipeline.
    Passthrough,
    /// Decode, GPU-crop, re-encode.  `x_offset` / `y_offset` are pixel offsets
    /// in the source frame; the destination is `crop_w × crop_h`.
    Crop {
        x_offset: u32,
        y_offset: u32,
        crop_w: u32,
        crop_h: u32,
    },
    /// Crop a spherical region from an equirectangular source, specified in
    /// degrees.  Pixel offsets are derived from the source dimensions after
    /// probing.  Horizontal wrap-around is handled by the Metal shader, so
    /// any longitude center is valid (including values that cross the ±180°
    /// seam).
    SphericalCrop {
        /// Longitude of the crop centre in degrees (−180 … +180, 0 = front).
        lon_center: f64,
        /// Latitude of the crop centre in degrees (−90 … +90, 0 = equator).
        lat_center: f64,
        /// Horizontal field of view in degrees (e.g. 180 for a VR180 front hemisphere).
        fov_h: f64,
        /// Vertical field of view in degrees (e.g. 180 for a VR180 front hemisphere).
        fov_v: f64,
    },
}

// Target bitrate for HEVC output: 80 Mbps suits 4K content.
const TARGET_BITRATE_BPS: i32 = 80_000_000;

pub fn run(input_path: &Path, output_path: &Path, op: Operation) -> Result<()> {
    ffmpeg::init().context("ffmpeg init")?;

    // ── Probe input ────────────────────────────────────────────────────────────
    let input_ctx = format::input(input_path).context("open input")?;
    let video_stream = input_ctx
        .streams()
        .best(Type::Video)
        .ok_or_else(|| anyhow!("no video stream in input"))?;
    let video_idx = video_stream.index();

    let (src_width, src_height, codec_id) = unsafe {
        let raw = video_stream.parameters().as_ptr();
        ((*raw).width as u32, (*raw).height as u32, (*raw).codec_id)
    };
    let fps = video_stream.avg_frame_rate();
    let fps_num = fps.numerator();
    let fps_den = fps.denominator();
    let stream_timescale = video_stream.time_base().denominator() as i32;
    let frame_duration = {
        let num = stream_timescale as i64 * fps_den as i64;
        let den = fps_num as i64;
        (num + den / 2) / den
    };

    // ── Validate input codec ───────────────────────────────────────────────────
    // The VT decoder path uses CMVideoFormatDescriptionCreateFromHEVCParameterSets
    // which requires HEVC input.  Output is always H.264.
    use ffmpeg_next::ffi::AVCodecID;
    if codec_id != AVCodecID::AV_CODEC_ID_HEVC {
        return Err(anyhow!(
            "Input codec {:?} is not supported; only HEVC (H.265) input is currently handled",
            codec_id
        ));
    }

    // ── Normalise SphericalCrop → Crop using probed source dimensions ──────────
    let op = match op {
        Operation::SphericalCrop { lon_center, lat_center, fov_h, fov_v } => {
            // Equirectangular mapping:
            //   longitude -180°…+180° → x 0…src_width
            //   latitude  +90°…-90°  → y 0…src_height
            let crop_w = (fov_h / 360.0 * src_width as f64).round() as u32;
            let crop_h = (fov_v / 180.0 * src_height as f64).round() as u32;
            // Same convention as crop.sh / preview_crops.sh:
            //   lon_center=0   → left edge of frame (Insta360 X5 front hemisphere)
            //   lon_center=180 → center of frame (back hemisphere)
            // x_offset = pixel directly below lon_center, minus half the crop width.
            let center_px = lon_center / 360.0 * src_width as f64;
            let x_offset = (center_px - crop_w as f64 / 2.0).round() as i64;
            let x_offset = x_offset.rem_euclid(src_width as i64) as u32;
            let y_offset_f = (src_height as f64 - crop_h as f64) / 2.0
                - lat_center / 180.0 * src_height as f64;
            let y_offset = y_offset_f.round().clamp(0.0, (src_height - crop_h) as f64) as u32;
            eprintln!(
                "SphericalCrop: lon={lon_center}° lat={lat_center}° fov={fov_h}°×{fov_v}° \
                 → pixel offset ({x_offset},{y_offset}) crop {crop_w}×{crop_h}"
            );
            Operation::Crop { x_offset, y_offset, crop_w, crop_h }
        }
        other => other,
    };

    // ── Derive output dimensions ───────────────────────────────────────────────
    let (out_width, out_height) = match &op {
        Operation::Passthrough => (src_width, src_height),
        Operation::Crop { crop_w, crop_h, .. } => (*crop_w, *crop_h),
        Operation::SphericalCrop { .. } => unreachable!("normalised above"),
    };

    eprintln!("Input:  {src_width}×{src_height} @ {fps_num}/{fps_den} fps");
    eprintln!("Output: {out_width}×{out_height}");

    // Build progress bar using the frame count from the container (may be 0 if unknown).
    let total_frame_count = video_stream.frames() as u64;
    let pb = if total_frame_count > 0 {
        ProgressBar::new(total_frame_count)
    } else {
        ProgressBar::new_spinner()
    };
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.cyan} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} frames  ({per_sec})  eta {eta}",
        )
        .unwrap()
        .progress_chars("█▉▊▋▌▍▎▏ "),
    );

    // ── Create VideoToolbox format description from HEVC parameter sets ────────
    let format_desc = unsafe {
        let stream = input_ctx
            .streams()
            .best(Type::Video)
            .ok_or_else(|| anyhow!("no video stream"))?;
        extract_format_description(&stream)?
    };

    // ── Instantiate decode / encode / GPU / mux ────────────────────────────────
    let decoder = Decoder::new(
        CFRetained::as_ptr(&format_desc).as_ptr() as *mut _,
        src_width,
        src_height,
    )
    .context("Decoder::new")?;

    let mut encoder =
        Encoder::new(out_width, out_height, fps_num, fps_den, TARGET_BITRATE_BPS)
            .context("Encoder::new")?;

    let mut gpu = GpuContext::new(out_width, out_height).context("GpuContext::new")?;

    let mut muxer = Muxer::open(
        output_path,
        input_path,
        out_width,
        out_height,
        stream_timescale,
        frame_duration,
    )
    .context("Muxer::open")?;

    // Map the high-level Operation to the GPU op used per-frame.
    let gpu_op = match &op {
        Operation::Passthrough => GpuOp::Passthrough,
        Operation::Crop { x_offset, y_offset, .. } => GpuOp::Crop {
            x_offset: *x_offset,
            y_offset: *y_offset,
            src_width,
        },
        Operation::SphericalCrop { .. } => unreachable!("normalised above"),
    };

    // ── Main loop: read GOPs, decode batch, GPU-process, encode ───────────────
    let mut header_written = false;
    let mut input_ctx = input_ctx;

    let packets_iter: Vec<(ffmpeg::Stream, ffmpeg::Packet)> = input_ctx.packets().collect();
    let mut gop_buf: Vec<ffmpeg::Packet> = Vec::new();

    for (stream, packet) in packets_iter {
        if stream.index() != video_idx {
            continue;
        }

        // Flush the previous GOP when we hit a new keyframe.
        if packet.is_key() && !gop_buf.is_empty() {
            process_gop(
                &gop_buf,
                &decoder,
                &mut gpu,
                &mut encoder,
                &mut muxer,
                &gpu_op,
                &mut header_written,
                &format_desc,
                src_width,
                src_height,
                stream_timescale,
                &pb,
            )?;
            gop_buf.clear();
        }

        gop_buf.push(packet);
    }

    // Final (possibly partial) GOP.
    if !gop_buf.is_empty() {
        process_gop(
            &gop_buf,
            &decoder,
            &mut gpu,
            &mut encoder,
            &mut muxer,
            &gpu_op,
            &mut header_written,
            &format_desc,
            src_width,
            src_height,
            stream_timescale,
            &pb,
        )?;
    }

    // Flush encoder and write any remaining packets.
    for pkt in encoder.flush()? {
        write_packet(&mut muxer, &pkt, &mut header_written)?;
    }

    if !header_written {
        return Err(anyhow!(
            "No keyframe with extradata was produced — the encoder may have failed silently"
        ));
    }

    pb.finish_with_message("encoding done");
    eprintln!("Copying audio...");
    muxer.copy_audio(input_path).context("copy audio")?;
    muxer.finish().context("muxer finish")?;

    eprintln!("Done → {}", output_path.display());
    Ok(())
}

// ── Per-GOP processing ────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn process_gop(
    packets: &[ffmpeg::Packet],
    decoder: &Decoder,
    gpu: &mut GpuContext,
    encoder: &mut Encoder,
    muxer: &mut Muxer,
    gpu_op: &GpuOp,
    header_written: &mut bool,
    format_desc: &CFRetained<objc2_core_media::CMVideoFormatDescription>,
    src_width: u32,
    src_height: u32,
    stream_timescale: i32,
    pb: &ProgressBar,
) -> Result<()> {
    // Bounded reorder window: keeps at most REORDER_DEPTH frames in memory
    // at once. Once the window is full, the minimum-PTS frame is guaranteed
    // to be the next in display order and can be emitted immediately.
    // This handles B-frame reordering without buffering an entire GOP.
    // 8K BGRA is ~118 MB/frame; REORDER_DEPTH=8 caps RAM use at ~1 GB.
    const REORDER_DEPTH: usize = 8;
    let mut reorder: Vec<crate::decode::DecodedFrame> = Vec::with_capacity(REORDER_DEPTH + 4);

    for packet in packets {
        let sample_buf = unsafe {
            packet_to_sample_buffer(
                packet,
                CFRetained::as_ptr(format_desc).as_ptr() as *mut _,
                stream_timescale,
            )?
        };
        decoder.decode(&sample_buf)?;
        reorder.extend(decoder.drain_frames());

        // Emit the minimum-PTS frame whenever the window is full.
        while reorder.len() > REORDER_DEPTH {
            let min_idx = reorder
                .iter()
                .enumerate()
                .min_by_key(|(_, f)| f.pts.value)
                .map(|(i, _)| i)
                .unwrap();
            let frame = reorder.swap_remove(min_idx);
            process_frame(frame, gpu, encoder, muxer, gpu_op, src_width, src_height, header_written, pb)?;
        }
    }

    // Flush stragglers: wait for remaining async decode, then drain and sort.
    decoder.wait_for_async()?;
    reorder.extend(decoder.drain_frames());
    reorder.sort_unstable_by_key(|f| f.pts.value);
    for frame in reorder {
        process_frame(frame, gpu, encoder, muxer, gpu_op, src_width, src_height, header_written, pb)?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn process_frame(
    decoded: crate::decode::DecodedFrame,
    gpu: &mut GpuContext,
    encoder: &mut Encoder,
    muxer: &mut Muxer,
    gpu_op: &GpuOp,
    src_width: u32,
    src_height: u32,
    header_written: &mut bool,
    pb: &ProgressBar,
) -> Result<()> {
    let src_tex = gpu.wrap_input(&decoded.pixel_buffer, src_width as usize, src_height as usize)?;
    let out_buf = gpu.process(&*src_tex, gpu_op)?;
    encoder.encode_frame(&out_buf, decoded.pts)?;
    for pkt in encoder.drain_packets() {
        write_packet(muxer, &pkt, header_written)?;
    }
    pb.inc(1);
    Ok(())
}

fn write_packet(
    muxer: &mut Muxer,
    pkt: &crate::encode::EncodedPacket,
    header_written: &mut bool,
) -> Result<()> {
    if !*header_written {
        if let Some(ref ed) = pkt.extradata {
            muxer.write_header(ed)?;
            *header_written = true;
        }
    }
    if *header_written {
        muxer.write_video_packet(pkt)?;
    }
    Ok(())
}

// ── CoreMedia / VideoToolbox helpers ─────────────────────────────────────────

unsafe fn extract_format_description(
    stream: &ffmpeg_next::Stream,
) -> Result<CFRetained<objc2_core_media::CMVideoFormatDescription>> {
    use objc2_core_media::{
        CMFormatDescription, CMVideoFormatDescriptionCreateFromHEVCParameterSets,
    };

    let codec_par = stream.parameters();
    let extradata_ptr = (*codec_par.as_ptr()).extradata;
    let extradata_size = (*codec_par.as_ptr()).extradata_size as usize;
    if extradata_ptr.is_null() || extradata_size == 0 {
        return Err(anyhow!("no extradata in video stream"));
    }
    let extradata = std::slice::from_raw_parts(extradata_ptr, extradata_size);
    let param_sets = parse_hevc_extradata(extradata)?;

    let ptrs: Vec<NonNull<u8>> = param_sets
        .iter()
        .map(|s| NonNull::new(s.as_ptr() as *mut u8).unwrap())
        .collect();
    let sizes: Vec<usize> = param_sets.iter().map(|s| s.len()).collect();

    let mut format_desc_raw: *const CMFormatDescription = std::ptr::null();
    let rv = CMVideoFormatDescriptionCreateFromHEVCParameterSets(
        None,
        param_sets.len(),
        NonNull::new(ptrs.as_ptr() as *mut NonNull<u8>).unwrap(),
        NonNull::new(sizes.as_ptr() as *mut usize).unwrap(),
        4,
        None,
        NonNull::new(&mut format_desc_raw).unwrap(),
    );
    if rv != 0 {
        return Err(anyhow!(
            "CMVideoFormatDescriptionCreateFromHEVCParameterSets failed: {rv}"
        ));
    }
    Ok(CFRetained::from_raw(
        NonNull::new(format_desc_raw as *mut _).unwrap(),
    ))
}

/// Parse a HEVCDecoderConfigurationRecord (ISO 14496-15 §8.3.3) and return
/// each VPS / SPS / PPS NAL unit as a byte slice.
fn parse_hevc_extradata(data: &[u8]) -> Result<Vec<Vec<u8>>> {
    if data.len() < 23 {
        return Err(anyhow!("HEVC extradata too short ({} bytes)", data.len()));
    }
    let mut pos = 22;
    let num_arrays = data[pos] as usize;
    pos += 1;

    let mut result = Vec::new();
    for _ in 0..num_arrays {
        if pos + 3 > data.len() {
            break;
        }
        pos += 1; // array_completeness | reserved | NAL_unit_type
        let num_nalus = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        for _ in 0..num_nalus {
            if pos + 2 > data.len() {
                break;
            }
            let nalu_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;
            if pos + nalu_len > data.len() {
                break;
            }
            result.push(data[pos..pos + nalu_len].to_vec());
            pos += nalu_len;
        }
    }
    if result.is_empty() {
        return Err(anyhow!("no NAL units found in HEVC extradata"));
    }
    Ok(result)
}

/// Wrap an ffmpeg packet as a CMSampleBuffer for VTDecompressionSession.
unsafe fn packet_to_sample_buffer(
    packet: &ffmpeg_next::Packet,
    format_desc: *mut objc2_core_media::CMVideoFormatDescription,
    timescale: i32,
) -> Result<CFRetained<objc2_core_media::CMSampleBuffer>> {
    use objc2_core_foundation::kCFAllocatorNull;
    use objc2_core_media::kCMBlockBufferAlwaysCopyDataFlag;
    use objc2_core_media::{
        kCMTimeInvalid, CMBlockBuffer, CMFormatDescription, CMSampleBuffer, CMSampleTimingInfo,
        CMTime,
    };

    let data = packet.data().ok_or_else(|| anyhow!("empty packet"))?;
    let pts = packet.pts().unwrap_or(0);

    let mut block_buf: *mut CMBlockBuffer = std::ptr::null_mut();
    let rv = CMBlockBuffer::create_with_memory_block(
        None,
        data.as_ptr() as *mut c_void,
        data.len(),
        unsafe { kCFAllocatorNull },
        std::ptr::null(),
        0,
        data.len(),
        kCMBlockBufferAlwaysCopyDataFlag,
        NonNull::new(&mut block_buf).unwrap(),
    );
    if rv != 0 {
        return Err(anyhow!("CMBlockBufferCreateWithMemoryBlock failed: {rv}"));
    }
    let block_buf = CFRetained::from_raw(NonNull::new(block_buf).unwrap());

    let presentation_ts = CMTime::new(pts, timescale);
    let timing_info = CMSampleTimingInfo {
        duration: kCMTimeInvalid,
        presentationTimeStamp: presentation_ts,
        decodeTimeStamp: kCMTimeInvalid,
    };
    let size = data.len();
    let format_desc_ref: Option<&CMFormatDescription> =
        (format_desc as *const CMFormatDescription).as_ref();

    let mut sample_buf: *mut CMSampleBuffer = std::ptr::null_mut();
    let rv = CMSampleBuffer::create_ready(
        None,
        Some(&block_buf),
        format_desc_ref,
        1,
        1,
        &timing_info,
        1,
        &size,
        NonNull::new(&mut sample_buf).unwrap(),
    );
    if rv != 0 {
        return Err(anyhow!("CMSampleBufferCreateReady failed: {rv}"));
    }
    Ok(CFRetained::from_raw(NonNull::new(sample_buf).unwrap()))
}
