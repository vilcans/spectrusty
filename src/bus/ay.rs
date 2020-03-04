//! `AY-3-8910` programmable sound generator.
use core::fmt::{self, Debug};
use core::ops::{Deref, DerefMut};
use core::marker::PhantomData;

use crate::clock::{VideoTs, FTs};
use crate::bus::{BusDevice, NullDevice, OptionalBusDevice, DynamicBusDevice, NamedBusDevice};
use crate::peripherals::ay::{Ay3_8910Io, AyPortDecode, AyIoPort, AyIoNullPort, Ay128kPortDecode, AyFullerBoxPortDecode};
use crate::chip::ula::{UlaTsCounter, Ula};
use crate::audio::ay::Ay3_891xAudio;
use crate::audio::{Blep, AmpLevels};
use crate::audio::sample::SampleDelta;
use crate::memory::ZxMemory;
use crate::video::VideoFrame;

/// Implement this empty trait for [BusDevice] so methods from [AyAudioVBusDevice]
/// will get auto implemented to pass method call to next devices.
pub trait PassByAyAudioBusDevice {}

/// A convenient [Ay3_891xBusDevice] type emulating a device with a `Melodik` port configuration.
pub type Ay3_891xMelodik<D=NullDevice<VideoTs>,
                         A=AyIoNullPort<VideoTs>,
                         B=AyIoNullPort<VideoTs>> = Ay3_891xBusDevice<
                                                        VideoTs,
                                                        Ay128kPortDecode,
                                                        A, B, D>;
/// A convenient [Ay3_891xBusDevice] type emulating a device with a `Fuller Box` port configuration.
pub type Ay3_891xFullerBox<D=NullDevice<VideoTs>,
                           A=AyIoNullPort<VideoTs>,
                           B=AyIoNullPort<VideoTs>> = Ay3_891xBusDevice<
                                                        VideoTs,
                                                        AyFullerBoxPortDecode,
                                                        A, B, D>;

impl<D> fmt::Display for Ay3_891xMelodik<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AY-3-8913 (Melodik)")
    }
}

impl<D> fmt::Display for Ay3_891xFullerBox<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AY-3-8913 (Fuller Box)")
    }
}
/// This trait is being used by [AyAudioFrame] implementations to render `AY-3-8910` audio with bus devices.
///
/// Allows for rendering audio frame using [AyAudioFrame] directly on the [ControlUnit] without the
/// need to "locate" the [Ay3_891xBusDevice] in the daisy chain.
///
/// This trait is implemented autmatically for all [BusDevice]s which implement [PassByAyAudioBusDevice].
///
/// [AyAudioFrame]: crate::audio::ay::AyAudioFrame
/// [ControlUnit]: crate::chip::ControlUnit
pub trait AyAudioVBusDevice {
    fn render_ay_audio_vts<L: AmpLevels<B::SampleDelta>,
                           V: VideoFrame,
                           B: Blep>(&mut self, blep: &mut B, end_ts: VideoTs, chans: [usize; 3]);
}

/// `AY-3-8910/8912/8913` programmable sound generator as a [BusDevice].
///
/// Envelops [Ay3_891xAudio] sound generator and [Ay3_8910Io] I/O ports peripherals.
///
/// Provides a helper method to produce sound generated by the last emulated frame.
#[derive(Clone, Default, Debug)]
pub struct Ay3_891xBusDevice<T, P,
                             A=AyIoNullPort<T>,
                             B=AyIoNullPort<T>,
                             D=NullDevice<T>>
{
    /// Provides a direct access to the sound generator.
    pub ay_sound: Ay3_891xAudio,
    /// Provides a direct access to the I/O ports.
    pub ay_io: Ay3_8910Io<T, A, B>,
        bus: D,
        _port_decode: PhantomData<P>
}

impl<D, N> AyAudioVBusDevice for D
    where D: BusDevice<Timestamp=VideoTs, NextDevice=N> + PassByAyAudioBusDevice,
          N: BusDevice<Timestamp=VideoTs> + AyAudioVBusDevice
{
    #[inline(always)]
    fn render_ay_audio_vts<S, V, E>(&mut self, blep: &mut E, end_ts: VideoTs, chans: [usize; 3])
        where S: AmpLevels<E::SampleDelta>,
              V: VideoFrame,
              E: Blep
    {
        self.next_device_mut().render_ay_audio_vts::<S, V, E>(blep, end_ts, chans)
    }
}

impl<D, N> AyAudioVBusDevice for OptionalBusDevice<D, N>
    where D: AyAudioVBusDevice,
          N: AyAudioVBusDevice
{
    /// # Note
    /// If a device is being attached to an optional device the call will be forwarded to
    /// both: an optional device and to the next bus device.
    #[inline(always)]
    fn render_ay_audio_vts<S, V, E>(&mut self, blep: &mut E, end_ts: VideoTs, chans: [usize; 3])
        where S: AmpLevels<E::SampleDelta>,
              V: VideoFrame,
              E: Blep
    {
        if let Some(ref mut device) = self.device {
            device.render_ay_audio_vts::<S, V, E>(blep, end_ts, chans)
        }
        self.next_device.render_ay_audio_vts::<S, V, E>(blep, end_ts, chans)
    }
}

