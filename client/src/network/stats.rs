//! Connection statistics and display.
//!
//! Provides functions to collect and display WebRTC connection statistics.

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::RtcPeerConnection;

use super::protocol::PeerId;

/// Connection type based on ICE candidate type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionType {
    /// Direct peer-to-peer connection (host candidate).
    Direct,
    /// Connection via STUN (server-reflexive or peer-reflexive).
    Stun,
    /// Connection via TURN relay.
    Turn,
    /// Unknown/connecting.
    Unknown,
}

/// Statistics for a single peer connection.
#[derive(Clone, Debug)]
pub struct PeerStats {
    pub peer_id: PeerId,
    pub name: Option<String>,
    pub connection_type: ConnectionType,
    /// Round-trip time in milliseconds, if available.
    pub rtt_ms: Option<f64>,
}

/// Fetch stats from a single peer connection and parse the RTCStatsReport.
pub async fn fetch_peer_stats(
    peer_id: PeerId,
    name: Option<String>,
    pc: RtcPeerConnection,
) -> Option<PeerStats> {
    // get_stats() returns a Promise<RTCStatsReport>
    let stats_promise = pc.get_stats();
    let stats_report = match JsFuture::from(stats_promise).await {
        Ok(report) => report,
        Err(e) => {
            log::debug!("Failed to get stats for peer {}: {:?}", peer_id, e);
            return None;
        }
    };

    // RTCStatsReport is a JS Map - iterate through it
    let stats_map: js_sys::Map = stats_report.unchecked_into();

    let mut connection_type = ConnectionType::Unknown;
    let mut rtt_ms: Option<f64> = None;
    let mut local_candidate_id: Option<String> = None;

    // First pass: find the succeeded candidate-pair
    stats_map.for_each(&mut |value, _key| {
        if let Ok(obj) = js_sys::Reflect::get(&value, &"type".into())
            && let Some(type_str) = obj.as_string()
            && type_str == "candidate-pair"
            && let Ok(state) = js_sys::Reflect::get(&value, &"state".into())
            && state.as_string().as_deref() == Some("succeeded")
        {
            // Get RTT
            if let Ok(rtt) = js_sys::Reflect::get(&value, &"currentRoundTripTime".into())
                && let Some(rtt_secs) = rtt.as_f64()
            {
                rtt_ms = Some(rtt_secs * 1000.0);
            }
            // Get local candidate ID to look up candidate type
            if let Ok(local_id) = js_sys::Reflect::get(&value, &"localCandidateId".into()) {
                local_candidate_id = local_id.as_string();
            }
        }
    });

    // Second pass: find the local candidate to get the connection type
    if let Some(ref candidate_id) = local_candidate_id {
        stats_map.for_each(&mut |value, key| {
            if key.as_string().as_deref() == Some(candidate_id.as_str())
                && let Ok(candidate_type) = js_sys::Reflect::get(&value, &"candidateType".into())
                && let Some(type_str) = candidate_type.as_string()
            {
                connection_type = match type_str.as_str() {
                    "host" => ConnectionType::Direct,
                    "srflx" | "prflx" => ConnectionType::Stun,
                    "relay" => ConnectionType::Turn,
                    _ => ConnectionType::Unknown,
                };
            }
        });
    }

    Some(PeerStats {
        peer_id,
        name,
        connection_type,
        rtt_ms,
    })
}

/// Update the peer stats display panel in the UI.
pub fn update_peer_stats_display(stats: &[PeerStats]) {
    let Some(container) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("peer-stats-list"))
    else {
        return;
    };

    // Clear existing content
    container.set_inner_html("");

    if stats.is_empty() {
        let div = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.create_element("div").ok())
            .unwrap();
        let _ = div.set_attribute("class", "no-peers");
        div.set_text_content(Some("No peers connected"));
        let _ = container.append_child(&div);
        return;
    }

    for stat in stats {
        let row = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.create_element("div").ok())
            .unwrap();
        let _ = row.set_attribute("class", "peer-row");

        // Peer ID and name
        let id_span = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.create_element("span").ok())
            .unwrap();
        let _ = id_span.set_attribute("class", "peer-id");
        let id_text = match &stat.name {
            Some(name) => format!("#{} {}", stat.peer_id % 100, name),
            None => format!("#{}", stat.peer_id % 100),
        };
        id_span.set_text_content(Some(&id_text));

        // Connection type
        let type_span = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.create_element("span").ok())
            .unwrap();
        let (type_text, type_class) = match stat.connection_type {
            ConnectionType::Direct => ("Direct", "peer-type type-direct"),
            ConnectionType::Stun => ("STUN", "peer-type type-stun"),
            ConnectionType::Turn => ("TURN", "peer-type type-turn"),
            ConnectionType::Unknown => ("...", "peer-type"),
        };
        let _ = type_span.set_attribute("class", type_class);
        type_span.set_text_content(Some(type_text));

        // RTT
        let rtt_span = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.create_element("span").ok())
            .unwrap();
        let _ = rtt_span.set_attribute("class", "peer-rtt");
        let rtt_text = match stat.rtt_ms {
            Some(rtt) => format!("{:.0}ms", rtt),
            None => "-".to_string(),
        };
        rtt_span.set_text_content(Some(&rtt_text));

        let _ = row.append_child(&id_span);
        let _ = row.append_child(&type_span);
        let _ = row.append_child(&rtt_span);
        let _ = container.append_child(&row);
    }
}
