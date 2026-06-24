//! Sound — the voyage's audio bed and its event cues.
//!
//! Three ambient loops play continuously and are cross-faded by volume each
//! frame: the **sailing** bed rides up with the boat's speed (silent at anchor),
//! the **calm** sea bed is the base ambience (ducked as a gale rises), and the
//! **storm** bed swells with the gale's fury. Over the top sit one-shot cues:
//! a canvas *flap* when sail is raised or lowered, a transition *whoosh* when the
//! wind shifts quarter, and a coin *chime* on a successful trade, repair or
//! upgrade.
//!
//! macroquad's bundled mixer (quad-snd → audrey) only decodes WAV/OGG, but the
//! shipped clips are MP3, so each is decoded to PCM with `symphonia` at load and
//! handed to the mixer wrapped in an in-memory WAV (see [`decode_mp3_to_wav`]).

use macroquad::audio::{
    load_sound_from_bytes, play_sound, set_sound_volume, stop_sound, PlaySoundParams, Sound,
};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

// The clips, baked into the binary so there's nothing to ship alongside it.
const SAILING_MP3: &[u8] = include_bytes!("../assets/sounds/dammafra-sailing-435998.mp3");
const CALM_MP3: &[u8] = include_bytes!("../assets/sounds/calm.mp3");
const STORM_MP3: &[u8] = include_bytes!("../assets/sounds/thunderstorm-cut.mp3");
// Canvas-flap one-shots: raising sail (more canvas catching wind) vs lowering.
const FLAP_UP_MP3: &[u8] = include_bytes!("../assets/sounds/flap1.mp3");
const FLAP_DOWN_MP3: &[u8] = include_bytes!("../assets/sounds/flap2.mp3");
const WIND_SHIFT_MP3: &[u8] = include_bytes!("../assets/sounds/universfield-transition-02-141076.mp3");
// The coin clink on a successful trade/repair/upgrade (`PortView.coinSound`). The
// `pw23check-winning` clip is the original's *race-won* jingle, not this cue.
const COIN_MP3: &[u8] = include_bytes!("../assets/sounds/collect-coin.mp3");

// Loudness ceilings for the three beds (each bed's volume rides between 0 and its
// ceiling) and the gain of the one-shot cues.
const SAIL_MAX_VOL: f32 = 0.5;
const CALM_MAX_VOL: f32 = 0.5;
const STORM_MAX_VOL: f32 = 0.85;
const FLAP_VOL: f32 = 0.9;
const WIND_SHIFT_VOL: f32 = 0.7;
const COIN_VOL: f32 = 0.8;
// The boat speed (knots) at which the sailing bed reaches full voice.
const SAIL_FULL_KN: f32 = 12.0;
// How fast a bed's volume chases its target (per second), so weather and speed
// changes fade rather than jump.
const VOL_EASE: f32 = 1.5;

/// The loaded clips plus the smoothed volumes of the three looping beds.
pub struct SoundBank {
    sailing: Sound,
    calm: Sound,
    storm: Sound,
    flap_up: Sound,
    flap_down: Sound,
    wind_shift: Sound,
    coin: Sound,
    sailing_vol: f32,
    calm_vol: f32,
    storm_vol: f32,
}

impl SoundBank {
    /// Decode and load every clip, then start the three ambient beds looping in
    /// silence — [`update`](Self::update) rides their volumes from there. Must be
    /// awaited after macroquad's context exists (i.e. inside `#[macroquad::main]`).
    pub async fn load() -> SoundBank {
        let sailing = load_clip(SAILING_MP3).await;
        let calm = load_clip(CALM_MP3).await;
        let storm = load_clip(STORM_MP3).await;
        let flap_up = load_clip(FLAP_UP_MP3).await;
        let flap_down = load_clip(FLAP_DOWN_MP3).await;
        let wind_shift = load_clip(WIND_SHIFT_MP3).await;
        let coin = load_clip(COIN_MP3).await;

        // Kick off the beds at zero volume so they're already running and in sync;
        // the per-frame update simply opens them up.
        for bed in [&sailing, &calm, &storm] {
            play_sound(bed, PlaySoundParams { looped: true, volume: 0.0 });
        }

        SoundBank {
            sailing,
            calm,
            storm,
            flap_up,
            flap_down,
            wind_shift,
            coin,
            sailing_vol: 0.0,
            calm_vol: 0.0,
            storm_vol: 0.0,
        }
    }

    /// Ease the three beds toward the targets the current sea state implies and
    /// push the new volumes to the mixer. Call once per frame.
    ///
    /// - `docked` silences the sailing bed (the ship lies at anchor).
    /// - `knots` opens the sailing bed up toward full at [`SAIL_FULL_KN`].
    /// - `storm` (gale fury, 0..1) swells the storm bed and ducks the calm one.
    pub fn update(&mut self, dt: f32, docked: bool, knots: f32, storm: f32) {
        let sail_target = if docked {
            0.0
        } else {
            SAIL_MAX_VOL * (knots / SAIL_FULL_KN).clamp(0.0, 1.0)
        };
        let storm_target = STORM_MAX_VOL * storm.clamp(0.0, 1.0);
        let calm_target = CALM_MAX_VOL * (1.0 - storm).clamp(0.0, 1.0);

        let ease = (VOL_EASE * dt).clamp(0.0, 1.0);
        self.sailing_vol += (sail_target - self.sailing_vol) * ease;
        self.storm_vol += (storm_target - self.storm_vol) * ease;
        self.calm_vol += (calm_target - self.calm_vol) * ease;

        set_sound_volume(&self.sailing, self.sailing_vol);
        set_sound_volume(&self.storm, self.storm_vol);
        set_sound_volume(&self.calm, self.calm_vol);
    }

