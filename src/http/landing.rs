/// Landing Page Handler
///
/// Generates HTML landing page for the Nostr relay.
use crate::config::Config;

/// Get the software version string (version + optional git commit)
fn get_version() -> String {
    let version = env!("CARGO_PKG_VERSION");
    match option_env!("GIT_COMMIT_SHORT") {
        Some(commit) if !commit.is_empty() => format!("v{}-{}", version, commit),
        _ => format!("v{}", version),
    }
}

/// Generate the footer JavaScript that sets the domain dynamically
fn get_footer_script() -> &'static str {
    r#"<script>
        (function() {
            var footerDomain = document.getElementById('footer-domain');
            if (footerDomain) {
                footerDomain.textContent = window.location.host;
            }
        })();
    </script>"#
}

/// Generate the common base CSS used across all pages
fn get_base_css() -> &'static str {
    r#":root {
            --brand: #4434FF;
            --brand-light: #6b5fff;
            --bg: #0a0a0f;
            --surface: #12121a;
            --border: #1e1e2e;
            --text: #e4e4eb;
            --text-muted: #a8a8bd;
            --error: #ff4444;
            --success: #22c55e;
        }
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', 'Inter', sans-serif;
            line-height: 1.6;
            background: var(--bg);
            color: var(--text);
            min-height: 100vh;
        }
        a { color: var(--brand-light); text-decoration: none; }
        a:hover { text-decoration: underline; }
        code {
            background: var(--border);
            padding: 4px 8px;
            border-radius: 4px;
            font-family: 'SF Mono', 'Consolas', monospace;
            font-size: 0.875rem;
            color: var(--brand-light);
        }
        .footer {
            margin-top: 48px;
            padding-top: 24px;
            border-top: 1px solid var(--border);
            text-align: center;
            color: var(--text-muted);
            font-size: 0.875rem;
        }
        .footer-separator {
            margin: 0 0.5em;
            opacity: 0.5;
        }
        .software-box {
            display: flex;
            align-items: flex-start;
            gap: 16px;
            text-align: left;
        }
        .software-logo {
            width: 48px;
            height: 48px;
            flex-shrink: 0;
        }
        .software-content {
            flex: 1;
        }
        .software-heading {
            font-size: 1.125rem;
            font-weight: 500;
            margin-bottom: 8px;
            color: var(--text-muted);
        }
        .software-heading a {
            color: var(--brand-light);
        }
        .software-heading a:hover {
            text-decoration: underline;
        }
        .software-desc {
            color: var(--text-muted);
            font-size: 0.9rem;
            line-height: 1.5;
        }"#
}

