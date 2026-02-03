use js_sys::{Array, Uint8Array};
use web_sys::{Blob, BlobPropertyBag, HtmlAudioElement, Url};

use crate::assets::{BELL_SOUND, CHARGE_SOUND};

pub struct Audio {
    charge_sound: HtmlAudioElement,
    threat_sound: HtmlAudioElement,
    bell_sound: HtmlAudioElement,
    is_charging: bool,
    is_threatened: bool,
}

impl Audio {
    pub fn new() -> Self {
        let charge_sound = create_audio_from_bytes(CHARGE_SOUND, "audio/mpeg");
        charge_sound.set_loop(true);

        let threat_sound = create_audio_from_bytes(CHARGE_SOUND, "audio/mpeg");
        threat_sound.set_loop(true);
        threat_sound.set_volume(0.6); // Slightly quieter when being targeted

        let bell_sound = create_audio_from_bytes(BELL_SOUND, "audio/mpeg");
        bell_sound.set_volume(0.6);

        Self {
            charge_sound,
            threat_sound,
            bell_sound,
            is_charging: false,
            is_threatened: false,
        }
    }

    pub fn update_charge(&mut self, has_target: bool, progress: f32) {
        if has_target && progress > 0.0 {
            if !self.is_charging {
                self.charge_sound.set_current_time(0.0);
                let _ = self.charge_sound.play();
                self.is_charging = true;
            }
        } else if self.is_charging {
            self.charge_sound.pause().ok();
            self.is_charging = false;
        }
    }

    /// Update sound when someone is targeting us
    pub fn update_threat(&mut self, is_being_targeted: bool) {
        if is_being_targeted {
            if !self.is_threatened {
                self.threat_sound.set_current_time(0.0);
                let _ = self.threat_sound.play();
                self.is_threatened = true;
            }
        } else if self.is_threatened {
            self.threat_sound.pause().ok();
            self.is_threatened = false;
        }
    }

    pub fn play_death(&self) {
        self.bell_sound.set_current_time(0.0);
        let _ = self.bell_sound.play();
    }
}

fn create_audio_from_bytes(data: &[u8], mime_type: &str) -> HtmlAudioElement {
    let array = Uint8Array::from(data);
    let parts = Array::new();
    parts.push(&array);

    let options = BlobPropertyBag::new();
    options.set_type(mime_type);

    let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &options)
        .expect("Failed to create blob");
    let url = Url::create_object_url_with_blob(&blob).expect("Failed to create object URL");

    HtmlAudioElement::new_with_src(&url).expect("Failed to create audio element")
}
