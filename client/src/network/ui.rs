//! Network status UI logging.
//!
//! Provides functions to display network connection status in the HTML UI.

/// Log level for network status messages.
#[derive(Clone, Copy, Debug)]
pub enum NetLogLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// Add a line to the network status log in the UI.
pub fn net_log(level: NetLogLevel, msg: &str) {
    let class = match level {
        NetLogLevel::Info => "info",
        NetLogLevel::Success => "success",
        NetLogLevel::Warning => "warning",
        NetLogLevel::Error => "error",
    };

    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(container) = doc.get_element_by_id("network-status")
        && let Ok(div) = doc.create_element("div")
    {
        let _ = div.set_attribute("class", &format!("log-line {}", class));
        div.set_text_content(Some(msg));
        let _ = container.append_child(&div);
        // Auto-scroll to bottom
        container.set_scroll_top(container.scroll_height());
    }
}