/// Generate the software-box HTML component
fn get_software_box_html() -> &'static str {
    r##"<div class="card">
                <div class="software-box">
                    <svg class="software-logo" viewBox="0 0 38 38" fill="none" xmlns="http://www.w3.org/2000/svg">
                        <rect width="38" height="38" rx="12" fill="#4434FF"/>
                        <path d="M10.6731 30.6348C8.83687 30.6346 7.34885 29.1458 7.34885 27.3096C7.34891 26.2473 7.84783 25.303 8.62326 24.6943C8.21265 23.3055 7.86571 22.049 7.45334 20.6758C6.90247 18.8412 7.4492 16.8197 8.93576 15.5605L15.7512 9.78906C15.6931 9.54286 15.6614 9.28642 15.6613 9.02246C15.6613 7.51617 16.6628 6.24465 18.0363 5.83594L18.0363 -1.11215e-06C18.511 0.000462658 18.4612 0.000975391 18.9856 0.000975533C19.5102 0.000975578 19.5802 -1.11589e-06 19.9367 -9.46012e-07L19.9367 5.83594C21.3097 6.24503 22.3108 7.5166 22.3108 9.02246C22.3107 9.29118 22.2792 9.55249 22.219 9.80273L29.0783 15.6123C30.5229 16.8359 31.1022 18.8013 30.5539 20.6133L29.3254 24.6758C30.1142 25.2837 30.6232 26.2367 30.6233 27.3096C30.6233 29.1459 29.1344 30.6348 27.2981 30.6348C25.4619 30.6346 23.9738 29.1458 23.9738 27.3096C23.974 25.4734 25.4619 23.9846 27.2981 23.9844C27.3814 23.9844 27.4643 23.9891 27.5461 23.9951L28.7356 20.0625C29.0645 18.9753 28.7166 17.7966 27.8498 17.0625L21.2424 11.4648C20.8746 11.8048 20.4294 12.0622 19.9367 12.209L19.9367 18.9258C21.0425 19.3175 21.836 20.3694 21.8362 21.6094C21.8362 23.1834 20.5596 24.46 18.9856 24.46C17.4117 24.4598 16.136 23.1833 16.136 21.6094C16.1361 20.3689 16.93 19.3172 18.0363 18.9258L18.0363 12.21C17.5395 12.0622 17.0916 11.801 16.7219 11.457L10.1643 17.0107C9.27919 17.7605 8.93068 18.9867 9.27365 20.1289C9.68708 21.5056 10.0175 22.7009 10.3986 23.998C10.4892 23.9906 10.5806 23.9844 10.6731 23.9844C12.5093 23.9844 13.9981 25.4733 13.9983 27.3096C13.9983 29.1459 12.5094 30.6348 10.6731 30.6348Z" fill="white"/>
                    </svg>
                    <div class="software-content">
                        <h3 class="software-heading"><a href="https://gitworkshop.dev/danconwaydev.com/ngit-grasp">Grasp Server</a> Powered by <a href="https://gitworkshop.dev/danconwaydev.com/ngit-grasp">ngit-grasp</a></h3>
                        <p class="software-desc">Git hosting distributed across relays using Nostr for authorization. <a href="https://ngit.dev/grasp">Find out more...</a></p>
                    </div>
                </div>
            </div>"##
}

/// Generate the HTML landing page
pub fn get_html(config: &Config) -> String {
    // Curation matches NIP-11 document - currently None for this relay
    let curation = "None".to_string();

    format!(
        include_str!("../../templates/landing.html"),
        base_css = get_base_css(),
        relay_name = config.relay_name,
        relay_description = config.relay_description,
        version = get_version(),
        curation = curation,
    )
}

/// Generate a generic 404 page for unknown paths
///
/// Used for any path that doesn't match a known route
pub fn get_generic_404_html(config: &Config, path: &str) -> String {
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Not Found - {relay_name}</title>
    <style>
        {base_css}
        body {{
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 24px;
        }}
        .container {{ max-width: 480px; text-align: center; }}
        .error-code {{
            font-size: 6rem;
            font-weight: 700;
            color: var(--error);
            line-height: 1;
            margin-bottom: 8px;
        }}
        h2 {{ font-size: 1.5rem; font-weight: 500; margin-bottom: 16px; }}
        p {{ color: var(--text-muted); margin-bottom: 24px; }}
        .path-info {{
            background: var(--surface);
            border: 1px solid var(--border);
            border-radius: 8px;
            padding: 16px;
            margin-bottom: 32px;
        }}
        .path-label {{
            font-size: 0.75rem;
            text-transform: uppercase;
            letter-spacing: 0.1em;
            color: var(--text-muted);
            margin-bottom: 8px;
        }}
        code {{ word-break: break-all; }}
        .footer {{ margin-top: 48px; }}
        .footer-separator {{ margin: 0 0.5em; opacity: 0.5; }}
    </style>
</head>
<body>
    <div class="container">
        <div class="error-code">404</div>
        <h2>Page Not Found</h2>
        <p>The page you're looking for doesn't exist.</p>
        <div class="path-info">
            <div class="path-label">Requested Path</div>
            <code>{path}</code>
        </div>
        <a href="/">&larr; Back to {relay_name}</a>
        <div class="footer"><span id="footer-domain"></span><span class="footer-separator">•</span>powered by <a href="https://gitworkshop.dev/danconwaydev.com/ngit-grasp"><strong>ngit-grasp</strong></a><span class="footer-separator">•</span>{version}<span class="footer-separator">•</span>MIT Licensed</div>
    </div>
    {footer_script}
</body>
</html>"##,
        base_css = get_base_css(),
        relay_name = config.relay_name,
        path = path,
        version = get_version(),
        footer_script = get_footer_script(),
    )
}

