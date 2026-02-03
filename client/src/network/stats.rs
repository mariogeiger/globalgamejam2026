//! Connection statistics and display.
//!
//! Provides functions to collect and display WebRTC connection statistics.

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::RtcPeerConnection;

use super::protocol::PeerId;

/// Connection type based on ICE candidate type (RFC 8445).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionType {
    /// Host candidate - direct local connection.
    Host,
    /// Server-reflexive candidate - discovered via STUN server.
    Srflx(String),
    /// Peer-reflexive candidate - discovered during ICE checks.
    Prflx(String),
    /// Relay candidate - via TURN relay server.
    Relay(String),
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

    // Second pass: find the local candidate to get the connection type and TURN server URL
    if let Some(ref candidate_id) = local_candidate_id {
        stats_map.for_each(&mut |value, key| {
            if key.as_string().as_deref() == Some(candidate_id.as_str())
                && let Ok(candidate_type) = js_sys::Reflect::get(&value, &"candidateType".into())
                && let Some(type_str) = candidate_type.as_string()
            {
                connection_type = match type_str.as_str() {
                    "host" => ConnectionType::Host,
                    "srflx" => {
                        // Extract STUN server URL for server-reflexive candidates
                        if let Ok(url) = js_sys::Reflect::get(&value, &"url".into())
                            && let Some(url_str) = url.as_string()
                        {
                            ConnectionType::Srflx(url_str)
                        } else {
                            ConnectionType::Srflx("unknown".to_string())
                        }
                    }
                    "prflx" => {
                        // Extract STUN server URL for peer-reflexive candidates
                        if let Ok(url) = js_sys::Reflect::get(&value, &"url".into())
                            && let Some(url_str) = url.as_string()
                        {
                            ConnectionType::Prflx(url_str)
                        } else {
                            ConnectionType::Prflx("unknown".to_string())
                        }
                    }
                    "relay" => {
                        // Extract TURN server URL for relay candidates
                        if let Ok(url) = js_sys::Reflect::get(&value, &"url".into())
                            && let Some(url_str) = url.as_string()
                        {
                            ConnectionType::Relay(url_str)
                        } else {
                            ConnectionType::Relay("unknown".to_string())
                        }
                    }
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

        // Connection type name and server URL
        let type_container = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.create_element("span").ok())
            .unwrap();
        let _ = type_container.set_attribute("class", "peer-type-container");

        // Type name span
        let type_name_span = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.create_element("span").ok())
            .unwrap();
        let (type_name, type_class) = match &stat.connection_type {
            ConnectionType::Host => ("host", "peer-type type-host"),
            ConnectionType::Srflx(_) => ("srflx", "peer-type type-srflx"),
            ConnectionType::Prflx(_) => ("prflx", "peer-type type-prflx"),
            ConnectionType::Relay(_) => ("relay", "peer-type type-relay"),
            ConnectionType::Unknown => ("...", "peer-type"),
        };
        let _ = type_name_span.set_attribute("class", type_class);
        type_name_span.set_text_content(Some(type_name));
        let _ = type_container.append_child(&type_name_span);

        // Server URL span (if applicable)
        if let Some(display_url) = match &stat.connection_type {
            ConnectionType::Host | ConnectionType::Unknown => None,
            ConnectionType::Srflx(url) | ConnectionType::Prflx(url) => {
                // Extract just the hostname:port from stun:hostname:port
                url.strip_prefix("stun:")
                    .or_else(|| url.strip_prefix("stuns:"))
                    .and_then(|s| s.split('?').next())
            }
            ConnectionType::Relay(url) => {
                // Extract just the hostname:port from turn:hostname:port?transport=udp
                url.strip_prefix("turn:")
                    .or_else(|| url.strip_prefix("turns:"))
                    .and_then(|s| s.split('?').next())
            }
        } {
            let url_span = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.create_element("span").ok())
                .unwrap();
            let _ = url_span.set_attribute("class", "peer-type-url");
            url_span.set_text_content(Some(&format!(" ({})", display_url)));
            let _ = type_container.append_child(&url_span);
        }

        let type_span = type_container;

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
