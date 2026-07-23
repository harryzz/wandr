// Phase-1: DXVA2 / ID3D11VideoDecoder H.264 decode with REFERENCE frames.
//
// Reuses cros-codecs' pure-Rust parser (codec module, Windows-buildable). The
// cros-codecs DECODER DRIVER is gated behind `backend` (Linux gbm/drm/nix), and
// its VideoFrame handle trait is V4L2/DRM-coupled — so this is a THIN Windows
// driver: parser + POC(type-0) + a minimal sliding-window DPB + the DXVA backend.
// (The heavy DPB algorithms in codec::h264::dpb are reused in Phase-1b/test-25fps.)
//
// Verified bit-exact per frame vs the ffmpeg framehash CRC references.

#![allow(non_snake_case, non_camel_case_types)]

use std::io::Cursor;
use std::mem::{size_of, zeroed};

use cros_codecs::codec::h264::parser::{Nalu, NaluType, Parser, Pps, SliceHeader, Sps};

use windows::core::{Interface, GUID};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC};

const H264_VLD_NOFGT: GUID = GUID::from_u128(0x1b81be68_a0c7_11d3_b984_00c04f2e73c5);

// which clip to run — arg[1] = "i" (single IDR), "ip" (I+P), default "ip"
const CLIP_I: &[u8] = include_bytes!(
    "../../../runtime/wandr-host/vendor/cros-codecs/src/codec/h264/test_data/64x64-I.h264"
);
const CRC_I: &str = include_str!(
    "../../../runtime/wandr-host/vendor/cros-codecs/src/codec/h264/test_data/64x64-I.h264.crc"
);
const CLIP_IP: &[u8] = include_bytes!(
    "../../../runtime/wandr-host/vendor/cros-codecs/src/codec/h264/test_data/64x64-I-P.h264"
);
const CRC_IP: &str = include_str!(
    "../../../runtime/wandr-host/vendor/cros-codecs/src/codec/h264/test_data/64x64-I-P.h264.crc"
);
const CLIP_IPBP: &[u8] = include_bytes!(
    "../../../runtime/wandr-host/vendor/cros-codecs/src/codec/h264/test_data/64x64-I-P-B-P.h264"
);
const CRC_IPBP: &str = include_str!(
    "../../../runtime/wandr-host/vendor/cros-codecs/src/codec/h264/test_data/64x64-I-P-B-P.h264.crc"
);
const CLIP_25: &[u8] = include_bytes!(
    "../../../runtime/wandr-host/vendor/cros-codecs/src/codec/h264/test_data/test-25fps.h264"
);
const CRC_25: &str = include_str!(
    "../../../runtime/wandr-host/vendor/cros-codecs/src/codec/h264/test_data/test-25fps.h264.crc"
);

// ---------------------------------------------------------------------------
// hand-defined DXVA structs (must match dxva.h — driver-facing)
// ---------------------------------------------------------------------------
type DXVA_PicEntry = u8;

#[repr(C)]
#[derive(Clone, Copy)]
struct DXVA_PicParams_H264 {
    wFrameWidthInMbsMinus1: u16,
    wFrameHeightInMbsMinus1: u16,
    CurrPic: DXVA_PicEntry,
    num_ref_frames: u8,
    wBitFields: u16,
    bit_depth_luma_minus8: u8,
    bit_depth_chroma_minus8: u8,
    Reserved16Bits: u16,
    StatusReportFeedbackNumber: u32,
    RefFrameList: [DXVA_PicEntry; 16],
    CurrFieldOrderCnt: [i32; 2],
    FieldOrderCntList: [[i32; 2]; 16],
    pic_init_qs_minus26: i8,
    chroma_qp_index_offset: i8,
    second_chroma_qp_index_offset: i8,
    ContinuationFlag: u8,
    pic_init_qp_minus26: i8,
    num_ref_idx_l0_active_minus1: u8,
    num_ref_idx_l1_active_minus1: u8,
    Reserved8BitsA: u8,
    FrameNumList: [u16; 16],
    UsedForReferenceFlags: u32,
    NonExistingFrameFlags: u16,
    frame_num: u16,
    log2_max_frame_num_minus4: u8,
    pic_order_cnt_type: u8,
    log2_max_pic_order_cnt_lsb_minus4: u8,
    delta_pic_order_always_zero_flag: u8,
    direct_8x8_inference_flag: u8,
    entropy_coding_mode_flag: u8,
    pic_order_present_flag: u8,
    num_slice_groups_minus1: u8,
    slice_group_map_type: u8,
    deblocking_filter_control_present_flag: u8,
    redundant_pic_cnt_present_flag: u8,
    Reserved8BitsB: u8,
    slice_group_change_rate_minus1: u16,
    SliceGroupMap: [u8; 810],
}