    /// A canvas *flap* — more sail hauled up, fresh cloth catching the wind.
    pub fn sail_up(&self) {
        self.flap(&self.flap_up);
    }

    /// A canvas *flap* — sail dropped a notch, cloth spilling its wind.
    pub fn sail_down(&self) {
        self.flap(&self.flap_down);
    }

    /// Play a flap, first cancelling any flap still ringing so a rapid haul/drop
    /// retriggers cleanly — only the newest flap sounds, never a pile-up.
    fn flap(&self, clip: &Sound) {
        stop_sound(&self.flap_up);
        stop_sound(&self.flap_down);
        play_sound(clip, PlaySoundParams { looped: false, volume: FLAP_VOL });
    }

    /// A transition *whoosh* — the wind has backed/veered to a fresh quarter.
    pub fn wind_shift(&self) {
        // Restart rather than layer if one's still ringing (`currentTime = 0`).
        stop_sound(&self.wind_shift);
        play_sound(
            &self.wind_shift,
            PlaySoundParams { looped: false, volume: WIND_SHIFT_VOL },
        );
    }

    /// A coin *chime* — a trade, repair or upgrade went through.
    pub fn transaction(&self) {
        // Restart rather than layer on rapid commits (`coinSound.currentTime = 0`).
        stop_sound(&self.coin);
        play_sound(
            &self.coin,
            PlaySoundParams { looped: false, volume: COIN_VOL },
        );
    }
}

/// Decode an embedded MP3 to an in-memory WAV and hand it to macroquad's mixer.
async fn load_clip(mp3: &'static [u8]) -> Sound {
    let wav = decode_mp3_to_wav(mp3);
    load_sound_from_bytes(&wav)
        .await
        .expect("decoded WAV should always load")
}

/// Decode MP3 bytes to interleaved 16-bit PCM and wrap them in a canonical WAV
/// container — the one container quad-snd's `audrey` reliably reads. Returns the
/// WAV bytes ready for `load_sound_from_bytes`.
fn decode_mp3_to_wav(bytes: &'static [u8]) -> Vec<u8> {
    let source = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .expect("probe MP3");
    let mut format = probed.format;
    let track = format.default_track().expect("MP3 has a track").clone();
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .expect("make MP3 decoder");

    let mut samples: Vec<i16> = Vec::new();
    let mut channels: u16 = 2;
    let mut rate: u32 = 44_100;
    let mut buf: Option<SampleBuffer<i16>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // Symphonia signals end-of-stream as an IO `UnexpectedEof`; any other
            // read error ends decoding too — we keep whatever we got.
            Err(_) => break,
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                channels = spec.channels.count() as u16;
                rate = spec.rate;
                let buf = buf.get_or_insert_with(|| {
                    SampleBuffer::<i16>::new(decoded.capacity() as u64, spec)
                });
                buf.copy_interleaved_ref(decoded);
                samples.extend_from_slice(buf.samples());
            }
            // A corrupt frame is recoverable — skip it and keep going.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }

    wav_pcm16(&samples, channels, rate)
}

/// Wrap interleaved 16-bit PCM in a 44-byte-header canonical WAV.
fn wav_pcm16(samples: &[i16], channels: u16, rate: u32) -> Vec<u8> {
    const BITS: u16 = 16;
    let block_align = channels * (BITS / 8);
    let byte_rate = rate * block_align as u32;
    let data_len = (samples.len() * 2) as u32;

    let mut v = Vec::with_capacity(44 + data_len as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVE");
    v.extend_from_slice(b"fmt ");
    v.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt-chunk size
    v.extend_from_slice(&1u16.to_le_bytes()); // format tag: PCM
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&byte_rate.to_le_bytes());
    v.extend_from_slice(&block_align.to_le_bytes());
    v.extend_from_slice(&BITS.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shipped clip must decode to a non-empty, mono/stereo WAV — that's
    /// exactly what quad-snd's audrey will accept, so a pass here means the bank
    /// loads without panicking at runtime.
    #[test]
    fn all_clips_decode_to_playable_wav() {
        for (name, mp3) in [
            ("sailing", SAILING_MP3),
            ("calm", CALM_MP3),
            ("storm", STORM_MP3),
            ("flap_up", FLAP_UP_MP3),
            ("flap_down", FLAP_DOWN_MP3),
            ("wind_shift", WIND_SHIFT_MP3),
            ("coin", COIN_MP3),
        ] {
            let wav = decode_mp3_to_wav(mp3);
            assert!(wav.len() > 44, "{name}: no audio decoded");
            assert_eq!(&wav[0..4], b"RIFF", "{name}: not a WAV");
            // audrey asserts 1 or 2 channels; the fmt chunk's channel count is at
            // byte offset 22 (little-endian u16).
            let channels = u16::from_le_bytes([wav[22], wav[23]]);
            assert!(
                channels == 1 || channels == 2,
                "{name}: {channels} channels (must be mono/stereo)"
            );
        }
    }
}
