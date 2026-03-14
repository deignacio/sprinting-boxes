use anyhow::{anyhow, Context, Result};
use ffmpeg::format;
use ffmpeg::media::Type;
use ffmpeg_next as ffmpeg;
use indicatif::{ProgressBar, ProgressStyle};
use objc2_core_foundation::CFRetained;
use std::path::Path;
use std::ptr::NonNull;

use crate::decode::Decoder;
use crate::encode::EncodedPacket;
use crate::encode::Encoder;
use crate::gpu::{GpuContext, RotationMatrix};
use crate::mux::Muxer;
use std::collections::BTreeMap;
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::thread;

pub struct Gop {
    pub index: usize,
    pub packets: Vec<ffmpeg::Packet>,
}

pub struct GopResult {
    pub index: usize,
    pub packets: Vec<EncodedPacket>,
}

// Target bitrate for HEVC output: 80 Mbps is appropriate for 4K fisheye.
const TARGET_BITRATE_BPS: i32 = 80_000_000;

pub fn run(
    input_path: &Path,
    output_path: &Path,
    yaw_deg: f32,
    pitch_deg: f32,
    roll_deg: f32,
) -> Result<()> {
    ffmpeg::init().context("ffmpeg init")?;

    // --- Probe input ---
    let input_ctx = format::input(input_path).context("open input")?;
    let video_stream = input_ctx
        .streams()
        .best(Type::Video)
        .ok_or_else(|| anyhow!("no video stream in input"))?;
    let video_idx = video_stream.index();

    // Read codec parameters directly from AVCodecParameters (works with ffmpeg 5+).
    let (src_width, src_height) = unsafe {
        let par = video_stream.parameters();
        let raw = par.as_ptr();
        ((*raw).width as u32, (*raw).height as u32)
    };
    let fps = video_stream.avg_frame_rate();
    let fps_num = fps.numerator();
    let fps_den = fps.denominator();

    eprintln!("Input: {src_width}×{src_height} @ {fps_num}/{fps_den} fps");
    eprintln!(
        "Output: {}×{} fisheye (yaw={yaw_deg}° pitch={pitch_deg}° roll={roll_deg}°)",
        src_width / 2,
        src_width / 2
    );

    let total_frames = video_stream.frames() as u64;
    let pb = if total_frames > 0 {
        ProgressBar::new(total_frames)
    } else {
        ProgressBar::new_spinner() // Variable frame count
    };
    let pb = Arc::new(pb);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ({per_sec}) ({eta})",
            )
            .unwrap()
            .progress_chars("#>-"),
    );

    // Precompute rotation matrix from degrees → radians.
    let rotation = RotationMatrix::from_yaw_pitch_roll(
        yaw_deg.to_radians(),
        pitch_deg.to_radians(),
        roll_deg.to_radians(),
    );

    // --- Decoder & Encoder ---
    let video_stream = input_ctx
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .ok_or_else(|| anyhow!("no video stream found"))?;
    let format_desc = unsafe { extract_format_description(&video_stream)? };

    // --- Build VideoToolbox encoder (removed from main thread) ---
    let out_size = src_width / 2;

    // Compute the stream timebase and per-frame duration in that timebase.
    // e.g. for ~29.97fps: time_base=1/30000, avg_frame_rate=5904000/196997,
    // frame_duration = round(30000 * 196997 / 5904000) = 1001.
    let stream_timescale = video_stream.time_base().denominator() as i32;
    let frame_duration = {
        let num = stream_timescale as i64 * fps_den as i64;
        let den = fps_num as i64;
        (num + den / 2) / den // rounded integer division
    };

    // --- Open muxer ---
    let mut muxer = Muxer::open(
        output_path,
        input_path,
        out_size,
        out_size,
        stream_timescale,
        frame_duration,
    )
    .context("Muxer::open")?;

    // --- Pipeline Channels ---
    let (gop_tx, gop_rx) = sync_channel::<Gop>(4); // Hand off 1 GOP at a time
    let gop_rx = Arc::new(Mutex::new(gop_rx));
    let (result_tx, result_rx) = sync_channel::<GopResult>(8);

    // --- Muxer Thread (Sequencer) ---
    let input_path_for_audio = input_path.to_path_buf();
    let muxer_handle = thread::spawn(move || -> Result<()> {
        let mut sequencer = BTreeMap::new();
        let mut next_expected_index = 0;
        let mut header_written = false;

        while let Ok(res) = result_rx.recv() {
            sequencer.insert(res.index, res.packets);

            while let Some(packets) = sequencer.remove(&next_expected_index) {
                for pkt in packets {
                    // Write the MP4 header once we have extradata from the first keyframe.
                    if !header_written {
                        if let Some(ref ed) = pkt.extradata {
                            muxer.write_header(ed)?;
                            header_written = true;
                        }
                    }
                    if header_written {
                        muxer.write_video_packet(&pkt)?;
                    }
                }
                next_expected_index += 1;
            }
        }

        if !header_written {
            return Err(anyhow!("no keyframe with extradata was produced — cannot write output"));
        }

        eprintln!("Copying audio...");
        muxer
            .copy_audio(&input_path_for_audio)
            .context("copy audio")?;
        muxer.finish().context("muxer finish")?;
        Ok(())
    });

    // --- Parallel Worker Threads ---
    let mut worker_handles = Vec::new();
    let num_workers = 4; // 2 engines * 2 workers per engine to maximize ASIC occupancy.

    // We need to move format_desc and other params into the workers.
    for _i in 0..num_workers {
        let gop_rx = gop_rx.clone();
        let result_tx = result_tx.clone();
        let format_desc = format_desc.clone();
        let rotation = rotation.clone();
        let pb = pb.clone();

        let handle = thread::spawn(move || -> Result<()> {
            let mut gpu = GpuContext::new(src_width, src_height).context("GpuContext::new")?;
            let mut encoder =
                Encoder::new(out_size, out_size, fps_num, fps_den, TARGET_BITRATE_BPS)
                    .context("Encoder::new")?;
            let decoder = Decoder::new(
                objc2_core_foundation::CFRetained::as_ptr(&format_desc).as_ptr() as *mut _,
                src_width,
                src_height,
            )
            .context("create decoder")?;

            loop {
                let gop = {
                    let lock = gop_rx.lock().unwrap();
                    match lock.recv() {
                        Ok(gop) => gop,
                        Err(_) => break, // Channel closed
                    }
                };

                let mut encoded_gop_packets = Vec::new();
                let gop_index = gop.index;

                let res = (|| -> Result<()> {
                    for packet in gop.packets {
                        let format_desc_ptr =
                            objc2_core_foundation::CFRetained::as_ptr(&format_desc).as_ptr();
                        let sample_buf = unsafe {
                            packet_to_sample_buffer(
                                &packet,
                                format_desc_ptr as *mut _,
                                stream_timescale,
                            )?
                        };

                        decoder.decode(&sample_buf)?;
                    }

                    // Block until VT delivers all decoded frames for this GOP, then encode them.
                    decoder.wait_for_async()?;
                    for decoded in decoder.drain_frames() {
                        let src_texture = gpu.wrap_pixel_buffer_as_texture(
                            &decoded.pixel_buffer,
                            src_width as usize,
                            src_height as usize,
                        )?;
                        let out_pix = gpu.reproject(&*src_texture, &rotation)?;
                        encoder.encode_frame(&out_pix, decoded.pts)?;
                        pb.inc(1);
                        for pkt in encoder.drain_packets() {
                            encoded_gop_packets.push(pkt);
                        }
                    }

                    for pkt in encoder.flush()? {
                        encoded_gop_packets.push(pkt);
                    }
                    Ok(())
                })();

                if let Err(e) = res {
                    eprintln!("Worker error on GOP {gop_index}: {e:?}");
                }

                result_tx
                    .send(GopResult {
                        index: gop_index,
                        packets: encoded_gop_packets,
                    })
                    .ok();
            }
            Ok(())
        });
        worker_handles.push(handle);
    }

    // Drop our clone of result_tx so the receiver can close when workers finish
    drop(result_tx);

    // --- Main Thread Dispatcher ---
    let mut current_gop_index = 0;
    let mut current_gop_packets = Vec::new();
    let mut input_ctx = input_ctx;

    for (stream, packet) in input_ctx.packets() {
        if stream.index() != video_idx {
            continue;
        }

        if packet.is_key() && !current_gop_packets.is_empty() {
            if gop_tx
                .send(Gop {
                    index: current_gop_index,
                    packets: std::mem::take(&mut current_gop_packets),
                })
                .is_err()
            {
                break;
            }
            current_gop_index += 1;
        }

        current_gop_packets.push(packet);
    }

    // Last GOP
    if !current_gop_packets.is_empty() {
        gop_tx
            .send(Gop {
                index: current_gop_index,
                packets: current_gop_packets,
            })
            .ok();
    }
    drop(gop_tx);

    for handle in worker_handles {
        handle
            .join()
            .map_err(|e| anyhow!("worker thread panicked: {:?}", e))??;
    }
    muxer_handle
        .join()
        .map_err(|e| anyhow!("muxer thread panicked: {:?}", e))??;

    pb.finish_with_message("Done");
    eprintln!("Done. Processed {} GOPs.", current_gop_index + 1);
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers: bridge ffmpeg AVPacket/AVCodecParameters ↔ CoreMedia types
// ---------------------------------------------------------------------------

