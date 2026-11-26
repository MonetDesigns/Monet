//! Windows Media Foundation H.264/MP4 encoder.
//!
//! The portable encoder lives in `figcraft-motion-codec`; this crate is an optional hardware
//! back end for Windows. It deliberately uses the Media Foundation sink writer instead of a
//! vendor SDK: Windows selects the H.264 encoder MFT for the installed GPU (NVIDIA, Intel,
//! AMD). `MfMp4Encoder::new` refuses a software MFT, so the engine falls back to the portable
//! encoder when no hardware encoder is available or a hardware encode fails.

#![cfg(windows)]

use std::{
    fs,
    path::PathBuf,
    ptr,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use figcraft_motion_codec::{FrameRgba, Mp4Options, rgba_to_yuv420_into, yuv_sizes};
use windows::{
    Win32::Media::MediaFoundation::{
        CODECAPI_AVEncCommonQuality, CODECAPI_AVEncCommonRateControlMode, IMFAttributes, IMFByteStream, IMFSinkWriter, IMFTransform,
        MF_MT_AVG_BITRATE, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MPEG2_PROFILE,
        MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_MT_TRANSFER_FUNCTION, MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_VIDEO_PRIMARIES, MF_MT_YUV_MATRIX,
        MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_VERSION, MFCreateAttributes, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
        MFCreateSinkWriterFromURL, MFMediaType_Video, MFNominalRange_16_235, MFSTARTUP_NOSOCKET, MFStartup, MFT_ENUM_HARDWARE_URL_Attribute,
        MFT_FRIENDLY_NAME_Attribute, MFVideoFormat_H264, MFVideoFormat_NV12, MFVideoInterlace_Progressive, MFVideoPrimaries_BT709,
        MFVideoTransFunc_709, MFVideoTransferMatrix_BT709, eAVEncCommonRateControlMode_Quality, eAVEncH264VProfile_High,
    },
    core::{GUID, HSTRING, Interface},
};

/// A Media Foundation sink writer configured for H.264 in MP4. Frames are straight RGBA8 and
/// are converted to NV12 on the caller's thread; opaque H.264 is the export target, so alpha is
/// flattened onto `background` exactly like the CPU encoder.
pub struct MfMp4Encoder {
    writer: Option<IMFSinkWriter>,
    stream: u32,
    path: PathBuf,
    w: u32,
    h: u32,
    fps: u32,
    sent: u32,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    nv12: Vec<u8>,
    background: [u8; 3],
}

impl MfMp4Encoder {
    pub fn new(w: u32, h: u32, o: &Mp4Options, background: [u8; 3], quality: u8) -> Result<Self, String> {
        let fps = o.fps.clamp(1, 120);
        if w == 0 || h == 0 || w % 2 == 1 || h % 2 == 1 {
            return Err("MP4 needs an even, non-zero picture size; lower the scale".into());
        }
        // Hardware H.264 MFTs commonly require at least a macroblock pair. Small previews are
        // cheaper on the CPU path anyway, and the caller falls back automatically.
        if w < 256 || h < 256 {
            return Err("Media Foundation H.264 needs a picture of at least 256x256".into());
        }
        mf_startup()?;

        let mut cleanup = TempFile { path: temp_path(), keep: false };
        let path = cleanup.path.clone();
        let path_h = HSTRING::from(path.to_string_lossy().as_ref());
        let mut attrs = None;
        unsafe { MFCreateAttributes(&mut attrs, 1) }.map_err(|e| format!("MFCreateAttributes: {e}"))?;
        let attrs = attrs.ok_or_else(|| "MFCreateAttributes returned no attributes".to_string())?;
        unsafe { attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1) }.map_err(|e| format!("enable hardware transforms: {e}"))?;

        let writer =
            unsafe { MFCreateSinkWriterFromURL(&path_h, None::<&IMFByteStream>, &attrs) }.map_err(|e| format!("MFCreateSinkWriterFromURL: {e}"))?;

        let out = unsafe { MFCreateMediaType() }.map_err(|e| format!("MFCreateMediaType(out): {e}"))?;
        unsafe {
            out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .and_then(|_| out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264))
                .and_then(|_| out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate_for(w, h, fps, quality)))
                .and_then(|_| out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32))
                .and_then(|_| out.SetUINT64(&MF_MT_FRAME_SIZE, pair(w, h)))
                .and_then(|_| out.SetUINT64(&MF_MT_FRAME_RATE, pair(fps, 1)))
                .and_then(|_| out.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pair(1, 1)))
                .and_then(|_| out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32))
        }
        .map_err(|e| format!("H.264 output type: {e}"))?;
        let stream = unsafe { writer.AddStream(&out) }.map_err(|e| format!("AddStream: {e}"))?;

        let input = unsafe { MFCreateMediaType() }.map_err(|e| format!("MFCreateMediaType(input): {e}"))?;
        unsafe {
            input
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .and_then(|_| input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12))
                .and_then(|_| input.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32))
                .and_then(|_| input.SetUINT64(&MF_MT_FRAME_SIZE, pair(w, h)))
                .and_then(|_| input.SetUINT64(&MF_MT_FRAME_RATE, pair(fps, 1)))
                .and_then(|_| input.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pair(1, 1)))
                .and_then(|_| input.SetUINT32(&MF_MT_DEFAULT_STRIDE, w))
                .and_then(|_| input.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32))
                .and_then(|_| input.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32))
                .and_then(|_| input.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32))
                .and_then(|_| input.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32))
        }
        .map_err(|e| format!("NV12 input type: {e}"))?;

        // Codec API parameters belong on the input stream. Quality-based VBR gives a CRF-like
        // result instead of spending the fixed average bitrate on easy frames. The average bitrate
        // on the output type remains as a fallback for MFTs that ignore quality mode.
        let mut enc_params = None;
        if unsafe { MFCreateAttributes(&mut enc_params, 2) }.is_ok()
            && let Some(ref params) = enc_params
        {
            let _ = unsafe { params.SetUINT32(&CODECAPI_AVEncCommonRateControlMode, eAVEncCommonRateControlMode_Quality.0 as u32) };
            // Export quality 85 is the UI default; Media Foundation's QP24/quality-70 point is the
            // closest match to the CPU encoder's default CRF 23.
            let _ = unsafe { params.SetUINT32(&CODECAPI_AVEncCommonQuality, quality.saturating_sub(15).clamp(1, 100) as u32) };
        }
        if let Err(e) = unsafe { writer.SetInputMediaType(stream, &input, enc_params.as_ref()) } {
            // Some third-party MFTs reject codec parameters on the input type; retry with the
            // plain NV12 type so the fixed average bitrate still gets a hardware attempt.
            eprintln!("[motion] Media Foundation rejected quality parameters, retrying defaults: {e}");
            unsafe { writer.SetInputMediaType(stream, &input, None::<&IMFAttributes>) }.map_err(|e| format!("SetInputMediaType: {e}"))?;
        }

        if !uses_hardware_encoder(&writer, stream) {
            return Err(format!("Media Foundation selected a software H.264 encoder ({:?})", encoder_name(&writer, stream)));
        }

        unsafe { writer.BeginWriting() }.map_err(|e| format!("BeginWriting: {e}"))?;

        let (ys, cs) = yuv_sizes(w as usize, h as usize);
        cleanup.keep = true;
        Ok(Self {
            writer: Some(writer),
            stream,
            path,
            w,
            h,
            fps,
            sent: 0,
            y: vec![0; ys],
            u: vec![128; cs],
            v: vec![128; cs],
            nv12: vec![0; ys + cs * 2],
            background,
        })
    }

    pub fn push(&mut self, f: FrameRgba) -> Result<(), String> {
        if (f.width, f.height) != (self.w, self.h) {
            return Err("all frames must have the same size".into());
        }
        rgba_to_yuv420_into(&f.rgba, self.w as usize, self.h as usize, self.background, &mut self.y, &mut self.u, &mut self.v);
        let cw = (self.w as usize) / 2;
        let ch = (self.h as usize) / 2;
        let ys = self.y.len();
        self.nv12[..ys].copy_from_slice(&self.y);
        for row in 0..ch {
            let src = row * cw;
            let dst = ys + row * self.w as usize;
            for col in 0..cw {
                self.nv12[dst + col * 2] = self.u[src + col];
                self.nv12[dst + col * 2 + 1] = self.v[src + col];
            }
        }

        let len = self.nv12.len();
        let buffer = unsafe { MFCreateMemoryBuffer(len as u32) }.map_err(|e| format!("MFCreateMemoryBuffer: {e}"))?;
        let mut dst = ptr::null_mut();
        let mut max_len = 0u32;
        unsafe { buffer.Lock(&mut dst, Some(&mut max_len), None) }.map_err(|e| format!("Lock sample buffer: {e}"))?;
        if dst.is_null() || max_len < len as u32 {
            let _ = unsafe { buffer.Unlock() };
            return Err("Media Foundation sample buffer is too small".into());
        }
        unsafe {
            ptr::copy_nonoverlapping(self.nv12.as_ptr(), dst, len);
            buffer.Unlock().map_err(|e| format!("Unlock sample buffer: {e}"))?;
            buffer.SetCurrentLength(len as u32).map_err(|e| format!("SetCurrentLength: {e}"))?;
        }

        let sample = unsafe { MFCreateSample() }.map_err(|e| format!("MFCreateSample: {e}"))?;
        unsafe { sample.AddBuffer(&buffer) }.map_err(|e| format!("AddBuffer: {e}"))?;
        let time = i64::from(self.sent) * 10_000_000 / i64::from(self.fps);
        let duration = 10_000_000 / i64::from(self.fps);
        unsafe {
            sample.SetSampleTime(time).map_err(|e| format!("SetSampleTime: {e}"))?;
            sample.SetSampleDuration(duration).map_err(|e| format!("SetSampleDuration: {e}"))?;
        }
        let writer = self.writer.as_ref().ok_or_else(|| "Media Foundation writer is already finished".to_string())?;
        unsafe { writer.WriteSample(self.stream, &sample) }.map_err(|e| format!("WriteSample: {e}"))?;
        self.sent += 1;
        Ok(())
    }

    pub fn finish(mut self) -> Result<Vec<u8>, String> {
        if self.sent == 0 {
            return Err("H.264 encoder produced no frames".into());
        }
        let writer = self.writer.take().ok_or_else(|| "Media Foundation writer is already finished".to_string())?;
        unsafe { writer.Finalize() }.map_err(|e| format!("Finalize: {e}"))?;
        drop(writer);
        fs::read(&self.path).map_err(|e| format!("read MP4: {e}"))
    }
}