// ‼️ dxva.h uses #pragma pack(1): DXVA_Slice_H264_Short is 10 bytes, NOT 12.
// With the default repr(C) (12-byte, u32-aligned) the 2nd array entry is
// misaligned and the driver silently decodes only the first slice.
#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
struct DXVA_Slice_H264_Short {
    BSNALunitDataLocation: u32,
    SliceBytesInBuffer: u32,
    wBadSliceChopping: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DXVA_Qmatrix_H264 {
    bScalingLists4x4: [[u8; 16]; 6],
    bScalingLists8x8: [[u8; 64]; 2],
}

const INVALID_ENTRY: u8 = 0xFF;

// ---------------------------------------------------------------------------
// owned SPS/PPS bits (cros-codecs Sps/Pps are not Clone)
// ---------------------------------------------------------------------------
#[derive(Clone)]
struct SpsBits {
    profile_idc: u8,
    pic_width_in_mbs_minus1: u16,
    pic_height_in_map_units_minus1: u16,
    frame_mbs_only_flag: bool,
    chroma_format_idc: u8,
    direct_8x8_inference_flag: bool,
    max_num_ref_frames: u8,
    bit_depth_luma_minus8: u8,
    bit_depth_chroma_minus8: u8,
    log2_max_frame_num_minus4: u8,
    pic_order_cnt_type: u8,
    log2_max_pic_order_cnt_lsb_minus4: u8,
    delta_pic_order_always_zero_flag: bool,
}
impl SpsBits {
    fn from(s: &Sps) -> Self {
        Self {
            profile_idc: s.profile_idc,
            pic_width_in_mbs_minus1: s.pic_width_in_mbs_minus1,
            pic_height_in_map_units_minus1: s.pic_height_in_map_units_minus1,
            frame_mbs_only_flag: s.frame_mbs_only_flag,
            chroma_format_idc: s.chroma_format_idc,
            direct_8x8_inference_flag: s.direct_8x8_inference_flag,
            max_num_ref_frames: s.max_num_ref_frames,
            bit_depth_luma_minus8: s.bit_depth_luma_minus8,
            bit_depth_chroma_minus8: s.bit_depth_chroma_minus8,
            log2_max_frame_num_minus4: s.log2_max_frame_num_minus4,
            pic_order_cnt_type: s.pic_order_cnt_type,
            log2_max_pic_order_cnt_lsb_minus4: s.log2_max_pic_order_cnt_lsb_minus4,
            delta_pic_order_always_zero_flag: s.delta_pic_order_always_zero_flag,
        }
    }
    fn width(&self) -> u32 {
        (self.pic_width_in_mbs_minus1 as u32 + 1) * 16
    }
    fn height(&self) -> u32 {
        (self.pic_height_in_map_units_minus1 as u32 + 1) * 16
    }
}

#[derive(Clone)]
struct PpsBits {
    entropy_coding_mode_flag: bool,
    bottom_field_pic_order_in_frame_present_flag: bool,
    weighted_pred_flag: bool,
    weighted_bipred_idc: u8,
    transform_8x8_mode_flag: bool,
    constrained_intra_pred_flag: bool,
    deblocking_filter_control_present_flag: bool,
    redundant_pic_cnt_present_flag: bool,
    num_slice_groups_minus1: u32,
    num_ref_idx_l0_default_active_minus1: u8,
    num_ref_idx_l1_default_active_minus1: u8,
    pic_init_qp_minus26: i8,
    pic_init_qs_minus26: i8,
    chroma_qp_index_offset: i8,
    second_chroma_qp_index_offset: i8,
}
impl PpsBits {
    fn from(p: &Pps) -> Self {
        Self {
            entropy_coding_mode_flag: p.entropy_coding_mode_flag,
            bottom_field_pic_order_in_frame_present_flag: p
                .bottom_field_pic_order_in_frame_present_flag,
            weighted_pred_flag: p.weighted_pred_flag,
            weighted_bipred_idc: p.weighted_bipred_idc,
            transform_8x8_mode_flag: p.transform_8x8_mode_flag,
            constrained_intra_pred_flag: p.constrained_intra_pred_flag,
            deblocking_filter_control_present_flag: p.deblocking_filter_control_present_flag,
            redundant_pic_cnt_present_flag: p.redundant_pic_cnt_present_flag,
            num_slice_groups_minus1: p.num_slice_groups_minus1,
            num_ref_idx_l0_default_active_minus1: p.num_ref_idx_l0_default_active_minus1,
            num_ref_idx_l1_default_active_minus1: p.num_ref_idx_l1_default_active_minus1,
            pic_init_qp_minus26: p.pic_init_qp_minus26,
            pic_init_qs_minus26: p.pic_init_qs_minus26,
            chroma_qp_index_offset: p.chroma_qp_index_offset,
            second_chroma_qp_index_offset: p.second_chroma_qp_index_offset,
        }
    }
}

// A reference picture held in the DPB.
#[derive(Clone, Copy)]
struct DpbRef {
    slice: u8,     // array-slice index in the surface pool
    frame_num: u16,
    top_poc: i32,
    bottom_poc: i32,
}

const POOL: u32 = 8;

struct D3d11Decoder {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    vcontext: ID3D11VideoContext,
    decoder: ID3D11VideoDecoder,
    views: Vec<ID3D11VideoDecoderOutputView>,
    pool: ID3D11Texture2D,
    staging: ID3D11Texture2D,
    free: Vec<u8>,
    dpb: Vec<DpbRef>,
    prev_poc_msb: i32,
    prev_poc_lsb: i32,
    prev_frame_num: i32,
    prev_frame_num_offset: i32,
    width: u32,
    height: u32,
    feedback: u32,
}

impl D3d11Decoder {
    unsafe fn new(width: u32, height: u32) -> anyhow::Result<Self> {
        let mut device = None;
        let mut context = None;
        let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
        let device: ID3D11Device = device.unwrap();
        let context: ID3D11DeviceContext = context.unwrap();
        let vdevice: ID3D11VideoDevice = device.cast()?;
        let vcontext: ID3D11VideoContext = context.cast()?;

        let desc = D3D11_VIDEO_DECODER_DESC {
            Guid: H264_VLD_NOFGT,
            SampleWidth: width,
            SampleHeight: height,
            OutputFormat: DXGI_FORMAT_NV12,
        };
        let cfg_count = vdevice.GetVideoDecoderConfigCount(&desc)?;
        let want_raw: u32 = std::env::var("WANDR_RAW").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
        let mut config: D3D11_VIDEO_DECODER_CONFIG = zeroed();
        let mut have = false;
        for i in 0..cfg_count {
            let mut c: D3D11_VIDEO_DECODER_CONFIG = zeroed();
            vdevice.GetVideoDecoderConfig(&desc, i, &mut c)?;
            if std::env::var("WANDR_CFG").is_ok() {
                println!("  [cfg {i}] BitstreamRaw={} ConfigMinRenderTargetBuffCount={} Residual={} SpatialResid={} IntraRefresh={} guidEnc={:?}",
                    c.ConfigBitstreamRaw, c.ConfigMinRenderTargetBuffCount, c.ConfigResidDiffAccelerator,
                    c.ConfigSpatialResid8, c.ConfigIntraResidUnsigned, c.guidConfigBitstreamEncryption);
            }
            if c.ConfigBitstreamRaw == want_raw && !have {
                config = c;
                have = true;
            }
        }
        anyhow::ensure!(have, "no decoder config with BitstreamRaw={want_raw}");
        if std::env::var("WANDR_CFG").is_ok() {
            println!("  -> chose config BitstreamRaw={}", config.ConfigBitstreamRaw);
        }
        let decoder = vdevice.CreateVideoDecoder(&desc, &config)?;

        // surface pool: one NV12 Texture2D array, one output view per slice
        let tdesc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: POOL,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_DECODER.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut pool = None;
        device.CreateTexture2D(&tdesc, None, Some(&mut pool))?;
        let pool: ID3D11Texture2D = pool.unwrap();
        let mut views = Vec::new();
        for s in 0..POOL {
            let ovdesc = D3D11_VIDEO_DECODER_OUTPUT_VIEW_DESC {
                DecodeProfile: H264_VLD_NOFGT,
                ViewDimension: D3D11_VDOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_DECODER_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VDOV { ArraySlice: s },
                },
            };
            let mut v = None;
            vdevice.CreateVideoDecoderOutputView(&pool, &ovdesc, Some(&mut v))?;
            views.push(v.unwrap());
        }