unsafe fn extract_format_description(
    stream: &ffmpeg_next::Stream,
) -> Result<CFRetained<objc2_core_media::CMVideoFormatDescription>> {
    use objc2_core_media::{
        CMFormatDescription, CMVideoFormatDescriptionCreateFromHEVCParameterSets,
    };

    // Extract extradata (parameter sets) from AVCodecParameters.
    let codec_par = stream.parameters();
    let extradata_ptr = (*codec_par.as_ptr()).extradata;
    let extradata_size = (*codec_par.as_ptr()).extradata_size as usize;
    if extradata_ptr.is_null() || extradata_size == 0 {
        return Err(anyhow!(
            "no extradata in video stream — cannot build format description"
        ));
    }
    let extradata = std::slice::from_raw_parts(extradata_ptr, extradata_size);

    // Parse HEVCDecoderConfigurationRecord to extract VPS/SPS/PPS.
    let param_sets = parse_hevc_extradata(extradata)?;

    // Build NonNull<NonNull<u8>> and NonNull<usize> as required by the new API.
    let ptrs: Vec<NonNull<u8>> = param_sets
        .iter()
        .map(|s| NonNull::new(s.as_ptr() as *mut u8).unwrap())
        .collect();
    let sizes: Vec<usize> = param_sets.iter().map(|s| s.len()).collect();

    let mut format_desc_raw: *const CMFormatDescription = std::ptr::null();
    let rv = CMVideoFormatDescriptionCreateFromHEVCParameterSets(
        None, // allocator
        param_sets.len(),
        NonNull::new(ptrs.as_ptr() as *mut NonNull<u8>).unwrap(),
        NonNull::new(sizes.as_ptr() as *mut usize).unwrap(),
        4, // NAL unit length size
        None,
        NonNull::new(&mut format_desc_raw).unwrap(),
    );
    if rv != 0 {
        return Err(anyhow!(
            "CMVideoFormatDescriptionCreateFromHEVCParameterSets failed: {rv}"
        ));
    }
    // Cast *const CMFormatDescription to *mut CMVideoFormatDescription.
    Ok(unsafe { CFRetained::from_raw(NonNull::new(format_desc_raw as *mut _).unwrap()) })
}

