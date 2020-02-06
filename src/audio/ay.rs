use super::*;
use crate::io::ay::{AyRegister, AyRegChange};

/* processor clock divisor */
pub const INTERNAL_CLOCK_DIVISOR: FTs = 16;
/* host clock ratio */
pub const HOST_CLOCK_RATIO: FTs = 2;

pub const AMPS: [f32;16] = [0.000000,0.007813,0.011049,0.015625,
                            0.022097,0.031250,0.044194,0.062500,
                            0.088388,0.125000,0.176777,0.250000,
                            0.353553,0.500000,0.707107,1.000000];

pub const AMPS_I32: [i32;16] = [0x00000000, 0x01000431, 0x016a0db9, 0x01ffffff,
                                0x02d41313, 0x03ffffff, 0x05a82627, 0x07ffffff,
                                0x0b504c4f, 0x0fffffff, 0x16a0a0ff, 0x1fffffff,
                                0x2d41397f, 0x3fffffff, 0x5a827b7f, 0x7fffffff];

pub const AMPS_I16: [i16;16] = [0x0000, 0x0100, 0x016a, 0x01ff,
                                0x02d4, 0x03ff, 0x05a8, 0x07ff,
                                0x0b50, 0x0fff, 0x16a0, 0x1fff,
                                0x2d40, 0x3fff, 0x5a81, 0x7fff];

pub struct LogAmpLevels16<T>(core::marker::PhantomData<T>);
impl<T: Copy + FromSample<f32>> AmpLevels<T> for LogAmpLevels16<T> {
    fn amp_level(level: u32) -> T {
        const A: f32 = 3.1623e-3;
        const B: f32 = 5.757;
        let y: f32 = match level & 0xF {
            0  => 0.0,
            15 => 1.0,
            l => {
                let x = l as f32 / 15.0;
                A * (B * x).exp()
            }
        };
        T::from_sample(y)
    }
}

pub struct AyAmpLevels<T>(core::marker::PhantomData<T>);
macro_rules! impl_ay_amp_levels {
    ($([$ty:ty, $amps:ident]),*) => { $(
        impl AmpLevels<$ty> for AyAmpLevels<$ty> {
            #[inline(always)]
            fn amp_level(level: u32) -> $ty {
                $amps[(level & 15) as usize]
            }
        }
    )* };
}
impl_ay_amp_levels!([f32, AMPS], [i32, AMPS_I32], [i16, AMPS_I16]);

pub trait AyAudioFrame<B: Blep> {
    fn render_ay_audio_frame<V: AmpLevels<B::SampleDelta>>(
        &mut self, blep: &mut B, time_rate: B::SampleTime, chans: [usize; 3]);
}

#[derive(Default, Clone, Debug)]
pub struct Ay3_8891xAudio {
    current_ts: FTs,
    last_levels: [u8; 3],
    amp_levels: [AmpLevel; 3],
    env_control: EnvelopeControl,
    noise_control: NoiseControl,
    tone_control: [ToneControl; 3],
    mixer: Mixer,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct AmpLevel(pub u8);

impl AmpLevel {
    #[inline]
    pub fn set(&mut self, level: u8) {
        self.0 = level & 0x1F;
    }
    #[inline]
    pub fn is_env_control(self) -> bool {
        self.0 & 0x10 != 0
    }
    #[inline]
    pub fn level(self) -> u8 {
        self.0 & 0x0F
    }
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Mixer(pub u8);

impl Mixer {
    #[inline]
    pub fn has_tone(self) -> bool {
        self.0 & 1 == 0
    }
    #[inline]
    pub fn has_noise(self) -> bool {
        self.0 & 0x08 == 0
    }
    #[inline]
    pub fn next_chan(&mut self) {
        self.0 >>= 1
    }
}

pub const ENV_SHAPE_CONT_MASK:   u8 = 0b00001000;
pub const ENV_SHAPE_ATTACK_MASK: u8 = 0b00000100;
pub const ENV_SHAPE_ALT_MASK:    u8 = 0b00000010;
pub const ENV_SHAPE_HOLD_MASK:   u8 = 0b00000001;
const ENV_LEVEL_REV_MASK:    u8 = 0b10000000;
const ENV_LEVEL_MOD_MASK:    u8 = 0b01000000;
const ENV_LEVEL_MASK:        u8 = 0x0F;
const ENV_CYCLE_MASK:        u8 = 0xF0;

#[derive(Clone, Copy, Debug)]
pub struct EnvelopeControl {
    period: u16,
    tick: u16,
    // c c c c CT AT AL HO
    cycle: u8,
    // RV MD 0 0 v v v v
    level: u8
}

impl Default for EnvelopeControl {
    fn default() -> Self {
        EnvelopeControl { period: 1, tick: 0, cycle: 0, level: 0 }
    }
}

impl EnvelopeControl {
    #[inline]
    pub fn set_shape(&mut self, shape: u8) {
        self.tick = 0;
        self.cycle = shape & !ENV_CYCLE_MASK;
        self.level = if shape & ENV_SHAPE_ATTACK_MASK != 0 {
            ENV_LEVEL_MOD_MASK
        }
        else {
            ENV_LEVEL_MOD_MASK|ENV_LEVEL_REV_MASK|ENV_LEVEL_MASK
        }
    }
    #[inline]
    pub fn set_period_fine(&mut self, perlo: u8) {
        self.set_period(self.period & 0xFF00 | perlo as u16)
    }