/// Generate a 404 page for a non-existent repository
///
/// GRASP-01: "...and a 404 page for repositories it doesn't host"
pub fn get_404_html(config: &Config, npub: &str, identifier: &str) -> String {
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Repository Not Found - {relay_name}</title>
    <style>
        {base_css}
        body {{
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 24px;
        }}
        .container {{ max-width: 480px; text-align: center; }}
        .error-code {{
            font-size: 6rem;
            font-weight: 700;
            color: var(--error);
            line-height: 1;
            margin-bottom: 8px;
        }}
        h2 {{ font-size: 1.5rem; font-weight: 500; margin-bottom: 16px; }}
        p {{ color: var(--text-muted); margin-bottom: 24px; }}
        .repo-info {{
            background: var(--surface);
            border: 1px solid var(--border);
            border-radius: 8px;
            padding: 16px;
            margin-bottom: 16px;
            text-align: left;
        }}
        .info-row {{
            display: flex;
            justify-content: space-between;
            align-items: center;
            padding: 8px 0;
        }}
        .info-row + .info-row {{ border-top: 1px solid var(--border); }}
        .info-label {{ font-size: 0.875rem; color: var(--text-muted); }}
        code {{
            font-size: 0.75rem;
            word-break: break-all;
            max-width: 200px;
            overflow: hidden;
            text-overflow: ellipsis;
        }}
        .hint {{
            background: var(--surface);
            border: 1px solid var(--border);
            border-radius: 8px;
            padding: 16px;
            margin-bottom: 32px;
            font-size: 0.875rem;
            color: var(--text-muted);
        }}
        .footer {{ margin-top: 48px; }}
        .footer-separator {{ margin: 0 0.5em; opacity: 0.5; }}
    </style>
</head>
<body>
    <div class="container">
        <div class="error-code">404</div>
        <h2>Repository Not Found</h2>
        <p>This repository doesn't exist on this GRASP server.</p>
        <div class="repo-info">
            <div class="info-row">
                <span class="info-label">Owner</span>
                <code>{npub}</code>
            </div>
            <div class="info-row">
                <span class="info-label">Repository</span>
                <code>{identifier}</code>
            </div>
        </div>
        <div class="hint">The repository may not have been announced to this server, or the URL may be incorrect.</div>
        <a href="/">&larr; Back to {relay_name}</a>
        <div class="footer"><span id="footer-domain"></span><span class="footer-separator">•</span>powered by <a href="https://gitworkshop.dev/danconwaydev.com/ngit-grasp"><strong>ngit-grasp</strong></a><span class="footer-separator">•</span>{version}<span class="footer-separator">•</span>MIT Licensed</div>
    </div>
    {footer_script}
</body>
</html>"##,
        base_css = get_base_css(),
        relay_name = config.relay_name,
        npub = npub,
        identifier = identifier,
        version = get_version(),
        footer_script = get_footer_script(),
    )
}

