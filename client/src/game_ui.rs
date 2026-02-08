use base64::Engine;
use glam::Vec3;

use crate::assets::{COWARD_IMAGE, GHOST_IMAGE, HUNTER_IMAGE};
use crate::network::PeerId;
use crate::player::MaskType;

// ---------------------------------------------------------------------------
// Core helpers
// ---------------------------------------------------------------------------

fn doc() -> Option<web_sys::Document> {
    web_sys::window().and_then(|w| w.document())
}

fn set_visible(id: &str, visible: bool) {
    if let Some(doc) = doc() {
        if let Some(el) = doc.get_element_by_id(id) {
            let display = if visible {
                "display: block;"
            } else {
                "display: none;"
            };
            let _ = el.set_attribute("style", display);
        }
    }
}

fn set_text(id: &str, text: &str) {
    if let Some(doc) = doc() {
        if let Some(el) = doc.get_element_by_id(id) {
            el.set_text_content(Some(text));
        }
    }
}

// ---------------------------------------------------------------------------
// Simple overlay show/hide
// ---------------------------------------------------------------------------

pub fn show_waiting() {
    set_visible("waiting-overlay", true);
}
pub fn hide_waiting() {
    set_visible("waiting-overlay", false);
}
pub fn show_countdown() {
    set_visible("countdown-overlay", true);
}
pub fn hide_countdown() {
    set_visible("countdown-overlay", false);
}
pub fn show_spectating() {
    set_visible("spectating-overlay", true);
}
pub fn hide_spectating() {
    set_visible("spectating-overlay", false);
}
pub fn hide_death() {
    set_visible("death-overlay", false);
}
pub fn hide_round_end() {
    set_visible("victory-overlay", false);
}

// ---------------------------------------------------------------------------
// Data-driven overlays
// ---------------------------------------------------------------------------

pub fn show_death(killer_name: Option<&str>) {
    set_visible("death-overlay", true);
    set_text("killer-id", killer_name.unwrap_or("Unknown"));
}

pub fn show_countdown_timer(seconds: u32) {
    show_countdown();
    set_text("countdown-timer", &seconds.to_string());
}

pub fn update_countdown_timer(seconds: u32) {
    set_text("countdown-timer", &seconds.to_string());
}

pub fn update_round_end_timer(seconds: u32) {
    set_text("victory-countdown", &seconds.to_string());
}

// ---------------------------------------------------------------------------
// Complex rendering
// ---------------------------------------------------------------------------

/// Data for the end-of-round overlay.
pub struct RoundOutcome {
    pub local_survived: bool,
    pub survivor_name: Option<String>,
    pub scores: Vec<ScoreEntry>,
    pub kill_feed: Vec<(String, String)>,
}

pub struct ScoreEntry {
    pub name: String,
    pub kills: u32,
    pub is_local: bool,
    pub is_survivor: bool,
}

pub fn show_round_end(outcome: &RoundOutcome) {
    let Some(doc) = doc() else { return };

    if let Some(overlay) = doc.get_element_by_id("victory-overlay") {
        let border_color = if outcome.local_survived {
            "#a6e3a1"
        } else {
            "#f38ba8"
        };
        let _ = overlay.set_attribute(
            "style",
            &format!("display: block; border-color: {};", border_color),
        );
    }
    if let Some(title) = doc.get_element_by_id("victory-title") {
        let _ = title.set_attribute(
            "style",
            if outcome.local_survived {
                "color: #a6e3a1;"
            } else {
                "color: #f38ba8;"
            },
        );
        title.set_text_content(Some(if outcome.local_survived {
            "YOU SURVIVED!"
        } else {
            "YOU DIED"
        }));
    }
    if let Some(subtitle) = doc.get_element_by_id("victory-subtitle") {
        match &outcome.survivor_name {
            Some(name) if !outcome.local_survived => {
                subtitle.set_text_content(Some(&format!("{} survived", name)));
                let _ = subtitle.set_attribute("style", "display: block;");
            }
            _ => {
                let _ = subtitle.set_attribute("style", "display: none;");
            }
        }
    }

    // Scoreboard
    if let Some(scoreboard) = doc.get_element_by_id("scoreboard") {
        let mut html = String::new();
        for entry in &outcome.scores {
            let mut classes = Vec::new();
            if entry.is_local {
                classes.push("local");
            }
            if entry.is_survivor {
                classes.push("survivor");
            }
            let class_str = classes.join(" ");
            html.push_str(&format!(
                r#"<div class="score-row {}"><span class="name">{}</span><span class="kills">{}</span></div>"#,
                class_str, entry.name, entry.kills
            ));
        }
        scoreboard.set_inner_html(&html);
    }

    // Kill feed
    if let Some(feed_el) = doc.get_element_by_id("kill-feed") {
        if outcome.kill_feed.is_empty() {
            feed_el.set_inner_html(r#"<div class="no-kills">No kills this round</div>"#);
        } else {
            let mut html = String::new();
            for (killer, victim) in &outcome.kill_feed {
                html.push_str(&format!(
                    r#"<div class="kill-entry"><span class="killer">{}</span> <span class="kill-arrow">→</span> <span class="victim">{}</span></div>"#,
                    killer, victim
                ));
            }
            feed_el.set_inner_html(&html);
        }
    }
}

// ---------------------------------------------------------------------------
// HUD updates
// ---------------------------------------------------------------------------

pub fn update_player_counts(alive: usize, dead: usize, spectating: usize) {
    set_text("count-alive", &alive.to_string());
    set_text("count-dead", &dead.to_string());
    set_text("count-spectator", &spectating.to_string());
}

pub fn update_peer_id(id: PeerId) {
    set_text("local-peer-id", &id.to_string());
}

pub fn update_player_name(name: &str) {
    set_text("local-player-name", name);
}

pub fn update_position(pos: Vec3) {
    set_text(
        "local-pos",
        &format!("[{:.1}, {:.1}, {:.1}]", pos.x, pos.y, pos.z),
    );
}

pub fn update_mask_selector(mask: MaskType) {
    let Some(doc) = doc() else { return };

    let mask_ids = ["mask-ghost", "mask-coward", "mask-hunter"];
    let active_id = match mask {
        MaskType::Ghost => "mask-ghost",
        MaskType::Coward => "mask-coward",
        MaskType::Hunter => "mask-hunter",
    };

    for id in mask_ids {
        if let Some(elem) = doc.get_element_by_id(id) {
            if id == active_id {
                let _ = elem.set_attribute("class", "mask-slot active");
            } else {
                let _ = elem.set_attribute("class", "mask-slot");
            }
        }
    }
}

/// Initialize mask selector images with embedded data URLs
pub fn init_mask_images() {
    let Some(doc) = doc() else { return };

    let engine = base64::engine::general_purpose::STANDARD;

    let masks = [
        ("mask-ghost", GHOST_IMAGE),
        ("mask-coward", COWARD_IMAGE),
        ("mask-hunter", HUNTER_IMAGE),
    ];

    for (id, image_data) in masks {
        if let Some(elem) = doc.get_element_by_id(id)
            && let Some(img) = elem.query_selector("img").ok().flatten()
        {
            let b64 = engine.encode(image_data);
            let data_url = format!("data:image/png;base64,{}", b64);
            let _ = img.set_attribute("src", &data_url);
        }
    }
}
