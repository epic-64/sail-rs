//! Sound — the voyage's audio bed and its event cues.
//!
//! Three ambient loops play continuously and are cross-faded by volume each
//! frame: the **sailing** bed rides up with the boat's speed (silent at anchor),
//! the **calm** sea bed is the base ambience (ducked as a gale rises), and the
//! **storm** bed swells with the gale's fury. Over the top sit one-shot cues:
//! a canvas *flap* when sail is raised or lowered, a transition *whoosh* when the
//! wind shifts quarter, a single coin / coin pour on a market Buy-Sell /
//! Fill-Dump (the pour reused for repairs, upgrades and contract payouts), a
//! confirming stamp on accepting a contract or booking a race, and a brighter
//! *chime* when the hull scoops up floating salvage.
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
// On the web the whole binary is downloaded up front, so the wasm build embeds
// re-encoded (smaller) copies that `build.rs` generates into `OUT_DIR/sounds-web/`
// at build time (ffmpeg if available, else the original copied through). Nothing
// re-encoded is committed. The native build embeds the full-quality
// `assets/sounds/` originals directly.
#[cfg(target_arch = "wasm32")]
macro_rules! snd {
    ($f:literal) => {
        include_bytes!(concat!(env!("OUT_DIR"), "/sounds-web/", $f))
    };
}
#[cfg(not(target_arch = "wasm32"))]
macro_rules! snd {
    ($f:literal) => {
        include_bytes!(concat!("../assets/sounds/", $f))
    };
}

const SAILING_MP3: &[u8] = snd!("dammafra-sailing-435998.mp3");
const CALM_MP3: &[u8] = snd!("calm.mp3");
const STORM_MP3: &[u8] = snd!("thunderstorm-cut.mp3");
// Canvas-flap one-shots: raising sail (more canvas catching wind) vs lowering.
const FLAP_UP_MP3: &[u8] = snd!("flap1.mp3");
const FLAP_DOWN_MP3: &[u8] = snd!("flap2.mp3");
const WIND_SHIFT_MP3: &[u8] = snd!("universfield-transition-02-141076.mp3");
// Market trade cues: a single coin for a per-unit Buy/Sell, a heavier coin pour
// for a bulk Fill/Dump (and reused for repairs, upgrades and contract payouts).
const ONE_COIN_MP3: &[u8] = snd!("one-coin.mp3");
const COINS_MP3: &[u8] = snd!("coins.mp3");
// A confirming stamp for committing to a venture: accepting or abandoning a
// contract, or booking a race.
const ACCEPT_MP3: &[u8] = snd!("accept.mp3");
// The bright chime as the hull scoops floating salvage (`SailingView.collectSound`)
// — a distinct clip from the trade coin.
const SALVAGE_MP3: &[u8] = snd!("collect-item.mp3");
// Race result one-shots: a triumphant chime on a win, a glum jingle on a loss
// (`SailingView.raceWonSound` / `raceLostSound`).
const RACE_WON_MP3: &[u8] = snd!("pw23check-winning-218995.mp3");
const RACE_LOST_MP3: &[u8] = snd!("lightyeartraxx-kl-peach-game-over-iii-142453.mp3");
// A short buzzer when the captain tries something the rules won't allow (no gold
// for a wager, no room in the hold, nothing to sell, …).
const INVALID_MP3: &[u8] = snd!("invalid-input.mp3");