        let sdesc = D3D11_TEXTURE2D_DESC {
            ArraySize: 1,
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            ..tdesc
        };
        let mut staging = None;
        device.CreateTexture2D(&sdesc, None, Some(&mut staging))?;

        Ok(Self {
            device,
            context,
            vcontext,
            decoder,
            views,
            pool,
            staging: staging.unwrap(),
            free: (0..POOL as u8).rev().collect(),
            dpb: Vec::new(),
            prev_poc_msb: 0,
            prev_poc_lsb: 0,
            prev_frame_num: 0,
            prev_frame_num_offset: 0,
            width,
            height,
            feedback: 0,
        })
    }

    // Picture order count (frames only). Updates the prev-POC state. Handles
    // type 0 (8.2.1.1) and type 2 (8.2.1.3); type 1 is unsupported here.
    fn compute_poc(&mut self, sps: &SpsBits, hdr: &SliceHeader, is_idr: bool, is_ref: bool) -> (i32, i32) {
        let fnum = hdr.frame_num as i32;
        match sps.pic_order_cnt_type {
            0 => {
                let (prev_msb, prev_lsb) =
                    if is_idr { (0, 0) } else { (self.prev_poc_msb, self.prev_poc_lsb) };
                let max_lsb = 1i32 << (sps.log2_max_pic_order_cnt_lsb_minus4 + 4);
                let lsb = hdr.pic_order_cnt_lsb as i32;
                let msb = if lsb < prev_lsb && (prev_lsb - lsb) >= max_lsb / 2 {
                    prev_msb + max_lsb
                } else if lsb > prev_lsb && (lsb - prev_lsb) > max_lsb / 2 {
                    prev_msb - max_lsb
                } else {
                    prev_msb
                };
                if is_ref {
                    self.prev_poc_msb = msb;
                    self.prev_poc_lsb = lsb;
                }
                let top = msb + lsb;
                (top, top + hdr.delta_pic_order_cnt_bottom)
            }
            2 => {
                let max_fn = 1i32 << (sps.log2_max_frame_num_minus4 + 4);
                let offset = if is_idr {
                    0
                } else if self.prev_frame_num > fnum {
                    self.prev_frame_num_offset + max_fn
                } else {
                    self.prev_frame_num_offset
                };
                let temp = if is_idr {
                    0
                } else if !is_ref {
                    2 * (offset + fnum) - 1
                } else {
                    2 * (offset + fnum)
                };
                self.prev_frame_num = fnum;
                self.prev_frame_num_offset = offset;
                (temp, temp)
            }
            t => panic!("POC type {t} unsupported in this spike"),
        }
    }