impl AyAudioVBusDevice for dyn NamedBusDevice<VideoTs> {
    /// # Note
    /// Because we need to guess the concrete type of the dynamic `BusDevice` we can currently handle
    /// only the most common cases: [Ay3_891xMelodik] and [Ay3_891xFullerBox]. If you use a customized
    /// [Ay3_891xBusDevice] for a dynamic `BusDevice` you need to render audio directly on the device
    /// downcasted to your custom type.
    #[inline]
    fn render_ay_audio_vts<S, V, E>(&mut self, blep: &mut E, end_ts: VideoTs, chans: [usize; 3])
        where S: AmpLevels<E::SampleDelta>,
              V: VideoFrame,
              E: Blep
    {
        if let Some(ay_dev) = self.downcast_mut::<Ay3_891xMelodik>() {
            ay_dev.render_ay_audio::<S, V, E>(blep, end_ts, chans)
        }
        else if let Some(ay_dev) = self.downcast_mut::<Ay3_891xFullerBox>() {
            ay_dev.render_ay_audio::<S, V, E>(blep, end_ts, chans)
        }
    }
}

impl<D> AyAudioVBusDevice for DynamicBusDevice<D>
    where D: AyAudioVBusDevice + BusDevice<Timestamp=VideoTs>
{
    /// # Note
    /// This implementation forwards a call to any recognizable [Ay3_891xBusDevice] device in a
    /// dynamic daisy-chain as well as to an upstream device.
    #[inline]
    fn render_ay_audio_vts<S, V, E>(&mut self, blep: &mut E, end_ts: VideoTs, chans: [usize; 3])
        where S: AmpLevels<E::SampleDelta>,
              V: VideoFrame,
              E: Blep
    {
        for dev in self.iter_mut() {
            dev.render_ay_audio_vts::<S, V, E>(blep, end_ts, chans)
        }
        self.next_device_mut().render_ay_audio_vts::<S, V, E>(blep, end_ts, chans)
    }
}

impl<P, A, B, N> AyAudioVBusDevice for Ay3_891xBusDevice<VideoTs, P, A, B, N> {
    #[inline(always)]
    fn render_ay_audio_vts<S, V, E>(&mut self, blep: &mut E, end_ts: VideoTs, chans: [usize; 3])
        where S: AmpLevels<E::SampleDelta>,
              V: VideoFrame,
              E: Blep
    {
        self.render_ay_audio::<S, V, E>(blep, end_ts, chans)
    }
}

impl AyAudioVBusDevice for NullDevice<VideoTs> {
    #[inline(always)]
    fn render_ay_audio_vts<S, V, E>(&mut self, _blep: &mut E, _end_ts: VideoTs, _chans: [usize; 3])
        where S: AmpLevels<E::SampleDelta>,
              V: VideoFrame,
              E: Blep
    {}
}

impl<T, P, A, B, D> BusDevice for Ay3_891xBusDevice<T, P, A, B, D>
    where P: AyPortDecode,
          A: AyIoPort<Timestamp=T>,
          B: AyIoPort<Timestamp=T>,
          T: Debug + Copy,
          D: BusDevice<Timestamp=T>
{
    type Timestamp = T;
    type NextDevice = D;

    #[inline]
    fn next_device_mut(&mut self) -> &mut Self::NextDevice {
        &mut self.bus
    }

    #[inline]
    fn next_device_ref(&self) -> &Self::NextDevice {
        &self.bus
    }

    #[inline]
    fn reset(&mut self, timestamp: Self::Timestamp) {
        self.ay_sound.reset();
        self.ay_io.reset(timestamp);
        self.bus.reset(timestamp);
    }

    #[inline]
    fn read_io(&mut self, port: u16, timestamp: Self::Timestamp) -> Option<u8> {
        if P::is_data_read(port) {
            return Some(self.ay_io.data_port_read(port, timestamp))
        }
        self.bus.read_io(port, timestamp)
    }

    #[inline]
    fn write_io(&mut self, port: u16, data: u8, timestamp: Self::Timestamp) -> bool {
        if P::write_ay_io(&mut self.ay_io, port, data, timestamp) {
            return true    
        }
        self.bus.write_io(port, data, timestamp)
    }

    #[inline]
    fn next_frame(&mut self, timestamp: Self::Timestamp) {
        self.ay_io.next_frame(timestamp);
        self.bus.next_frame(timestamp)
    }
}

impl<P, A, B, D> Ay3_891xBusDevice<VideoTs, P, A, B, D> {
    /// Renders square-wave pulses via [Blep] interface.
    ///
    /// Provide [AmpLevels] that can handle `level` values from 0 to 15 (4-bits).
    /// `channels` - target [Blep] audio channels for `[A, B, C]` AY-3-8910 channels.
    pub fn render_ay_audio<S,V,E>(
            &mut self,
            blep: &mut E,
            end_ts: VideoTs,
            chans: [usize; 3]
        )
        where S: AmpLevels<E::SampleDelta>,
              V: VideoFrame,
              E: Blep
    {
        let end_ts = V::vts_to_tstates(end_ts);
        let changes = self.ay_io.recorder.drain_ay_reg_changes::<V>();
        self.ay_sound.render_audio::<S,_,_>(changes, blep, end_ts, V::FRAME_TSTATES_COUNT, chans)
    }

}

impl<P, A, B, D> Ay3_891xBusDevice<FTs, P, A, B, D> {
    /// Renders square-wave pulses via [Blep] interface.
    ///
    /// Provide [AmpLevels] that can handle `level` values from 0 to 15 (4-bits).
    /// `channels` - target [Blep] audio channels for `[A, B, C]` AY-3-8910 channels.
    pub fn render_ay_audio<S,E>(
            &mut self,
            blep: &mut E,
            end_ts: FTs,
            frame_tstates: FTs,
            chans: [usize; 3]
        )
        where S: AmpLevels<E::SampleDelta>,
              E: Blep
    {
        let changes = self.ay_io.recorder.drain_ay_reg_changes();
        self.ay_sound.render_audio::<S,_,_>(changes, blep, end_ts, frame_tstates, chans)
    }

}