    #[inline]
    pub fn set_period_coarse(&mut self, perhi: u8) {
        self.set_period(u16::from_le_bytes([self.period as u8, perhi]))
    }
    #[inline]
    pub fn set_period(&mut self, mut period: u16) {
        if period == 0 { period = 1 }
        self.period = period;
        if self.tick >= period {
            self.tick %= period;
        }
    }
    #[inline]
    pub fn update_level(&mut self) -> u8 {
        let EnvelopeControl { period, mut tick, mut level, .. } = *self;
        if tick >= period {
            tick -= period;

            if level & ENV_LEVEL_MOD_MASK != 0 {
                level = (level & !ENV_LEVEL_MASK) | (
                    if level & ENV_LEVEL_REV_MASK == 0 {
                        level.wrapping_add(1)
                    }
                    else {
                        level.wrapping_sub(1)
                    }
                & ENV_LEVEL_MASK);

                let cycle = self.cycle.wrapping_add(0x10); // 16 times
                if cycle & ENV_CYCLE_MASK == 0 {
                    if cycle & ENV_SHAPE_CONT_MASK == 0 {
                        level = 0;
                    }
                    else {
                        if cycle & ENV_SHAPE_HOLD_MASK != 0 {
                            if cycle & ENV_SHAPE_ALT_MASK == 0 {
                                level ^= ENV_LEVEL_MOD_MASK|ENV_LEVEL_MASK;
                            }
                            else {
                                level ^= ENV_LEVEL_MOD_MASK;
                            }
                        } else {
                            if cycle & ENV_SHAPE_ALT_MASK != 0 {
                                level ^= ENV_LEVEL_REV_MASK|ENV_LEVEL_MASK;
                            }
                        }
                    }
                }
                self.level = level;
                self.cycle = cycle;
            }
        }
        self.tick = tick.wrapping_add(1);
        level & ENV_LEVEL_MASK
    }
}

const NOISE_PERIOD_MASK: u8 = 0x1F;

#[derive(Clone, Copy, Debug)]
pub struct NoiseControl {
    rng: i32,
    period: u8,
    tick: u8,
    low: bool,
}

impl Default for NoiseControl {
    fn default() -> Self {
        NoiseControl { rng: 1, period: 0, tick: 0, low: false }
    }
}

impl NoiseControl {
    #[inline]
    pub fn set_period(&mut self, mut period: u8) {
        period &= NOISE_PERIOD_MASK;
        if period == 0 { period = 1 }
        self.period = period;
        if self.tick >= period {
            self.tick %= period;
        }
    }

    #[inline]
    pub fn update_is_low(&mut self) -> bool {
        let NoiseControl { mut rng, period, mut tick, mut low } = *self;
        if tick >= period {
            tick -= period;

            if (rng + 1) & 2 != 0 {
                low = !low;
                self.low = low;
            }
            rng = (-(rng & 1) & 0x12000) ^ (rng >> 1);
            self.rng = rng;
        }
        self.tick = tick.wrapping_add(1);
        low
    }
}

const TONE_GEN_MIN_THRESHOLD: u16 = 5;
const TONE_PERIOD_MASK: u16 = 0xFFF;

#[derive(Default, Clone, Copy, Debug)]
pub struct ToneControl {
    period: u16,
    tick: u16,
    low: bool
}


impl ToneControl {
    #[inline]
    pub fn set_period_fine(&mut self, perlo: u8) {
        self.set_period(self.period & 0xFF00 | perlo as u16)
    }