    fn build_pic_params(
        &self,
        sps: &SpsBits,
        pps: &PpsBits,
        hdr: &SliceHeader,
        is_idr: bool,
        is_ref: bool,
        out_slice: u8,
        top_poc: i32,
        bottom_poc: i32,
        feedback: u32,
    ) -> DXVA_PicParams_H264 {
        let mut pp: DXVA_PicParams_H264 = unsafe { zeroed() };
        pp.wFrameWidthInMbsMinus1 = sps.pic_width_in_mbs_minus1;
        let interlaced = !sps.frame_mbs_only_flag as u16;
        pp.wFrameHeightInMbsMinus1 = ((sps.pic_height_in_map_units_minus1 + 1) << interlaced) - 1;
        pp.CurrPic = out_slice & 0x7F;
        pp.num_ref_frames = sps.max_num_ref_frames;

        let is_intra = matches!(hdr.slice_type, cros_codecs::codec::h264::parser::SliceType::I);
        let mut bf: u16 = 0;
        bf |= (hdr.field_pic_flag as u16) << 0;
        bf |= ((sps.chroma_format_idc as u16) & 0x3) << 4;
        bf |= (is_ref as u16) << 6; // RefPicFlag
        bf |= (pps.constrained_intra_pred_flag as u16) << 7;
        bf |= (pps.weighted_pred_flag as u16) << 8;
        bf |= ((pps.weighted_bipred_idc as u16) & 0x3) << 9;
        bf |= 1u16 << 11; // MbsConsecutiveFlag
        bf |= (sps.frame_mbs_only_flag as u16) << 12;
        bf |= (pps.transform_8x8_mode_flag as u16) << 13;
        bf |= (sps.direct_8x8_inference_flag as u16) << 14;
        bf |= (is_intra as u16) << 15; // IntraPicFlag
        pp.wBitFields = bf;

        pp.bit_depth_luma_minus8 = sps.bit_depth_luma_minus8;
        pp.bit_depth_chroma_minus8 = sps.bit_depth_chroma_minus8;
        pp.Reserved16Bits = 3;
        pp.StatusReportFeedbackNumber = feedback;

        pp.RefFrameList = [INVALID_ENTRY; 16];
        pp.CurrFieldOrderCnt = [top_poc, bottom_poc];
        let mut used: u32 = 0;
        for (k, r) in self.dpb.iter().enumerate().take(16) {
            pp.RefFrameList[k] = r.slice & 0x7F; // short-term frame ref
            pp.FieldOrderCntList[k] = [r.top_poc, r.bottom_poc];
            pp.FrameNumList[k] = r.frame_num;
            used |= 3u32 << (2 * k); // top+bottom fields used for reference
        }
        pp.UsedForReferenceFlags = used;
        let _ = is_idr;

        pp.pic_init_qs_minus26 = pps.pic_init_qs_minus26;
        pp.chroma_qp_index_offset = pps.chroma_qp_index_offset;
        pp.second_chroma_qp_index_offset = pps.second_chroma_qp_index_offset;
        pp.ContinuationFlag = 1;
        pp.pic_init_qp_minus26 = pps.pic_init_qp_minus26;
        pp.num_ref_idx_l0_active_minus1 = pps.num_ref_idx_l0_default_active_minus1;
        pp.num_ref_idx_l1_active_minus1 = pps.num_ref_idx_l1_default_active_minus1;
        pp.frame_num = hdr.frame_num;
        pp.log2_max_frame_num_minus4 = sps.log2_max_frame_num_minus4;
        pp.pic_order_cnt_type = sps.pic_order_cnt_type;
        pp.log2_max_pic_order_cnt_lsb_minus4 = sps.log2_max_pic_order_cnt_lsb_minus4;
        pp.delta_pic_order_always_zero_flag = sps.delta_pic_order_always_zero_flag as u8;
        pp.direct_8x8_inference_flag = sps.direct_8x8_inference_flag as u8;
        pp.entropy_coding_mode_flag = pps.entropy_coding_mode_flag as u8;
        pp.pic_order_present_flag = pps.bottom_field_pic_order_in_frame_present_flag as u8;
        pp.num_slice_groups_minus1 = pps.num_slice_groups_minus1 as u8;
        pp.deblocking_filter_control_present_flag = pps.deblocking_filter_control_present_flag as u8;
        pp.redundant_pic_cnt_present_flag = pps.redundant_pic_cnt_present_flag as u8;
        pp
    }

