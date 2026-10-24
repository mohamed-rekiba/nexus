//! Theme sounds through the system's default output, with `rodio`, on a thread of their own.
//! Without an output device the launcher stays silent and says so once in the log. The `move`
//! sound plays at most once in 40 ms, so a held key doesn't stack its clicks.

use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The shortest time between two `move` sounds.
const MOVE_GAP: Duration = Duration::from_millis(40);

/// Which sounds play: `move` at most once in [`MOVE_GAP`], every other sound each time.
#[derive(Default)]
pub struct Throttle {
    last_move: Option<Instant>,
}

impl Throttle {
    /// Whether the sound `name` asked for at `now` plays.
    pub fn pass(&mut self, name: &str, now: Instant) -> bool {
        if name != "move" {
            return true;
        }
        if self.last_move.is_some_and(|last| now.saturating_duration_since(last) < MOVE_GAP) {
            return false;
        }
        self.last_move = Some(now);
        true
    }
}

/// Plays sounds sent to it. Cheap to clone.
#[derive(Clone)]
pub struct Player {
    tx: Sender<Arc<[u8]>>,
    throttle: Arc<Mutex<Throttle>>,
}

impl Player {
    pub fn start() -> Self {
        let (tx, rx) = channel::<Arc<[u8]>>();
        let started = std::thread::Builder::new().name("audio".into()).spawn(move || {
            // Opened on the first sound, so a silent session never touches the device.
            let mut output: Option<rodio::MixerDeviceSink> = None;
            let mut failed = false;
            for bytes in rx {
                if output.is_none() && !failed {
                    match rodio::DeviceSinkBuilder::open_default_sink() {
                        Ok(mut sink) => {
                            // It goes when the launcher closes; rodio would say so on stderr.
                            sink.log_on_drop(false);
                            output = Some(sink);
                        }
                        Err(error) => {
                            tracing::warn!(%error, "no audio output; theme sounds are off");
                            failed = true;
                        }
                    }
                }
                let Some(sink) = &output else { continue };
                match rodio::Decoder::new(std::io::Cursor::new(bytes)) {
                    Ok(sound) => sink.mixer().add(sound),
                    Err(error) => tracing::warn!(%error, "a theme sound could not be decoded"),
                }
            }
        });
        if let Err(error) = started {
            tracing::warn!(%error, "could not start the audio thread");
        }
        Self { tx, throttle: Arc::default() }
    }

    /// Plays the sound `name`, whose file is `bytes` (WAV or Ogg Vorbis), without waiting,
    /// unless [`Throttle`] drops it.
    pub fn play(&self, name: &str, bytes: Arc<[u8]>) {
        if self.throttle.lock().unwrap_or_else(|p| p.into_inner()).pass(name, Instant::now()) {
            let _ = self.tx.send(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_theme_s_sounds_decode() {
        for name in ["move", "select", "back", "error", "start"] {
            let path = format!("{}/../../plugins/themes/default/theme/sounds/{name}.wav", env!("CARGO_MANIFEST_DIR"));
            let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            let sound = rodio::Decoder::new(std::io::Cursor::new(bytes)).expect("a WAV rodio plays");
            assert!(rodio::Source::total_duration(&sound).is_some_and(|d| d.as_millis() > 10), "{name}");
        }
    }

    #[test]
    fn the_move_sound_plays_at_most_once_in_40_ms() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut throttle = Throttle::default();
        assert!(throttle.pass("move", start));
        assert!(!throttle.pass("move", at(39)), "too soon");
        assert!(throttle.pass("select", at(39)), "other sounds always play");
        assert!(throttle.pass("move", at(40)));
        assert!(!throttle.pass("move", at(60)));
        assert!(throttle.pass("move", at(80)), "40 ms after the last one that played");
    }
}
