use core::iter::StepBy;
use core::ops::Range;

#[cfg(feature = "snapshot")]
use serde::{Serialize, Deserialize};

use crate::memory::ZxMemory;
use crate::clock::{VideoTs, Ts, VFrameTsCounter, VideoTsData3, MemoryContention};
use crate::video::{
    Renderer, BorderSize, BorderColor, PixelBuffer, Palette,
    VideoFrame, Video, CellCoords, MAX_BORDER_SIZE,
    frame_cache::{
        pixel_address_coords, color_address_coords
    }
};
use super::{Ula, UlaMemoryContention};
use super::frame_cache::{
    UlaFrameCache, UlaFrameProducer
};

/// Implements [VideoFrame] for PAL ULA.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[cfg_attr(feature = "snapshot", derive(Serialize, Deserialize))]
pub struct UlaVideoFrame;

impl VideoFrame for UlaVideoFrame {
    /// A range of horizontal T-states, 0 should be when the frame starts.
    const HTS_RANGE: Range<Ts> = -69..155;
    /// The first video scan line index of the top border.
    const VSL_BORDER_TOP: Ts = 16;
    /// A range of video scan line indexes for the pixel area.
    const VSL_PIXELS: Range<Ts> = 64..256;
    /// The last video scan line index of the bottom border.
    const VSL_BORDER_BOT: Ts = 304;
    /// A total number of video scan lines.
    const VSL_COUNT: Ts = 312;

    type BorderHtsIter = StepBy<Range<Ts>>;

    fn border_whole_line_hts_iter(border_size: BorderSize) -> Self::BorderHtsIter {
        let invborder = ((MAX_BORDER_SIZE - Self::border_size_pixels(border_size))/2) as Ts;
        (-20+invborder..156-invborder).step_by(4)
    }

    fn border_left_hts_iter(border_size: BorderSize) -> Self::BorderHtsIter {
        let invborder = ((MAX_BORDER_SIZE - Self::border_size_pixels(border_size))/2) as Ts;
        (-20+invborder..4).step_by(4)
    }

    fn border_right_hts_iter(border_size: BorderSize) -> Self::BorderHtsIter {
        let invborder = ((MAX_BORDER_SIZE - Self::border_size_pixels(border_size))/2) as Ts;
        (132..156-invborder).step_by(4)
    }

    #[inline]
    fn contention(hc: Ts) -> Ts {
        if hc >= -1 && hc < 125 {
            let ct = (hc + 1) & 7;
            if ct < 6 {
                return hc + 6 - ct;
            }
        }
        hc
    }

    #[inline(always)]
    fn floating_bus_offset(hc: Ts) -> Option<u16> {
        // println!("floating_bus_offset: {},{} {}", vc, hc, crate::clock::VFrameTsCounter::<Self>::vc_hc_to_tstates(vc, hc));
        match hc {
            c @ 0..=123 if c & 4 == 0 => Some(c as u16),
            _ => None
        }
    }

    #[inline(always)]
    fn snow_interference_coords(VideoTs { vc, hc }: VideoTs) -> Option<CellCoords> {
        let row = vc - Self::VSL_PIXELS.start;
        if row >= 0 && vc < Self::VSL_PIXELS.end {
            let hc = hc - 2;
            if hc >= 0 && hc <= 123 {
                return match hc & 7 {
                    0|1 => Some(0),
                    2|3 => Some(1),
                    _ => None
                }.map(|offs| {
                    let column = (((hc >> 2) & !1) | offs) as u8;
                    CellCoords { column, row: row as u8 }
                })
            }
        }
        None
    }
}

impl<M: ZxMemory, D, X, V: VideoFrame> Video for Ula<M, D, X, V> {
    type VideoFrame = V;
    type Contention = UlaMemoryContention;

    #[inline]
    fn border_color(&self) -> BorderColor {
        self.last_border
    }

    fn set_border_color(&mut self, border: BorderColor) {
        if self.last_border != border {
            self.border_out_changes.push((self.tsc, border.bits()).into());
            self.last_border = border;
        }
    }

    fn render_video_frame<'a, B: PixelBuffer<'a>, P: Palette<Pixel=B::Pixel>>(
            &mut self,
            buffer: &'a mut [u8],
            pitch: usize,
            border_size: BorderSize
        )
    {
        self.create_renderer(border_size).render_pixels::<B, P, V>(buffer, pitch)
    }