    unsafe fn decode_picture(
        &mut self,
        sps: &SpsBits,
        pps: &PpsBits,
        hdr: &SliceHeader,
        is_idr: bool,
        ref_idc: u8,
        slice_nals: &[Vec<u8>],
    ) -> anyhow::Result<(Vec<u8>, i32)> {
        if is_idr {
            self.dpb.clear();
            self.free = (0..POOL as u8).rev().collect();
            self.prev_poc_msb = 0;
            self.prev_poc_lsb = 0;
            self.prev_frame_num = 0;
            self.prev_frame_num_offset = 0;
        }
        let is_ref = ref_idc != 0;
        let out_slice = self.free.pop().expect("surface pool exhausted");
        let (top_poc, bottom_poc) = self.compute_poc(sps, hdr, is_idr, is_ref);
        self.feedback += 1;

        let pic = self.build_pic_params(sps, pps, hdr, is_idr, is_ref, out_slice, top_poc, bottom_poc, self.feedback);
        let qm = DXVA_Qmatrix_H264 {
            bScalingLists4x4: [[16u8; 16]; 6],
            bScalingLists8x8: [[16u8; 64]; 2],
        };

        // This driver decodes exactly ONE slice per SubmitDecoderBuffers (proven
        // by a slice-order-reversal test). So submit each slice of the picture in
        // its own SubmitDecoderBuffers call, inside one BeginFrame/EndFrame —
        // each mirroring the proven single-slice path (bitstream starts at 0).
        // Array approach (ffmpeg's): all slices in one bitstream + a slice-control
        // ARRAY, one SubmitDecoderBuffers.
        // ffmpeg's exact approach: 2-entry SliceControl array, and buffer descs in
        // the order PP, IQ, BITSTREAM, SLICE_CONTROL (bitstream BEFORE slice control).
        let n_mbs = (self.width / 16) * (self.height / 16);
        let mut bitstream: Vec<u8> = Vec::new();
        let mut slice_ctl: Vec<DXVA_Slice_H264_Short> = Vec::new();
        for nal in slice_nals {
            slice_ctl.push(DXVA_Slice_H264_Short {
                BSNALunitDataLocation: bitstream.len() as u32,
                SliceBytesInBuffer: nal.len() as u32,
                wBadSliceChopping: 0,
            });
            bitstream.extend_from_slice(nal);
        }
        let padding = (128 - (bitstream.len() & 127)) & 127;
        if let Some(last) = slice_ctl.last_mut() {
            // copy-modify-writeback (can't take a ref to a packed field)
            let mut e = *last;
            e.SliceBytesInBuffer += padding as u32;
            *last = e;
        }
        // size_of is now 10 (packed) — correct array stride for the driver.
        let sc_bytes = std::slice::from_raw_parts(
            slice_ctl.as_ptr() as *const u8,
            slice_ctl.len() * size_of::<DXVA_Slice_H264_Short>(),
        );
        let sc_size = sc_bytes.len();

        self.vcontext.DecoderBeginFrame(&self.decoder, &self.views[out_slice as usize], 0, None)?;
        self.put(D3D11_VIDEO_DECODER_BUFFER_PICTURE_PARAMETERS, as_bytes(&pic))?;
        self.put(D3D11_VIDEO_DECODER_BUFFER_INVERSE_QUANTIZATION_MATRIX, as_bytes(&qm))?;
        let bs_padded = self.put_bitstream(&bitstream)?;
        self.put(D3D11_VIDEO_DECODER_BUFFER_SLICE_CONTROL, &sc_bytes)?;
        // ‼️ Desc order matches ffmpeg: PP, IQ, BITSTREAM, SLICE_CONTROL.
        let descs = [
            buf_desc(D3D11_VIDEO_DECODER_BUFFER_PICTURE_PARAMETERS, size_of::<DXVA_PicParams_H264>(), 0),
            buf_desc(D3D11_VIDEO_DECODER_BUFFER_INVERSE_QUANTIZATION_MATRIX, size_of::<DXVA_Qmatrix_H264>(), 0),
            buf_desc(D3D11_VIDEO_DECODER_BUFFER_BITSTREAM, bs_padded, n_mbs),
            buf_desc(D3D11_VIDEO_DECODER_BUFFER_SLICE_CONTROL, sc_size, 0),
        ];
        self.vcontext.SubmitDecoderBuffers(&self.decoder, &descs)?;
        self.vcontext.DecoderEndFrame(&self.decoder)?;

        let nv12 = self.readback(out_slice)?;

        // DPB update (sliding window). prev-POC state already updated in compute_poc.
        if is_ref {
            if self.dpb.len() >= sps.max_num_ref_frames.max(1) as usize {
                // evict lowest frame_num (sliding window)
                let (idx, evicted) = self
                    .dpb
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, r)| r.frame_num)
                    .map(|(i, r)| (i, *r))
                    .unwrap();
                self.free.push(evicted.slice);
                self.dpb.remove(idx);
            }
            self.dpb.push(DpbRef { slice: out_slice, frame_num: hdr.frame_num, top_poc, bottom_poc });
        } else {
            self.free.push(out_slice); // non-ref: return immediately
        }
        Ok((nv12, top_poc))
    }

    unsafe fn put(&self, ty: D3D11_VIDEO_DECODER_BUFFER_TYPE, data: &[u8]) -> anyhow::Result<()> {
        let mut size: u32 = 0;
        let mut ptr = std::ptr::null_mut();
        self.vcontext.GetDecoderBuffer(&self.decoder, ty, &mut size, &mut ptr)?;
        anyhow::ensure!(size as usize >= data.len(), "buffer too small");
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, data.len());
        self.vcontext.ReleaseDecoderBuffer(&self.decoder, ty)?;
        Ok(())
    }

    unsafe fn put_bitstream(&self, nal: &[u8]) -> anyhow::Result<usize> {
        let padded = (nal.len() + 127) & !127;
        let mut size: u32 = 0;
        let mut ptr = std::ptr::null_mut();
        self.vcontext.GetDecoderBuffer(&self.decoder, D3D11_VIDEO_DECODER_BUFFER_BITSTREAM, &mut size, &mut ptr)?;
        anyhow::ensure!(size as usize >= padded, "bitstream buffer too small");
        std::ptr::copy_nonoverlapping(nal.as_ptr(), ptr as *mut u8, nal.len());
        if padded > nal.len() {
            std::ptr::write_bytes((ptr as *mut u8).add(nal.len()), 0, padded - nal.len());
        }
        self.vcontext.ReleaseDecoderBuffer(&self.decoder, D3D11_VIDEO_DECODER_BUFFER_BITSTREAM)?;
        Ok(padded)
    }

    // Copy one array slice to the staging texture and pack tightly to NV12.
    unsafe fn readback(&self, slice: u8) -> anyhow::Result<Vec<u8>> {
        self.context.CopySubresourceRegion(
            &self.staging, 0, 0, 0, 0, &self.pool, slice as u32, None,
        );
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        self.context.Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut m))?;
        let stride = m.RowPitch as usize;
        let base = m.pData as *const u8;
        let uv = base.add(stride * self.height as usize);
        let (w, h) = (self.width as usize, self.height as usize);
        let mut out = Vec::with_capacity(w * h * 3 / 2);
        for y in 0..h {
            out.extend_from_slice(std::slice::from_raw_parts(base.add(y * stride), w));
        }
        for y in 0..h / 2 {
            out.extend_from_slice(std::slice::from_raw_parts(uv.add(y * stride), w));
        }
        self.context.Unmap(&self.staging, 0);
        Ok(out)
    }
}