// Loudness ceilings for the three beds (each bed's volume rides between 0 and its
// ceiling) and the gain of the one-shot cues. These started from the per-clip
// volumes the original `SailingView`/`PortView` assigned to each `<audio>` element
// (sailing/calm/storm beds at 0.5/0.5/0.6, the flap/wind-shift/coin one-shots at
// the browser default 1.0, salvage at 0.6, race stings at 0.25 since their clips
// are mastered hot), since tuned: the sailing bed lifted 50% (0.5 -> 0.75) so it
// reads over the sea, and the trade coin dropped 50% (1.0 -> 0.5) as it was too hot.
const SAIL_MAX_VOL: f32 = 0.75;
const CALM_MAX_VOL: f32 = 0.5;
const STORM_MAX_VOL: f32 = 0.6;
const FLAP_VOL: f32 = 1.0;
const WIND_SHIFT_VOL: f32 = 1.0;
// Market Buy/Sell (one coin) and Fill/Dump (coin pour), both at half voice.
const ONE_COIN_VOL: f32 = 0.5;
const COINS_VOL: f32 = 0.5;
// Accept/abandon contract, book a race.
const ACCEPT_VOL: f32 = 0.5;
const SALVAGE_VOL: f32 = 0.6;
const RACE_WON_VOL: f32 = 0.25;
const RACE_LOST_VOL: f32 = 0.25;
// The invalid-action buzzer — present enough to register without nagging.
const INVALID_VOL: f32 = 0.4;
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
    one_coin: Sound,
    coins: Sound,
    accept: Sound,
    salvage: Sound,
    race_won: Sound,
    race_lost: Sound,
    invalid: Sound,
    sailing_vol: f32,
    calm_vol: f32,
    storm_vol: f32,
    /// Master gain (0..1), set from the options menu, that every clip's own
    /// volume is multiplied by — the single dial over the whole mix.
    master: f32,
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
        let one_coin = load_clip(ONE_COIN_MP3).await;
        let coins = load_clip(COINS_MP3).await;
        let accept = load_clip(ACCEPT_MP3).await;
        let salvage = load_clip(SALVAGE_MP3).await;
        let race_won = load_clip(RACE_WON_MP3).await;
        let race_lost = load_clip(RACE_LOST_MP3).await;
        let invalid = load_clip(INVALID_MP3).await;

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
            one_coin,
            coins,
            accept,
            salvage,
            race_won,
            race_lost,
            invalid,
            sailing_vol: 0.0,
            calm_vol: 0.0,
            storm_vol: 0.0,
            master: 1.0,
        }
    }

    /// The current master gain (0..1), as the options slider shows it.
    pub fn master(&self) -> f32 {
        self.master
    }

    /// Set the master gain (0..1). Beds pick it up on the next [`update`](Self::update);
    /// one-shots use it the next time they fire.
    pub fn set_master(&mut self, v: f32) {
        self.master = v.clamp(0.0, 1.0);
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

        set_sound_volume(&self.sailing, self.sailing_vol * self.master);
        set_sound_volume(&self.storm, self.storm_vol * self.master);
        set_sound_volume(&self.calm, self.calm_vol * self.master);
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
        play_sound(clip, PlaySoundParams { looped: false, volume: FLAP_VOL * self.master });
    }

    /// A transition *whoosh* — the wind has backed/veered to a fresh quarter.
    pub fn wind_shift(&self) {
        // Restart rather than layer if one's still ringing (`currentTime = 0`).
        stop_sound(&self.wind_shift);
        play_sound(
            &self.wind_shift,
            PlaySoundParams { looped: false, volume: WIND_SHIFT_VOL * self.master },
        );
    }

    /// A single coin — a per-unit market Buy or Sell went through. Restarted so a
    /// rapid run of one-unit trades retriggers cleanly rather than piling up.
    pub fn trade_one(&self) {
        stop_sound(&self.one_coin);
        play_sound(
            &self.one_coin,
            PlaySoundParams { looped: false, volume: ONE_COIN_VOL * self.master },
        );
    }

    /// A pour of coins — a bulk market Fill or Dump went through. Restarted so a
    /// rapid run of bulk trades retriggers cleanly rather than piling up.
    pub fn trade_bulk(&self) {
        stop_sound(&self.coins);
        play_sound(
            &self.coins,
            PlaySoundParams { looped: false, volume: COINS_VOL * self.master },
        );
    }

    /// A confirming stamp — a contract accepted or abandoned, or a race booked.
    /// Restarted so a rapid run of commits retriggers cleanly rather than piling up.
    pub fn accept(&self) {
        stop_sound(&self.accept);
        play_sound(
            &self.accept,
            PlaySoundParams { looped: false, volume: ACCEPT_VOL * self.master },
        );
    }

    /// A bright *chime* — salvage hauled aboard from the swell. Its own clip
    /// (`collect-item`), distinct from the trade coin, played at the original
    /// level and restarted so a rapid run of pickups retriggers cleanly rather
    /// than piling up.
    pub fn salvage(&self) {
        stop_sound(&self.salvage);
        play_sound(
            &self.salvage,
            PlaySoundParams { looped: false, volume: SALVAGE_VOL * self.master },
        );
    }

    /// A triumphant chime — the player took the race.
    pub fn race_won(&self) {
        stop_sound(&self.race_won);
        play_sound(
            &self.race_won,
            PlaySoundParams { looped: false, volume: RACE_WON_VOL * self.master },
        );
    }

    /// A glum jingle — the rival reached the mark first.
    pub fn race_lost(&self) {
        stop_sound(&self.race_lost);
        play_sound(
            &self.race_lost,
            PlaySoundParams { looped: false, volume: RACE_LOST_VOL * self.master },
        );
    }

    /// A short buzzer — the captain tried something the rules forbid (no gold for
    /// the wager, no room in the hold, nothing to sell). Restarted so a flurry of
    /// rejected key-presses retriggers cleanly rather than piling up.
    pub fn invalid(&self) {
        stop_sound(&self.invalid);
        play_sound(
            &self.invalid,
            PlaySoundParams { looped: false, volume: INVALID_VOL * self.master },
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

    // Symphonia signals end-of-stream as an IO `UnexpectedEof`; any other read
    // error ends decoding too — we keep whatever we got and stop.
    while let Ok(packet) = format.next_packet() {
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
            ("one_coin", ONE_COIN_MP3),
            ("coins", COINS_MP3),
            ("accept", ACCEPT_MP3),
            ("salvage", SALVAGE_MP3),
            ("race_won", RACE_WON_MP3),
            ("race_lost", RACE_LOST_MP3),
            ("invalid", INVALID_MP3),
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