/// Parse the HEVCDecoderConfigurationRecord (ISO 14496-15 §8.3.3) and return
/// the raw VPS, SPS, and PPS NAL unit byte slices.
fn parse_hevc_extradata(data: &[u8]) -> Result<Vec<Vec<u8>>> {
    // Minimal parser: skip 22-byte header, then read numOfArrays.
    if data.len() < 23 {
        return Err(anyhow!("HEVC extradata too short"));
    }
    let mut pos = 22;
    let num_arrays = data[pos] as usize;
    pos += 1;

    let mut result = Vec::new();
    for _ in 0..num_arrays {
        if pos + 3 > data.len() {
            break;
        }
        pos += 1; // array_completeness + reserved + NAL_unit_type
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

/// Wrap an ffmpeg AVPacket as a CMSampleBuffer for VTDecompressionSession.
unsafe fn packet_to_sample_buffer(
    packet: &ffmpeg_next::Packet,
    format_desc: *mut objc2_core_media::CMVideoFormatDescription,
    timescale: i32,
) -> Result<CFRetained<objc2_core_media::CMSampleBuffer>> {
    use objc2_core_media::CMBlockBuffer;
    use objc2_core_media::{
        kCMTimeInvalid, CMFormatDescription, CMSampleBuffer, CMSampleTimingInfo, CMTime,
    };

    let data = packet.data().ok_or_else(|| anyhow!("empty packet"))?;
    let pts = packet.pts().unwrap_or(0);

    // CMBlockBuffer wrapping the packet bytes.
    // We use kCMBlockBufferAlwaysCopyDataFlag to ensure the data is copied.
    // We use kCFAllocatorNull to prevent the block buffer from trying to
    // deallocate the original FFmpeg packet data pointer.
    use objc2_core_foundation::kCFAllocatorNull;
    use objc2_core_media::kCMBlockBufferAlwaysCopyDataFlag;

    let mut block_buf: *mut CMBlockBuffer = std::ptr::null_mut();
    let rv = CMBlockBuffer::create_with_memory_block(
        None,
        data.as_ptr() as *mut c_void,
        data.len(),
        unsafe { kCFAllocatorNull }, // Do not deallocate the source pointer
        std::ptr::null(),            // custom block source
        0,
        data.len(),
        kCMBlockBufferAlwaysCopyDataFlag,
        NonNull::new(&mut block_buf).unwrap(),
    );
    if rv != 0 {
        return Err(anyhow!("CMBlockBufferCreateWithMemoryBlock failed: {rv}"));
    }
    let block_buf = unsafe { CFRetained::from_raw(NonNull::new(block_buf).unwrap()) };

    let presentation_ts = unsafe { CMTime::new(pts, timescale) };
    let timing_info = CMSampleTimingInfo {
        duration: kCMTimeInvalid,
        presentationTimeStamp: presentation_ts,
        decodeTimeStamp: kCMTimeInvalid,
    };
    let size = data.len();

    let format_desc_ref: Option<&CMFormatDescription> =
        unsafe { (format_desc as *const CMFormatDescription).as_ref() };

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

    Ok(unsafe { CFRetained::from_raw(NonNull::new(sample_buf).unwrap()) })
}

use std::ffi::c_void;