    fn current_video_ts(&self) -> VideoTs {
        self.tsc
    }

    fn current_video_clock(&self) -> VFrameTsCounter<V, UlaMemoryContention> {
        VFrameTsCounter::from_video_ts(self.tsc, UlaMemoryContention)
    }

    fn set_video_ts(&mut self, vts: VideoTs) {
        self.tsc = vts;
    }
}

impl<M: ZxMemory, B, X, V: VideoFrame> Ula<M, B, X, V> {
    #[inline]
    pub(super) fn update_frame_cache(&mut self, addr: u16, ts: VideoTs) {
        match addr {
            0x4000..=0x57FF => {
                let coords = pixel_address_coords(addr);
                self.frame_cache.update_frame_pixels(&self.memory, coords, addr, ts);
            }
            0x5800..=0x5AFF => {
                let coords = color_address_coords(addr);
                self.frame_cache.update_frame_colors(&self.memory, coords, addr, ts);
            }
            _ => {}
        }
    }

    #[inline(always)]
    pub(super) fn update_snow_interference(&mut self, ts: VideoTs, ir: u16) {
        if UlaMemoryContention.is_contended_address(ir) {
            if let Some(coords) = V::snow_interference_coords(ts) {
                let screen = self.memory.screen_ref(0).unwrap();
                self.frame_cache.apply_snow_interference(screen, coords, ir as u8)
            }
        }
    }
}

impl<M: ZxMemory, B, X, V> Ula<M, B, X, V> {
    #[inline]
    pub(super) fn cleanup_video_frame_data(&mut self) {
        self.border = self.last_border;
        self.border_out_changes.clear();
        self.frame_cache.clear();
    }

    #[inline]
    pub(crate) fn video_render_data_view(&mut self) -> (&mut Vec<VideoTsData3>, &M, &UlaFrameCache<V>) {
        (&mut self.border_out_changes, &self.memory, &self.frame_cache)
    }

