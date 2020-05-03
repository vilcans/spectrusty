use core::iter::StepBy;
use core::ops::Range;

#[cfg(feature = "snapshot")]
use serde::{Serialize, Deserialize};

use crate::memory::ZxMemory;
use crate::clock::{VideoTs, Ts, MemoryContention, VideoTsData3};
use crate::chip::ula::{
    frame_cache::{pixel_address_coords, color_address_coords}
};
use crate::video::{
    Renderer, BorderSize, BorderColor, PixelBuffer, Palette,
    VideoFrame, Video, CellCoords, MAX_BORDER_SIZE
};
use super::{
    Ula128, Ula128MemContention, UlaMemoryContention,
    frame_cache::Ula128FrameProducer
};

#[derive(Clone, Copy, Default, Debug, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(Serialize, Deserialize))]
pub struct Ula128VidFrame;

impl VideoFrame for Ula128VidFrame {
    /// A range of horizontal T-states, 0 should be where the frame starts.
    const HTS_RANGE: Range<Ts> = -73..155;
    /// The first video scan line index of the top border.
    const VSL_BORDER_TOP: Ts = 15;
    /// A range of video scan line indexes for the pixel area.
    const VSL_PIXELS: Range<Ts> = 63..255;
    /// The last video scan line index of the bottom border.
    const VSL_BORDER_BOT: Ts = 303;
    /// A total number of video scan lines.
    const VSL_COUNT: Ts = 311;

    type HtsIter = StepBy<Range<Ts>>;

    fn border_whole_line_hts_iter(border_size: BorderSize) -> Self::HtsIter {
        let invborder = ((MAX_BORDER_SIZE - Self::border_size_pixels(border_size))/2) as Ts;
        (-22+invborder..154-invborder).step_by(4)
    }

    fn border_left_hts_iter(border_size: BorderSize) -> Self::HtsIter {
        let invborder = ((MAX_BORDER_SIZE - Self::border_size_pixels(border_size))/2) as Ts;
        (-22+invborder..2).step_by(4)
    }

    fn border_right_hts_iter(border_size: BorderSize) -> Self::HtsIter {
        let invborder = ((MAX_BORDER_SIZE - Self::border_size_pixels(border_size))/2) as Ts;
        (130..154-invborder).step_by(4)
    }

    #[inline]
    fn contention(hc: Ts) -> Ts {
        if hc >= -3 && hc < 123 {
            let ct = (hc + 3) & 7;
            if ct < 6 {
                return hc + 6 - ct;
            }
        }
        hc
    }

    #[inline(always)]
    fn floating_bus_offset(hc: Ts) -> Option<u16> {
        // println!("floating_bus_offset: {},{} {}", vc, hc, crate::clock::VFrameTsCounter::<Self>::vc_hc_to_tstates(vc, hc));
        match hc + 2 {
            c @ 0..=123 if c & 4 == 0 => Some(c as u16),
            _ => None
        }
    }

