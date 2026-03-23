use anyhow::{anyhow, Context, Result};
use ffmpeg::codec;
use ffmpeg::format::{self, context::Output};
use ffmpeg::media::Type;
use ffmpeg::Rational;
use ffmpeg_next as ffmpeg;
use ffmpeg_next::packet::Mut;
use std::path::Path;

use crate::encode::EncodedPacket;

/// MP4 muxer: receives encoded HEVC packets + passthrough audio from an input file.
pub struct Muxer {
    output: Output,
    video_stream_idx: usize,
    audio_stream_idx: Option<usize>,
    video_time_base: Rational,
    audio_time_base: Option<Rational>,
    frame_duration: i64,
}

impl Muxer {
    pub fn open(
        output_path: &Path,
        input_path: &Path,
        out_width: u32,
        out_height: u32,
        stream_timescale: i32,
        frame_duration: i64,
    ) -> Result<Self> {
        ffmpeg::init().context("ffmpeg::init")?;

        let mut output = format::output(output_path).context("open output")?;
        let input_ctx = format::input(input_path).context("open input")?;

        // --- Video stream ---
        // Copy all codec parameters from the input stream so that color space (bt709),
        // full/limited range, codec tag (hvc1), pixel format, etc. are preserved exactly.
        // Only the dimensions and extradata are overridden.
        let video_time_base = Rational::new(1, stream_timescale);
        let input_video = input_ctx
            .streams()
            .best(Type::Video)
            .ok_or_else(|| anyhow!("no video stream in input"))?;

        let mut video_stream = output
            .add_stream(codec::encoder::find(codec::Id::HEVC))
            .context("add video stream")?;
        video_stream.set_time_base(video_time_base);

        unsafe {
            let src = input_video.parameters().as_ptr();
            let dst = video_stream.parameters().as_mut_ptr();
            let rv = ffmpeg_next::ffi::avcodec_parameters_copy(dst, src);
            if rv < 0 {
                return Err(anyhow!("avcodec_parameters_copy failed: {rv}"));
            }
            // Override dimensions (matters for crop; same as source for passthrough).
            (*dst).width = out_width as i32;
            (*dst).height = out_height as i32;
            // Clear extradata — the encoder's hvcC record is injected later via write_header().
            (*dst).extradata = std::ptr::null_mut();
            (*dst).extradata_size = 0;
            // codec_tag=0 lets the MOV muxer choose hvc1 (the correct out-of-band tag for HEVC).
            (*dst).codec_tag = 0;
        }

        // Copy stream-level side data (Spherical Mapping, Stereo 3D, etc.).
        // FFmpeg 8.0: side data lives in AVCodecParameters.coded_side_data (not AVStream).
        unsafe {
            let src_par = input_video.parameters().as_ptr();
            let dst_par = video_stream.parameters().as_mut_ptr();
            let nb = (*src_par).nb_coded_side_data as usize;
            for i in 0..nb {
                let sd = &*(*src_par).coded_side_data.add(i);
                // av_packet_side_data_add takes ownership of the data pointer.
                let data_copy = ffmpeg_next::ffi::av_memdup(sd.data as *const _, sd.size);
                if !data_copy.is_null() {
                    ffmpeg_next::ffi::av_packet_side_data_add(
                        &mut (*dst_par).coded_side_data,
                        &mut (*dst_par).nb_coded_side_data,
                        sd.type_,
                        data_copy,
                        sd.size,
                        0,
                    );
                }
            }
        }

        let video_stream_idx = video_stream.index();

        // --- Audio stream: copy from input ---
        let (audio_stream_idx, audio_time_base) =
            if let Some(audio_in) = input_ctx.streams().best(Type::Audio) {
                let mut audio_out = output
                    .add_stream(codec::encoder::find(codec::Id::None))
                    .context("add audio stream")?;
                audio_out.set_parameters(audio_in.parameters());
                unsafe {
                    (*audio_out.parameters().as_mut_ptr()).codec_tag = 0;
                }
                let tb = audio_in.time_base();
                audio_out.set_time_base(tb);
                (Some(audio_out.index()), Some(tb))
            } else {
                (None, None)
            };

        Ok(Self {
            output,
            video_stream_idx,
            audio_stream_idx,
            video_time_base,
            audio_time_base,
            frame_duration,
        })
    }

    /// Set the HEVCDecoderConfigurationRecord extradata on the video stream and write the MP4 header.
    /// Must be called exactly once, before any write_video_packet calls.
    pub fn write_header(&mut self, extradata: &[u8]) -> Result<()> {
        unsafe {
            let mut par = self.output.stream(self.video_stream_idx).unwrap().parameters();
            let raw = par.as_mut_ptr();
            let buf = ffmpeg_next::ffi::av_malloc(extradata.len()) as *mut u8;
            if buf.is_null() {
                return Err(anyhow!("av_malloc failed for extradata"));
            }
            std::ptr::copy_nonoverlapping(extradata.as_ptr(), buf, extradata.len());
            (*raw).extradata = buf;
            (*raw).extradata_size = extradata.len() as i32;
        }
        self.output.write_header().context("write MP4 header")
    }

    /// Write an encoded HEVC packet into the video stream.
    pub fn write_video_packet(&mut self, pkt: &EncodedPacket) -> Result<()> {
        // VideoToolbox outputs HVCC (4-byte big-endian length-prefixed NAL units), which
        // is the correct format for MP4 mdat. Use av_new_packet so ffmpeg owns the buffer.
        let data = &pkt.data;

        let mut ffpkt = ffmpeg::Packet::empty();
        unsafe {
            let inner = ffpkt.as_mut_ptr();
            let rv = ffmpeg_next::ffi::av_new_packet(inner, data.len() as i32);
            if rv < 0 {
                return Err(anyhow!("av_new_packet failed: {rv}"));
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), (*inner).data, data.len());
            (*inner).stream_index = self.video_stream_idx as i32;
            (*inner).pts = pkt.pts;
            (*inner).dts = pkt.dts;
            (*inner).duration = self.frame_duration;
            (*inner).time_base = ffmpeg_next::ffi::AVRational {
                num: self.video_time_base.numerator(),
                den: self.video_time_base.denominator(),
            };
            if pkt.is_keyframe {
                (*inner).flags |= ffmpeg_next::ffi::AV_PKT_FLAG_KEY as i32;
            }
        }

        ffpkt
            .write(&mut self.output)
            .context("write video packet")?;
        Ok(())
    }

    /// Copy all audio packets from the input file into the output.
    /// Call this after all video packets have been written.
    pub fn copy_audio(&mut self, input_path: &Path) -> Result<()> {
        let audio_stream_idx = match self.audio_stream_idx {
            Some(idx) => idx,
            None => return Ok(()), // no audio in source
        };
        let out_tb = self.audio_time_base.unwrap();

        let mut input_ctx = format::input(input_path).context("open input for audio copy")?;
        let audio_in_idx = input_ctx
            .streams()
            .best(Type::Audio)
            .ok_or_else(|| anyhow!("audio stream disappeared"))?
            .index();
        let in_tb = input_ctx.stream(audio_in_idx).unwrap().time_base();

        for (stream, mut packet) in input_ctx.packets() {
            if stream.index() != audio_in_idx {
                continue;
            }
            packet.rescale_ts(in_tb, out_tb);
            packet.set_stream(audio_stream_idx);
            packet.set_position(-1);
            packet
                .write(&mut self.output)
                .context("write audio packet")?;
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<()> {
        self.output.write_trailer().context("write MP4 trailer")?;
        Ok(())
    }
}