    #[inline]
    pub fn set_period_coarse(&mut self, perhi: u8) {
        self.set_period(u16::from_le_bytes([self.period as u8, perhi]))
    }

    #[inline]
    pub fn set_period(&mut self, mut period: u16) {
        period &= TONE_PERIOD_MASK;
        if period == 0 { period = 1 }
        self.period = period;
        if self.tick >= period*2 {
            self.tick %= period*2;
        }
    }

    #[inline]
    pub fn update_is_low(&mut self) -> bool {
        let ToneControl { period, mut tick, mut low } = *self;
        if period < TONE_GEN_MIN_THRESHOLD {
            low = false;
        }
        else if tick >= period {
            tick -= period;
            low = !low;
            self.low = low;
        }
        self.tick = tick.wrapping_add(2);
        low
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Ticker {
    pub current: FTs,
    pub end_ts: FTs
}

impl Ticker {
    pub const CLOCK_INCREASE: FTs = HOST_CLOCK_RATIO * INTERNAL_CLOCK_DIVISOR;
    pub fn new(current: FTs, end_ts: FTs) -> Self {
        Ticker { current, end_ts }
    }
    pub fn into_next_frame_ts(self) -> FTs {
        self.current - self.end_ts
    }
}

impl Iterator for Ticker {
    type Item = FTs;
    fn next(&mut self) -> Option<FTs> {
        let res = self.current;
        if res < self.end_ts {
            self.current = res + Self::CLOCK_INCREASE;
            Some(res)
        }
        else {
            None
        }
    }
}

impl Ay3_8891xAudio {
    pub fn reset(&mut self) {
        *self = Default::default()
    }
    /// Converts a frequency given in Hz to AY-3-891x tone period value.
    ///
    /// `clock_hz` AY-3-891x clock frequency in Hz. Usually it's CPU_HZ / 2.
    pub fn freq_to_tone_period(clock_hz: f32, hz: f32) -> u16 {
        let ftp = (clock_hz / (16.0 * hz)).round();
        let utp = ftp as u16;
        if utp as f32 != ftp {
            panic!("tone period out of 16-bit unsigned integer range: {}", ftp);
        }
        utp
    }
    /// Creates an iterator of tone periods for the AY-3-891x chip.
    ///
    /// `min_octave` A minimal octave number, 0-based (0 is the minimum).
    /// `max_octave` A maximal octave number, 0-based (7 is the maximum).
    /// `note_freqs` An array of tone frequencies (in Hz) in the 5th octave (0-based: 4).
    ///  To generate frequencies you may want to use audio::music::equal_tempered_scale_note_freqs.
    /// `clock_hz` The AY-3-891x clock frequency in Hz. Usually it's CPU_HZ / 2.
    pub fn tone_periods<I>(clock_hz: f32, min_octave: i32, max_octave: i32, note_freqs: I) -> impl IntoIterator<Item=u16>
    where I: Clone + IntoIterator<Item=f32>
    {
        (min_octave..=max_octave).flat_map(move |octave| {
          note_freqs.clone().into_iter().map(move |hz| {
            let hz = hz * (2.0f32).powi(octave - 4);
            let tp = Self::freq_to_tone_period(clock_hz, hz);
            if !(1..=4095).contains(&tp) {
                panic!("tone period out of range: {} ({} Hz)", tp, hz)
            }
            tp
          })
        })
    }

    /// Render BLEP deltas mutating the internal state. This can be done only once per frame.
    pub fn render_audio<V,L,I,A,FT>(&mut self, changes: I, blep: &mut A, time_rate: FT, end_ts: FTs,
                                    chans: [usize; 3])
    where V: AmpLevels<L>,
          L: SampleDelta + Default,
          I: IntoIterator<Item=AyRegChange>,
          FT: SampleTime,
          A: Blep<SampleDelta=L, SampleTime=FT>
    {
        let mut change_iter = changes.into_iter().peekable();
        let mut ticker = Ticker::new(self.current_ts, end_ts);
        let mut tone_levels: [u8; 3] = self.last_levels;
        let mut vol_levels: [L;3] = Default::default();
        // println!("current {:?}", ticker);
        for (level, tgt_amp) in tone_levels.iter().copied()
                                .zip(vol_levels.iter_mut()) {
            *tgt_amp = V::amp_level(level.into());
        }
        for tick in &mut ticker {
            while let Some(change) = change_iter.peek() {
                if change.time <= tick {
                    let AyRegChange { reg, val, .. } = change_iter.next().unwrap();
                    self.update_register(reg, val);
                }
                else {
                    break
                }
            }


            let env_level = self.env_control.update_level();
            let noise_low = self.noise_control.update_is_low();
            let mut mixer = self.mixer;
            for ((level, tone_control), tgt_lvl) in self.amp_levels.iter().copied()
                                                  .zip(self.tone_control.iter_mut())
                                                        .zip(tone_levels.iter_mut()) {
                *tgt_lvl = if (mixer.has_tone() && tone_control.update_is_low()) ||
                   (mixer.has_noise() && noise_low) {
                    0
                }
                else if level.is_env_control() {
                    env_level
                }
                else {
                    level.0
                };
                mixer.next_chan();
            }

            for (chan, (level, last_vol)) in chans.iter().copied()
                                                  .zip(tone_levels.iter().copied()
                                                  .zip(vol_levels.iter_mut())) {
                let vol = V::amp_level(level.into());
                if let Some(delta) = last_vol.sample_delta(vol) {
                    let time = time_rate.at_timestamp(tick);
                    blep.add_step(chan, time, delta);
                    *last_vol = vol;
                }
            }

        }
        while let Some(AyRegChange { reg, val, .. }) = change_iter.next() {
            self.update_register(reg, val);
        }
        self.current_ts = ticker.into_next_frame_ts();
        // println!("tone_levels {:?}", tone_levels);
        self.last_levels = tone_levels;
    }

    #[inline]
    fn update_register(&mut self, reg: AyRegister, val: u8) {
        // println!("update: {:?} {}", reg, val);
        use AyRegister::*;
        match reg {
            ToneFineA|
            ToneFineB|
            ToneFineC => self.tone_control[usize::from(reg) >> 1].set_period_fine(val),
            ToneCoarseA|
            ToneCoarseB|
            ToneCoarseC => self.tone_control[usize::from(reg) >> 1].set_period_coarse(val),
            NoisePeriod => self.noise_control.set_period(val),
            MixerControl => self.mixer = Mixer(val),
            AmpLevelA|
            AmpLevelB|
            AmpLevelC => self.amp_levels[usize::from(reg) - 8].set(val),
            EnvPerFine => self.env_control.set_period_fine(val),
            EnvPerCoarse => self.env_control.set_period_coarse(val),
            EnvShape => self.env_control.set_shape(val),
            _ => ()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // use std::error;

    #[test]
    fn ay_3_889x_tone_periods() {
        use super::music::*;
        let clock_hz = 3_546_900.0/2.0f32;
        let mut notes: Vec<u16> = Vec::new();
        assert_eq!(252, Ay3_8891xAudio::freq_to_tone_period(clock_hz, 440.0));
        assert_eq!(5, Ay3_8891xAudio::freq_to_tone_period(clock_hz, 24000.0));
        notes.extend(Ay3_8891xAudio::tone_periods(clock_hz, 0, 7, equal_tempered_scale_note_freqs(440.0, 0, 12)));
        assert_eq!(
            vec![4031, 3804, 3591, 3389, 3199, 3020, 2850, 2690, 2539, 2397, 2262, 2135,
                 2015, 1902, 1795, 1695, 1600, 1510, 1425, 1345, 1270, 1198, 1131, 1068,
                 1008,  951,  898,  847,  800,  755,  713,  673,  635,  599,  566,  534,
                  504,  476,  449,  424,  400,  377,  356,  336,  317,  300,  283,  267,
                  252,  238,  224,  212,  200,  189,  178,  168,  159,  150,  141,  133,
                  126,  119,  112,  106,  100,   94,   89,   84,   79,   75,   71,   67,
                   63,   59,   56,   53,   50,   47,   45,   42,   40,   37,   35,   33,
                   31,   30,   28,   26,   25,   24,   22,   21,   20,   19,   18,   17], notes);
    }

    #[test]
    fn ay_3_889x_env_works() {
        // println!("Ay3_8891xAudio {:?}", core::mem::size_of::<Ay3_8891xAudio>());
        let mut ay = Ay3_8891xAudio::default();

        for shape in [0, ENV_SHAPE_ALT_MASK,
                         ENV_SHAPE_HOLD_MASK,
                         ENV_SHAPE_ALT_MASK|ENV_SHAPE_HOLD_MASK,
                         ENV_SHAPE_CONT_MASK|ENV_SHAPE_HOLD_MASK].iter().copied() {
            ay.env_control.set_shape(shape);
            assert_eq!(ay.env_control.tick, 0);
            assert_eq!(ay.env_control.cycle, shape);
            assert_eq!(ay.env_control.level, ENV_LEVEL_REV_MASK|ENV_LEVEL_MOD_MASK|ENV_LEVEL_MASK);
            ay.env_control.set_period(0);
            assert_eq!(ay.env_control.period, 1);
            for exp_level in (0..=15).rev() {
                assert_eq!(ay.env_control.update_level(), exp_level);
                assert_eq!(ay.env_control.tick, 1);
            }
            for _ in 0..100 {
                assert_eq!(ay.env_control.tick, 1);
                assert_eq!(ay.env_control.update_level(), 0);
            }
        }

        for shape in [0, ENV_SHAPE_ALT_MASK,
                         ENV_SHAPE_HOLD_MASK,
                         ENV_SHAPE_ALT_MASK|ENV_SHAPE_HOLD_MASK,
                         ENV_SHAPE_CONT_MASK|ENV_SHAPE_ATTACK_MASK|ENV_SHAPE_ALT_MASK|ENV_SHAPE_HOLD_MASK
                         ].iter().copied() {
            ay.env_control.set_shape(shape|ENV_SHAPE_ATTACK_MASK);
            assert_eq!(ay.env_control.tick, 0);
            assert_eq!(ay.env_control.cycle, shape|ENV_SHAPE_ATTACK_MASK);
            assert_eq!(ay.env_control.level, ENV_LEVEL_MOD_MASK);
            ay.env_control.set_period(0);
            assert_eq!(ay.env_control.period, 1);
            for exp_level in 0..=15 {
                assert_eq!(ay.env_control.update_level(), exp_level);
                assert_eq!(ay.env_control.level, ENV_LEVEL_MOD_MASK|exp_level);
                assert_eq!(ay.env_control.tick, 1);
            }
            for _ in 0..100 {
                assert_eq!(ay.env_control.tick, 1);
                assert_eq!(ay.env_control.update_level(), 0);
                assert_eq!(ay.env_control.level, 0);
            }
        }

        ay.env_control.set_shape(ENV_SHAPE_CONT_MASK);
        assert_eq!(ay.env_control.tick, 0);
        assert_eq!(ay.env_control.cycle, ENV_SHAPE_CONT_MASK);
        assert_eq!(ay.env_control.level, ENV_LEVEL_REV_MASK|ENV_LEVEL_MOD_MASK|ENV_LEVEL_MASK);
        ay.env_control.set_period(0);
        assert_eq!(ay.env_control.period, 1);
        for _ in 0..10 {
            for exp_level in (0..=15).rev() {
                assert_eq!(ay.env_control.update_level(), exp_level);
                assert_eq!(ay.env_control.level, ENV_LEVEL_REV_MASK|ENV_LEVEL_MOD_MASK|exp_level);
                assert_eq!(ay.env_control.tick, 1);
            }
        }

        ay.env_control.set_shape(ENV_SHAPE_CONT_MASK|ENV_SHAPE_ALT_MASK);
        assert_eq!(ay.env_control.tick, 0);
        assert_eq!(ay.env_control.cycle, ENV_SHAPE_CONT_MASK|ENV_SHAPE_ALT_MASK);
        assert_eq!(ay.env_control.level, ENV_LEVEL_REV_MASK|ENV_LEVEL_MOD_MASK|ENV_LEVEL_MASK);
        ay.env_control.set_period(0);
        assert_eq!(ay.env_control.period, 1);
        for _ in 0..10 {
            for exp_level in (0..=15).rev() {
                assert_eq!(ay.env_control.update_level(), exp_level);
                assert_eq!(ay.env_control.level, ENV_LEVEL_REV_MASK|ENV_LEVEL_MOD_MASK|exp_level);
                assert_eq!(ay.env_control.tick, 1);
            }
            for exp_level in 0..=15 {
                assert_eq!(ay.env_control.update_level(), exp_level);
                assert_eq!(ay.env_control.level, ENV_LEVEL_MOD_MASK|exp_level);
                assert_eq!(ay.env_control.tick, 1);
            }
        }

        ay.env_control.set_shape(ENV_SHAPE_CONT_MASK|ENV_SHAPE_ALT_MASK|ENV_SHAPE_HOLD_MASK);
        assert_eq!(ay.env_control.tick, 0);
        assert_eq!(ay.env_control.cycle, ENV_SHAPE_CONT_MASK|ENV_SHAPE_ALT_MASK|ENV_SHAPE_HOLD_MASK);
        assert_eq!(ay.env_control.level, ENV_LEVEL_REV_MASK|ENV_LEVEL_MOD_MASK|ENV_LEVEL_MASK);
        ay.env_control.set_period(0);
        assert_eq!(ay.env_control.period, 1);
        for exp_level in (0..=15).rev() {
            assert_eq!(ay.env_control.update_level(), exp_level);
            assert_eq!(ay.env_control.level, ENV_LEVEL_REV_MASK|ENV_LEVEL_MOD_MASK|exp_level);
            assert_eq!(ay.env_control.tick, 1);
        }
        for _ in 0..100 {
            assert_eq!(ay.env_control.tick, 1);
            assert_eq!(ay.env_control.update_level(), 15);
            assert_eq!(ay.env_control.level, ENV_LEVEL_REV_MASK|15);
        }

        ay.env_control.set_shape(ENV_SHAPE_CONT_MASK|ENV_SHAPE_ATTACK_MASK);
        assert_eq!(ay.env_control.tick, 0);
        assert_eq!(ay.env_control.cycle, ENV_SHAPE_CONT_MASK|ENV_SHAPE_ATTACK_MASK);
        assert_eq!(ay.env_control.level, ENV_LEVEL_MOD_MASK);
        ay.env_control.set_period(0);
        assert_eq!(ay.env_control.period, 1);
        for _ in 0..10 {
            for exp_level in 0..=15 {
                assert_eq!(ay.env_control.update_level(), exp_level);
                assert_eq!(ay.env_control.level, ENV_LEVEL_MOD_MASK|exp_level);
                assert_eq!(ay.env_control.tick, 1);
            }
        }

        ay.env_control.set_shape(ENV_SHAPE_CONT_MASK|ENV_SHAPE_ATTACK_MASK|ENV_SHAPE_HOLD_MASK);
        assert_eq!(ay.env_control.tick, 0);
        assert_eq!(ay.env_control.cycle, ENV_SHAPE_CONT_MASK|ENV_SHAPE_ATTACK_MASK|ENV_SHAPE_HOLD_MASK);
        assert_eq!(ay.env_control.level, ENV_LEVEL_MOD_MASK);
        ay.env_control.set_period(0);
        assert_eq!(ay.env_control.period, 1);
        for exp_level in 0..=15 {
            assert_eq!(ay.env_control.update_level(), exp_level);
            assert_eq!(ay.env_control.level, ENV_LEVEL_MOD_MASK|exp_level);
            assert_eq!(ay.env_control.tick, 1);
        }
        for _ in 0..100 {
            assert_eq!(ay.env_control.tick, 1);
            assert_eq!(ay.env_control.update_level(), 15);
            assert_eq!(ay.env_control.level, 15);
        }

        ay.env_control.set_shape(ENV_SHAPE_CONT_MASK|ENV_SHAPE_ATTACK_MASK|ENV_SHAPE_ALT_MASK);
        assert_eq!(ay.env_control.tick, 0);
        assert_eq!(ay.env_control.cycle, ENV_SHAPE_CONT_MASK|ENV_SHAPE_ATTACK_MASK|ENV_SHAPE_ALT_MASK);
        assert_eq!(ay.env_control.level, ENV_LEVEL_MOD_MASK);
        ay.env_control.set_period(0);
        assert_eq!(ay.env_control.period, 1);
        for _ in 0..10 {
            for exp_level in 0..=15 {
                assert_eq!(ay.env_control.update_level(), exp_level);
                assert_eq!(ay.env_control.level, ENV_LEVEL_MOD_MASK|exp_level);
                assert_eq!(ay.env_control.tick, 1);
            }
            for exp_level in (0..=15).rev() {
                assert_eq!(ay.env_control.update_level(), exp_level);
                assert_eq!(ay.env_control.level, ENV_LEVEL_REV_MASK|ENV_LEVEL_MOD_MASK|exp_level);
                assert_eq!(ay.env_control.tick, 1);
            }
        }
    }
}
