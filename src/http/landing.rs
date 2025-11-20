/// Landing Page Handler
///
/// Generates HTML landing page for the Nostr relay.
use crate::config::Config;

/// Generate the HTML landing page
pub fn get_html(config: &Config) -> String {
    format!(
        include_str!("../../templates/landing.html"),
        relay_name = config.relay_name,
        relay_description = config.relay_description,
        domain = config.domain,
        bind_address = config.bind_address,
    )
}