fn buf_desc(ty: D3D11_VIDEO_DECODER_BUFFER_TYPE, size: usize, n_mbs: u32) -> D3D11_VIDEO_DECODER_BUFFER_DESC {
    let mut d: D3D11_VIDEO_DECODER_BUFFER_DESC = unsafe { zeroed() };
    d.BufferType = ty;
    d.DataSize = size as u32;
    d.NumMBsInBuffer = n_mbs;
    d
}

fn as_bytes<T>(v: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts(v as *const T as *const u8, size_of::<T>()) }
}

// NV12 (tightly packed) -> 24-bit BMP, BT.601 limited, for eyeball diagnosis.
fn nv12_to_bmp(nv12: &[u8], w: usize, h: usize) -> Vec<u8> {
    let row = w * 3 + (4 - (w * 3) % 4) % 4;
    let img = row * h;
    let mut v = Vec::with_capacity(54 + img);
    v.extend_from_slice(b"BM");
    v.extend_from_slice(&((54 + img) as u32).to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&54u32.to_le_bytes());
    v.extend_from_slice(&40u32.to_le_bytes());
    v.extend_from_slice(&(w as i32).to_le_bytes());
    v.extend_from_slice(&(h as i32).to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&24u16.to_le_bytes());
    v.extend_from_slice(&[0u8; 24]);
    let uv = w * h;
    let pad = (4 - (w * 3) % 4) % 4;
    for y in (0..h).rev() {
        for x in 0..w {
            let yy = nv12[y * w + x] as i32;
            let cx = (x / 2) * 2;
            let u = nv12[uv + (y / 2) * w + cx] as i32;
            let vv = nv12[uv + (y / 2) * w + cx + 1] as i32;
            let (c, d, e) = (yy - 16, u - 128, vv - 128);
            let r = ((298 * c + 409 * e + 128) >> 8).clamp(0, 255) as u8;
            let g = ((298 * c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
            let b = ((298 * c + 516 * d + 128) >> 8).clamp(0, 255) as u8;
            v.extend_from_slice(&[b, g, r]);
        }
        v.extend(std::iter::repeat(0).take(pad));
    }
    v
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn main() -> anyhow::Result<()> {
    let which = std::env::args().nth(1).unwrap_or_else(|| "ip".into());
    let (clip, crcs): (&[u8], Vec<&str>) = match which.as_str() {
        "i" => (CLIP_I, CRC_I.split_whitespace().collect()),
        "ipbp" => (CLIP_IPBP, CRC_IPBP.split_whitespace().collect()),
        "25" => (CLIP_25, CRC_25.split_whitespace().collect()),
        _ => (CLIP_IP, CRC_IP.split_whitespace().collect()),
    };
    println!("clip '{which}' ({} bytes), {} reference CRC(s)\n", clip.len(), crcs.len());
    {
        let mut c2 = Cursor::new(clip);
        let mut p2 = Parser::default();
        while let Ok(n) = Nalu::next(&mut c2) {
            if matches!(n.header.type_, NaluType::Sps) {
                if let Ok(s) = p2.parse_sps(&n) {
                    println!("  [info] profile_idc={} chroma_idc={} scaling_matrix_present={}",
                        s.profile_idc, s.chroma_format_idc, s.seq_scaling_matrix_present_flag);
                }
                break;
            }
        }
    }

    let mut parser = Parser::default();
    let mut cursor = Cursor::new(clip);
    let mut sps: Option<SpsBits> = None;
    let mut pps: Option<PpsBits> = None;
    let mut dec: Option<D3d11Decoder> = None;
    let mut frame = 0usize;
    let mut all_ok = true;
    let mut got_all: Vec<(i32, i32, String)> = Vec::new(); // (gop, POC, crc) decode order
    let mut gop: i32 = -1;

    // Pending picture being accumulated (one picture = 1+ slices).
    struct Pending {
        hdr0: SliceHeader,
        is_idr: bool,
        ref_idc: u8,
        nals: Vec<Vec<u8>>,
    }
    let mut pending: Option<Pending> = None;

    // Flush a completed picture through the decoder + CRC check.
    macro_rules! flush {
        ($pending:expr) => {
            if let Some(pp) = $pending.take() {
                let s = sps.as_ref().unwrap();
                let p = pps.as_ref().unwrap();
                if dec.is_none() {
                    dec = Some(unsafe { D3d11Decoder::new(s.width(), s.height())? });
                }
                let d = dec.as_mut().unwrap();
                let (nv12, poc) = unsafe {
                    d.decode_picture(s, p, &pp.hdr0, pp.is_idr, pp.ref_idc, &pp.nals)?
                };
                let got = format!("{:08x}", crc32(&nv12));
                if pp.is_idr { gop += 1; } // each IDR starts a new GOP; POC resets to 0
                got_all.push((gop, poc, got.clone()));
                let want = crcs.get(frame).copied().unwrap_or("<none>");
                let ok = got == want;
                all_ok &= ok;
                if crcs.len() <= 8 || frame < 12 || !ok {
                    println!(
                        "  pic {frame}: type={:?} fn={} poc={poc} slices={} CRC={got} {}",
                        pp.hdr0.slice_type, pp.hdr0.frame_num, pp.nals.len(),
                        if ok { "✅dec" } else { "" }
                    );
                }
                frame += 1;
            }
        };
    }

    while let Ok(nalu) = Nalu::next(&mut cursor) {
        match nalu.header.type_ {
            NaluType::Sps => {
                flush!(pending);
                sps = Some(SpsBits::from(parser.parse_sps(&nalu).map_err(anyhow::Error::msg)?));
            }
            NaluType::Pps => {
                pps = Some(PpsBits::from(parser.parse_pps(&nalu).map_err(anyhow::Error::msg)?));
            }
            NaluType::SliceIdr | NaluType::Slice => {
                let is_idr = nalu.header.idr_pic_flag;
                let ref_idc = nalu.header.ref_idc;
                let nal = nalu.data.to_vec();
                let hdr = parser.parse_slice_header(nalu).map_err(anyhow::Error::msg)?.header;
                anyhow::ensure!(
                    sps.as_ref().unwrap().pic_order_cnt_type != 1,
                    "POC type 1 unsupported in this spike"
                );

                if hdr.first_mb_in_slice == 0 {
                    flush!(pending); // new picture starts
                    pending = Some(Pending { hdr0: hdr, is_idr, ref_idc, nals: vec![nal] });
                } else if let Some(pp) = pending.as_mut() {
                    pp.nals.push(nal); // another slice of the same picture
                }
            }
            _ => {}
        }
    }
    flush!(pending);

    println!();
    if all_ok && frame == crcs.len() {
        println!("PHASE-1 PASS — all {frame} frames BIT-EXACT in decode order. ✅");
        return Ok(());
    }

    println!("  [poc] {} GOP(s) (IDR-delimited)", gop + 1);
    // Display order = per-GOP POC order (POC resets at each IDR). Stable-sort by
    // (gop, poc); compare positionally against the ffmpeg reference (display order).
    let mut display = got_all.clone();
    display.sort_by_key(|(g, poc, _)| (*g, *poc));
    let pos_display = display.iter().zip(crcs.iter()).filter(|((_, _, g), w)| g.as_str() == **w).count();
    let pos_decode = got_all.iter().zip(crcs.iter()).filter(|((_, _, g), w)| g.as_str() == **w).count();
    println!(
        "decoded {} pics; POSITIONAL match: {}/{} in decode order, {}/{} in display order (GOP,POC).",
        got_all.len(), pos_decode, crcs.len(), pos_display, crcs.len()
    );
    if pos_display == crcs.len() {
        println!("PHASE-1 PASS — all {} frames BIT-EXACT in display order (multi-slice + B-frames). ✅", crcs.len());
        return Ok(());
    }
    anyhow::bail!("not all bit-exact (best {pos_display}/{} in display order)", crcs.len());
}