    #[inline(always)]
    fn snow_interference_coords(VideoTs { vc, hc }: VideoTs) -> Option<CellCoords> {
        let row = vc - Self::VSL_PIXELS.start;
        if row >= 0 && vc < Self::VSL_PIXELS.end {
            if hc >= 0 && hc <= 122 {
                return match hc & 7 {
                    0 => Some(0),
                    2 => Some(1),
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

impl<D, X> Video for Ula128<D, X> {
    type VideoFrame = Ula128VidFrame;

    #[inline]
    fn border_color(&self) -> BorderColor {
        self.ula.ula_border_color()
    }

    fn set_border_color(&mut self, border: BorderColor) {
        self.ula.ula_set_border_color(border)
    }

    fn render_video_frame<'a, B: PixelBuffer<'a>, P: Palette<Pixel=B::Pixel>>(
            &mut self,
            buffer: &'a mut [u8],
            pitch: usize,
            border_size: BorderSize
        )
    {
        self.create_renderer(border_size).render_pixels::<B, P, Self::VideoFrame>(buffer, pitch)
    }
}

impl<B, X> Ula128<B, X> {
    #[inline]
    pub(super) fn update_frame_cache(&mut self, addr: u16, ts: VideoTs) {
        let frame_cache = match addr {
            0x4000..=0x5AFF => &mut self.ula.frame_cache,
            0xC000..=0xDAFF => match self.page3_screen_shadow_bank() {
                Some(false) => &mut self.ula.frame_cache,
                Some(true)  => &mut self.shadow_frame_cache,
                None => return
            }
            _ => return
        };
        if addr & 0x1800 != 0x1800 {
            let coords = pixel_address_coords(addr);
            frame_cache.update_frame_pixels(&self.ula.memory, coords, addr, ts);
        }
        else {
            let coords = color_address_coords(addr);
            frame_cache.update_frame_colors(&self.ula.memory, coords, addr, ts);
        }
    }

    #[inline(always)]
    pub(super) fn update_snow_interference(&mut self, ts: VideoTs, ir: u16) {
        let is_contended = if self.is_page3_contended() {
            Ula128MemContention::is_contended_address(ir)
        } else {
            UlaMemoryContention::is_contended_address(ir)
        };
        if is_contended {
            if let Some(coords) = Ula128VidFrame::snow_interference_coords(ts) {
                let (screen, frame_cache) = if self.cur_screen_shadow {
                    (self.ula.memory.screen_ref(1).unwrap(), &mut self.shadow_frame_cache)
                }
                else {
                    (self.ula.memory.screen_ref(0).unwrap(), &mut self.ula.frame_cache)
                };
                frame_cache.apply_snow_interference(screen, coords, ir as u8);
            }
        }
    }

    fn create_renderer<'a>(
            &'a mut self,
            border_size: BorderSize
        ) -> Renderer<Ula128FrameProducer<'a, Ula128VidFrame>, std::vec::Drain<'a, VideoTsData3>>
    {
        let swap_screens = self.beg_screen_shadow;
        let border = self.ula.ula_border_color().into();
        let invert_flash = self.ula.frames.0 & 16 != 0;
        let (border_changes, memory, frame_cache0) = self.ula.ula_video_render_data_view();
        let frame_cache1 = &self.shadow_frame_cache;
        let screen0 = &memory.screen_ref(0).unwrap();
        let screen1 = &memory.screen_ref(1).unwrap();
        let frame_image_producer = Ula128FrameProducer::new(
            swap_screens,
            screen0,
            screen1,
            frame_cache0,
            frame_cache1,
            self.screen_changes.drain(..)
        );
        // print!("render: {} {:?}", screen_bank, screen.as_ptr());
        Renderer {
            frame_image_producer,
            border,
            border_size,
            border_changes: border_changes.drain(..),
            invert_flash
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contention() {
        let vts0 = VideoTs::new(0, 0);
        let tstates = [(14361, 14367),
                       (14362, 14367),
                       (14363, 14367),
                       (14364, 14367),
                       (14365, 14367),
                       (14366, 14367),
                       (14367, 14367),
                       (14368, 14368)];
        for offset in (0..16).map(|x| x * 8i32) {
            for (testing, target) in tstates.iter().copied() {
                let mut vts = Ula128VidFrame::vts_add_ts(vts0, testing + offset as u32);
                vts.hc = Ula128VidFrame::contention(vts.hc);
                assert_eq!(Ula128VidFrame::normalize_vts(vts),
                           Ula128VidFrame::tstates_to_vts(target + offset));
            }
        }
        let refts = tstates[0].0 as i32;
        for ts in (refts - 100..refts)
            .chain(refts + 128..refts+Ula128VidFrame::HTS_COUNT as i32) {
            let vts = Ula128VidFrame::tstates_to_vts(ts);
            assert_eq!(Ula128VidFrame::contention(vts.hc), vts.hc);
        }
    }
}