    fn create_renderer(
            &mut self,
            border_size: BorderSize
        ) -> Renderer<UlaFrameProducer<'_, V>, std::vec::Drain<'_, VideoTsData3>>
    {
        let border = self.border.into();
        let screen = self.memory.screen_ref(0).unwrap();
        // print!("render: {} {:?}", screen_bank, screen.as_ptr());
        Renderer {
            frame_image_producer: UlaFrameProducer::new(screen, &self.frame_cache),
            border,
            border_size,
            border_changes: self.border_out_changes.drain(..),
            invert_flash: self.frames.0 & 16 != 0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestVideoFrame = UlaVideoFrame;

    #[test]
    fn test_contention() {
        let vts0 = VideoTs::new(0, 0);
        let tstates = [(14335, 14341),
                       (14336, 14341),
                       (14337, 14341),
                       (14338, 14341),
                       (14339, 14341),
                       (14340, 14341),
                       (14341, 14341),
                       (14342, 14342)];
        for offset in (0..16).map(|x| x * 8i32) {
            for (testing, target) in tstates.iter().copied() {
                let mut vts = TestVideoFrame::vts_add_ts(vts0, testing + offset as u32);
                vts.hc = TestVideoFrame::contention(vts.hc);
                assert_eq!(TestVideoFrame::normalize_vts(vts),
                           TestVideoFrame::tstates_to_vts(target + offset));
            }
        }
        let refts = tstates[0].0 as i32;
        for ts in (refts - 96..refts)
            .chain(refts + 128..refts+TestVideoFrame::HTS_COUNT as i32) {
            let vts = TestVideoFrame::tstates_to_vts(ts);
            assert_eq!(TestVideoFrame::contention(vts.hc), vts.hc);
        }
    }

    #[test]
    fn test_video_frame_vts_utils() {
        let items = [((  0, -69),   -69, ( 0, 69819), false, true , (  0, -69)),
                     ((  0,   0),     0, ( 1,     0), false, true , (  0,   0)),
                     ((  0,  -1),    -1, ( 0, 69887), false, true , (  0,  -1)),
                     (( -1,   0),  -224, ( 0, 69664), false, true , ( -1,   0)),
                     ((  1,   0),   224, ( 1,   224), false, true , (  1,   0)),
                     ((312,  -1), 69887, ( 1, 69887), true , true , (312,  -1)),
                     ((312,   0), 69888, ( 2,     0), true , true , (312,   0)),
                     ((  0, 224),   224, ( 1,   224), false, false, (  1,   0)),
                     ((624,-223),139553, ( 2, 69665), true,  false, (623,   1))];
        for ((vc, hc), fts, (nfr, nfts), eof, is_norm, (nvc, nhc)) in items.iter().copied() {
            let vts = VideoTs::new(vc, hc);
            let nvts = VideoTs::new(nvc, nhc);
            assert_eq!(TestVideoFrame::vc_hc_to_tstates(vc, hc), fts);
            assert_eq!(TestVideoFrame::vts_to_tstates(vts), fts);
            assert_eq!(TestVideoFrame::tstates_to_vts(fts), nvts);
            assert_eq!(TestVideoFrame::vts_to_norm_tstates(1, vts), (nfr, nfts));
            assert_eq!(TestVideoFrame::is_vts_eof(vts), eof);
            assert_eq!(TestVideoFrame::is_normalized_vts(vts), is_norm);
            assert_eq!(TestVideoFrame::normalize_vts(vts), nvts);
        }
        assert_eq!(TestVideoFrame::vts_max(), VideoTs::new(i16::max_value(), 154));
        assert_eq!(TestVideoFrame::vts_min(), VideoTs::new(i16::min_value(), -69));
        let items = [((  0,   0),     0, (  0,   0)),
                     ((  0,   0),     1, (  0,   1)),
                     (( -1, 154),     1, (  0, -69)),
                     ((  0,   0),   224, (  1,   0)),
                     (( -1,   1),   223, (  0,   0)),
                     ((  0,   0), 69888, (312,   0)),
                     ((  1,  -1), 69888, (313,  -1)),
                     ((  2, 224), 69888, (315,   0))];
        for ((vc0, hc0), delta, (vc1, hc1)) in items.iter().copied() {
            let vts0 = VideoTs::new(vc0, hc0);
            let vts1 = VideoTs::new(vc1, hc1);
            assert_eq!(TestVideoFrame::vts_add_ts(vts0, delta), vts1);
            assert_eq!(TestVideoFrame::vts_diff(vts0, vts1), delta as i32);
            assert_eq!(TestVideoFrame::vts_diff(vts1, vts0), -(delta as i32));
        }
        let items = [((   312,      0), (     0,      0)),
                     ((   312,    -69), (     0,    -69)),
                     ((   623,    154), (   311,    154)),
                     ((     0,    224), (  -312,    224)),
                     ((-32767, -32768), (-32768, -32768)),
                     ((-32768, -32768), (-32768, -32768))];
        for ((vc0, hc0), (vc1, hc1)) in items.iter().copied() {
            let vts0 = VideoTs::new(vc0, hc0);
            let vts1 = VideoTs::new(vc1, hc1);
            assert_eq!(TestVideoFrame::vts_saturating_sub_frame(vts0), vts1);
        }
        let items = [((     0,      0), (     0,      0), (     0,      0), (     0,      0)),
                     ((     1,      1), (     1,      1), (     0,      0), (     2,      2)),
                     ((     1,      1), (    -1,     -1), (     2,      2), (     0,      0)),
                     ((     1,    154), (     1,      1), (     0,    153), (     3,    -69)),
                     ((-32768,    -69), (     1,      1), (-32768,    -69), (-32767,    -68)),
                     ((-32768,    -69), (-32768,    -69), (     0,      0), (-32768,    -69)),
                     (( 32767,    154), (     1,      1), ( 32766,    153), ( 32767,    154)),
                     (( 32767,    154), ( 32767,    154), (     0,      0), ( 32767,    154))];
        for ((vc0, hc0), (vc1, hc1), (svc, shc), (avc, ahc)) in items.iter().copied() {
            let vts0 = VideoTs::new(vc0, hc0);
            let vts1 = VideoTs::new(vc1, hc1);
            let subvts = VideoTs::new(svc, shc);
            let addvts = VideoTs::new(avc, ahc);
            assert_eq!(TestVideoFrame::vts_saturating_sub_vts_normalized(vts0, vts1), subvts);
            assert_eq!(TestVideoFrame::vts_saturating_add_vts_normalized(vts0, vts1), addvts);
            assert_eq!(TestVideoFrame::vts_saturating_add_vts_normalized(vts1, vts0), addvts);
        }
    }
}
