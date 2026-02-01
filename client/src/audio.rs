use js_sys::{Array, Uint8Array};
use web_sys::{Blob, BlobPropertyBag, HtmlAudioElement, Url};

const CHARGE_SOUND: &[u8] = include_bytes!("../assets/laser-charge-175727.mp3");
const BELL_SOUND: &[u8] = include_bytes!("../assets/bell.mp3");

pub struct Audio {
    charge_sound: HtmlAudioElement,
    bell_sound: HtmlAudioElement,
    is_charging: bool,
}

impl Audio {
    pub fn new() -> Self {
        let charge_sound = create_audio_from_bytes(CHARGE_SOUND, "audio/mpeg");
        charge_sound.set_loop(true);
        let bell_sound = create_audio_from_bytes(BELL_SOUND, "audio/mpeg");
        Self {
            charge_sound,
            bell_sound,
            is_charging: false,
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
