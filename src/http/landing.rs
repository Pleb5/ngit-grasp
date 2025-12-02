/// Landing Page Handler
///
/// Generates HTML landing page for the Nostr relay.
use crate::config::Config;

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
        }"#
}

/// Generate the HTML landing page
pub fn get_html(config: &Config) -> String {
    format!(
        include_str!("../../templates/landing.html"),
        base_css = get_base_css(),
        relay_name = config.relay_name,
        relay_description = config.relay_description,
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
        <div class="footer">Powered by <strong>ngit-grasp</strong></div>
    </div>
</body>
</html>"##,
        base_css = get_base_css(),
        relay_name = config.relay_name,
        path = path,
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
        <div class="footer">Powered by <strong>ngit-grasp</strong></div>
    </div>
</body>
</html>"##,
        base_css = get_base_css(),
        relay_name = config.relay_name,
        npub = npub,
        identifier = identifier,
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
        <p class="subtitle">Git repository hosted on {relay_name}</p>
        
        <a id="gitworkshop-link" href="https://gitworkshop.dev" class="browse-link" target="_blank">
            <span class="browse-identifier">Browse Repository</span>
            <span class="browse-site">on GitWorkshop.dev &rarr;</span>
        </a>
        
        <div class="section">
            <div class="section-title">Clone</div>
            <div class="card">
                <div class="clone-box">
                    <div class="clone-line"><span class="cmd">curl -Ls https://ngit.dev/install.sh | bash</span></div>
                    <div class="clone-line"><span class="cmd">git clone</span> <span class="url" id="nostr-clone-url">nostr://{npub}/<span id="relayref"></span>/{identifier}</span></div>
                </div>
            </div>
        </div>
        <div class="footer">Powered by <strong>ngit-grasp</strong></div>
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
    </script>
</body>
</html>"##,
        base_css = get_base_css(),
        relay_name = config.relay_name,
        npub = npub,
        identifier = identifier,
    )
}