impl Drop for MfMp4Encoder {
    fn drop(&mut self) {
        self.writer.take();
        let _ = fs::remove_file(&self.path);
    }
}

fn mf_startup() -> Result<(), String> {
    static START: OnceLock<Result<(), String>> = OnceLock::new();
    START.get_or_init(|| unsafe { MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET) }.map_err(|e| format!("MFStartup: {e}"))).clone()
}

/// Query the MFT the sink writer actually selected. Hardware MFTs expose a device symbolic link
/// in `MFT_ENUM_HARDWARE_URL_Attribute`; software MFTs do not. If the service query is unsupported
/// we conservatively use the CPU path rather than silently shipping software-encoded output.
fn uses_hardware_encoder(writer: &IMFSinkWriter, stream: u32) -> bool {
    unsafe {
        let mut raw: *mut core::ffi::c_void = ptr::null_mut();
        if writer.GetServiceForStream(stream, &GUID::zeroed(), &IMFTransform::IID, &mut raw).is_err() || raw.is_null() {
            return false;
        }
        let transform = IMFTransform::from_raw(raw);
        let Ok(attrs) = transform.GetAttributes() else { return false };
        let Ok(len) = attrs.GetStringLength(&MFT_ENUM_HARDWARE_URL_Attribute) else { return false };
        let mut name = vec![0u16; len as usize + 1];
        let mut out_len = 0u32;
        attrs.GetString(&MFT_ENUM_HARDWARE_URL_Attribute, &mut name, Some(&mut out_len)).is_ok() && out_len > 0
    }
}