/// Generate a webpage for an existing repository
///
/// GRASP-01: "SHOULD serve a webpage at the same endpoint linking to git nostr client(s)
/// to browse the repository"
pub fn get_repo_html(config: &Config, npub: &str, identifier: &str) -> String {
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{identifier} - {relay_name}</title>
    <style>
        {base_css}
        .container {{ max-width: 720px; margin: 0 auto; padding: 60px 24px; }}
        .back-link {{ margin-bottom: 32px; }}
        .header {{ margin-bottom: 8px; }}
        h1 {{ font-size: 1.75rem; font-weight: 600; letter-spacing: -0.02em; }}
        .subtitle {{ color: var(--text-muted); }}
        .section {{ margin-bottom: 32px; }}
        .section-title {{
            font-size: 0.75rem;
            font-weight: 600;
            text-transform: uppercase;
            letter-spacing: 0.1em;
            color: var(--text);
            margin-bottom: 12px;
        }}
        .card {{
            background: var(--surface);
            border: 1px solid var(--border);
            border-radius: 12px;
            padding: 16px 20px;
        }}
        .card + .card {{ margin-top: 8px; }}
        code {{ font-size: 0.8rem; word-break: break-all; }}
        .clone-box {{
            background: var(--bg);
            border: 1px solid var(--border);
            border-radius: 8px;
            padding: 16px;
            font-family: 'SF Mono', 'Consolas', monospace;
            font-size: 0.875rem;
            color: var(--text);
            overflow-x: auto;
        }}
        .clone-line {{ margin-bottom: 8px; }}
        .clone-line:last-child {{ margin-bottom: 0; }}
        .clone-box .cmd {{ color: var(--text-muted); }}
        .clone-box .url {{ color: var(--success); }}
        .browse-link {{
            display: inline-block;
            background: var(--brand);
            color: white;
            padding: 14px 24px;
            border-radius: 8px;
            font-weight: 500;
            font-size: 1rem;
            margin: 32px 0;
            transition: background 0.2s;
            text-align: center;
        }}
        .browse-link:hover {{
            background: var(--brand-light);
            text-decoration: none;
        }}
        .browse-link .browse-identifier {{
            display: block;
            font-size: 1.125rem;
            font-weight: 600;
        }}
        .browse-link .browse-site {{
            display: block;
            font-size: 0.875rem;
            opacity: 0.9;
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="back-link">
            <a href="/">&larr; {relay_name}</a>
        </div>
        <div class="header">
            <h1>{identifier}</h1>
            <h3 class="subtitle">by {npub}</h3>
        </div>
        <p class="subtitle">Git repository hosted using the <a href="https://ngit.dev/grasp">Grasp Protocol</a></p>
        
        <a id="gitworkshop-link" href="https://gitworkshop.dev" class="browse-link" target="_blank">
            <span class="browse-identifier">Browse Repository</span>
            <span class="browse-site">on GitWorkshop.dev &rarr;</span>
        </a>
        
        <div class="section">
            <div class="section-title">Clone</div>
            <div class="card">
                <div class="clone-box">
                    <div class="clone-line"><span class="cmd">curl -Ls https://ngit.dev/install.sh | bash</span></div>
                    <div class="clone-line"><span class="cmd">git clone</span> <span class="url" id="nostr-clone-url">nostr://{{npub}}/<span id="relayref"></span>/{{identifier}}</span></div>
                </div>
            </div>
        </div>
        <div class="footer"><span id="footer-domain"></span><span class="footer-separator">•</span>powered by <a href="https://gitworkshop.dev/danconwaydev.com/ngit-grasp"><strong>ngit-grasp</strong></a><span class="footer-separator">•</span>{version}<span class="footer-separator">•</span>MIT Licensed</div>
    </div>
    <script>
        // Detect protocol and construct relayref
        const protocol = window.location.protocol; // 'http:' or 'https:'
        const host = window.location.host; // 'domain.com' or 'domain.com:port'
        
        // For http, use ws:// prefix and URL encode; for https, just use host (implies wss://)
        let relayref = host;
        if (protocol === 'http:') relayref = encodeURIComponent("ws://" + host);
        
        // Update the relayref in the clone URL
        document.getElementById('relayref').textContent = relayref;
        
        // Construct gitworkshop link: gitworkshop.dev/npub/relayref/identifier
        const gitworkshopLink = document.getElementById('gitworkshop-link');
        gitworkshopLink.setAttribute('href', 'https://gitworkshop.dev/{npub}/' + relayref + '/{identifier}');
        
        // Set footer domain
        var footerDomain = document.getElementById('footer-domain');
        if (footerDomain) {{
            footerDomain.textContent = host;
        }}
    </script>
</body>
</html>"##,
        base_css = get_base_css(),
        relay_name = config.relay_name,
        npub = npub,
        identifier = identifier,
        version = get_version(),
    )
}