fn encoder_name(writer: &IMFSinkWriter, stream: u32) -> Option<String> {
    unsafe {
        let mut raw: *mut core::ffi::c_void = ptr::null_mut();
        writer.GetServiceForStream(stream, &GUID::zeroed(), &IMFTransform::IID, &mut raw).ok()?;
        if raw.is_null() {
            return None;
        }
        let transform = IMFTransform::from_raw(raw);
        let attrs = transform.GetAttributes().ok()?;
        string_attribute(&attrs, &MFT_FRIENDLY_NAME_Attribute)
    }
}

fn string_attribute(attrs: &IMFAttributes, key: &GUID) -> Option<String> {
    unsafe {
        let len = attrs.GetStringLength(key).ok()?;
        let mut buf = vec![0u16; len as usize + 1];
        let mut out_len = 0u32;
        attrs.GetString(key, &mut buf, Some(&mut out_len)).ok()?;
        Some(String::from_utf16_lossy(&buf[..out_len as usize]))
    }
}

fn pair(hi: u32, lo: u32) -> u64 {
    (u64::from(hi) << 32) | u64::from(lo)
}

struct TempFile {
    path: PathBuf,
    keep: bool,
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn bitrate_for(w: u32, h: u32, fps: u32, quality: u8) -> u32 {
    let q = f64::from(quality.clamp(1, 100)) / 100.0;
    let bpp = 0.03 + 0.12 * q;
    (f64::from(w) * f64::from(h) * f64::from(fps) * bpp).round().clamp(200_000.0, 120_000_000.0) as u32
}

fn temp_path() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |t| t.as_nanos());
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("monet-motion-{}-{nonce}-{n}.mp4", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Manual smoke test for a machine with a hardware H.264 MFT. It is ignored in normal CI
    /// because GitHub's Windows runners do not expose a GPU encoder; run it locally with
    /// `cargo test -p figcraft-motion-mediafoundation -- --ignored`.
    #[test]
    #[ignore = "requires a Windows GPU with a hardware H.264 encoder"]
    fn hardware_h264_writes_a_real_mp4() {
        let o = Mp4Options { fps: 30, crf: 23.0 };
        let mut enc = MfMp4Encoder::new(256, 256, &o, [255, 255, 255], 85).expect("hardware H.264 MFT");
        for i in 0..10u32 {
            let mut rgba = vec![0u8; 256 * 256 * 4];
            for y in 0..256usize {
                for x in 0..256usize {
                    let p = (y * 256 + x) * 4;
                    rgba[p] = (x as u8).saturating_add(i as u8);
                    rgba[p + 1] = y as u8;
                    rgba[p + 2] = 200;
                    rgba[p + 3] = 255;
                }
            }
            enc.push(FrameRgba::new(256, 256, rgba).unwrap()).expect("push");
        }
        let bytes = enc.finish().expect("finish");
        assert_eq!(&bytes[4..8], b"ftyp", "ISO BMFF");
        assert!(bytes.windows(4).any(|w| w == b"avc1"), "H.264 sample entry");
        assert!(bytes.len() > 1024, "real encoded frames");
    }

    /// Local throughput probe; prints wall time for a 1080p/30fps/5s stream.
    #[test]
    #[ignore = "requires a Windows GPU with a hardware H.264 encoder"]
    fn hardware_h264_1080p_probe() {
        use std::time::Instant;

        let o = Mp4Options { fps: 30, crf: 23.0 };
        let mut enc = MfMp4Encoder::new(1920, 1080, &o, [255, 255, 255], 85).expect("hardware H.264 MFT");
        let mut rgba = vec![0u8; 1920 * 1080 * 4];
        for y in 0..1080usize {
            for x in 0..1920usize {
                let p = (y * 1920 + x) * 4;
                rgba[p] = (x % 256) as u8;
                rgba[p + 1] = (y % 256) as u8;
                rgba[p + 2] = 128;
                rgba[p + 3] = 255;
            }
        }
        let frame = FrameRgba::new(1920, 1080, rgba).unwrap();
        let start = Instant::now();
        for _ in 0..150 {
            enc.push(frame.clone()).expect("push");
        }
        let bytes = enc.finish().expect("finish");
        let ms = start.elapsed().as_millis();
        eprintln!("[motion] MF H.264 1080p150: {ms} ms, {} bytes", bytes.len());
    }
}
