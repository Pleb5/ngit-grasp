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

/// Generate the theme toggle HTML button
fn get_theme_toggle_html() -> &'static str {
    r##"<button class="theme-toggle" id="theme-toggle" aria-label="Toggle theme" title="Toggle light/dark mode">
        <svg class="sun-icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
            <path d="M12 7c-2.76 0-5 2.24-5 5s2.24 5 5 5 5-2.24 5-5-2.24-5-5-5zM2 13h2c.55 0 1-.45 1-1s-.45-1-1-1H2c-.55 0-1 .45-1 1s.45 1 1 1zm18 0h2c.55 0 1-.45 1-1s-.45-1-1-1h-2c-.55 0-1 .45-1 1s.45 1 1 1zM11 2v2c0 .55.45 1 1 1s1-.45 1-1V2c0-.55-.45-1-1-1s-1 .45-1 1zm0 18v2c0 .55.45 1 1 1s1-.45 1-1v-2c0-.55-.45-1-1-1s-1 .45-1 1zM5.99 4.58a.996.996 0 00-1.41 0 .996.996 0 000 1.41l1.06 1.06c.39.39 1.03.39 1.41 0s.39-1.03 0-1.41L5.99 4.58zm12.37 12.37a.996.996 0 00-1.41 0 .996.996 0 000 1.41l1.06 1.06c.39.39 1.03.39 1.41 0a.996.996 0 000-1.41l-1.06-1.06zm1.06-10.96a.996.996 0 000-1.41.996.996 0 00-1.41 0l-1.06 1.06c-.39.39-.39 1.03 0 1.41s1.03.39 1.41 0l1.06-1.06zM7.05 18.36a.996.996 0 000-1.41.996.996 0 00-1.41 0l-1.06 1.06c-.39.39-.39 1.03 0 1.41s1.03.39 1.41 0l1.06-1.06z"/>
        </svg>
        <svg class="moon-icon" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
            <path d="M12 3a9 9 0 109 9c0-.46-.04-.92-.1-1.36a5.389 5.389 0 01-4.4 2.26 5.403 5.403 0 01-3.14-9.8c-.44-.06-.9-.1-1.36-.1z"/>
        </svg>
    </button>"##
}

/// Generate the theme toggle JavaScript
fn get_theme_script() -> &'static str {
    r#"<script>
        (function() {
            const THEME_KEY = 'grasp-theme';
            const toggle = document.getElementById('theme-toggle');
            
            // Get saved theme or null (use system preference)
            function getSavedTheme() {
                try {
                    return localStorage.getItem(THEME_KEY);
                } catch (e) {
                    return null;
                }
            }
            
            // Save theme preference
            function saveTheme(theme) {
                try {
                    if (theme) {
                        localStorage.setItem(THEME_KEY, theme);
                    } else {
                        localStorage.removeItem(THEME_KEY);
                    }
                } catch (e) {}
            }
            
            // Get current effective theme
            function getCurrentTheme() {
                const saved = getSavedTheme();
                if (saved) return saved;
                return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
            }
            
            // Apply theme to document
            function applyTheme(theme) {
                if (theme) {
                    document.documentElement.setAttribute('data-theme', theme);
                } else {
                    document.documentElement.removeAttribute('data-theme');
                }
            }
            
            // Initialize theme on page load
            const savedTheme = getSavedTheme();
            if (savedTheme) {
                applyTheme(savedTheme);
            }
            
            // Toggle theme on button click
            if (toggle) {
                toggle.addEventListener('click', function() {
                    const current = getCurrentTheme();
                    const newTheme = current === 'dark' ? 'light' : 'dark';
                    applyTheme(newTheme);
                    saveTheme(newTheme);
                });
            }
            
            // Listen for system theme changes
            window.matchMedia('(prefers-color-scheme: light)').addEventListener('change', function(e) {
                // Only react if no manual preference is set
                if (!getSavedTheme()) {
                    // Theme will auto-update via CSS, no JS needed
                }
            });
        })();
    </script>"#
}

/// Generate the common base CSS used across all pages
fn get_base_css() -> &'static str {
    r#"/* Dark mode (default) */
        :root {
            --brand: #4434FF;
            --brand-light: #6b5fff;
            --bg: #0a0a0f;
            --surface: #12121a;
            --border: #1e1e2e;
            --text: #e4e4eb;
            --text-muted: #a8a8bd;
            --error: #ff4444;
            --success: #22c55e;
            --logo-bg: #4434FF;
            --logo-icon: white;
        }
        /* Light mode - system preference */
        @media (prefers-color-scheme: light) {
            :root:not([data-theme="dark"]) {
                --brand: #4434FF;
                --brand-light: #3525cc;
                --bg: #f8f9fa;
                --surface: #ffffff;
                --border: #e1e4e8;
                --text: #1a1a2e;
                --text-muted: #586069;
                --error: #dc3545;
                --success: #28a745;
                --logo-bg: #4434FF;
                --logo-icon: white;
            }
        }
        /* Manual light mode override */
        :root[data-theme="light"] {
            --brand: #4434FF;
            --brand-light: #3525cc;
            --bg: #f8f9fa;
            --surface: #ffffff;
            --border: #e1e4e8;
            --text: #1a1a2e;
            --text-muted: #586069;
            --error: #dc3545;
            --success: #28a745;
            --logo-bg: #4434FF;
            --logo-icon: white;
        }
        /* Manual dark mode override */
        :root[data-theme="dark"] {
            --brand: #4434FF;
            --brand-light: #6b5fff;
            --bg: #0a0a0f;
            --surface: #12121a;
            --border: #1e1e2e;
            --text: #e4e4eb;
            --text-muted: #a8a8bd;
            --error: #ff4444;
            --success: #22c55e;
            --logo-bg: #4434FF;
            --logo-icon: white;
        }
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', 'Inter', sans-serif;
            line-height: 1.6;
            background: var(--bg);
            color: var(--text);
            min-height: 100vh;
            transition: background-color 0.3s ease, color 0.3s ease;
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
        /* Theme toggle button */
        .theme-toggle {
            position: fixed;
            top: 16px;
            right: 16px;
            z-index: 1000;
            background: var(--surface);
            border: 1px solid var(--border);
            border-radius: 50%;
            width: 44px;
            height: 44px;
            cursor: pointer;
            display: flex;
            align-items: center;
            justify-content: center;
            transition: all 0.3s ease;
            box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
        }
        .theme-toggle:hover {
            transform: scale(1.1);
            box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
        }
        .theme-toggle svg {
            width: 20px;
            height: 20px;
            fill: var(--text);
            transition: fill 0.3s ease;
        }
        .theme-toggle .sun-icon { display: none; }
        .theme-toggle .moon-icon { display: block; }
        :root[data-theme="light"] .theme-toggle .sun-icon,
        :root:not([data-theme="dark"]) .theme-toggle .sun-icon { display: block; }
        :root[data-theme="light"] .theme-toggle .moon-icon,
        :root:not([data-theme="dark"]) .theme-toggle .moon-icon { display: none; }
        @media (prefers-color-scheme: dark) {
            :root:not([data-theme="light"]) .theme-toggle .sun-icon { display: none; }
            :root:not([data-theme="light"]) .theme-toggle .moon-icon { display: block; }
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
        .software-logo rect {
            fill: var(--logo-bg);
        }
        .software-logo path {
            fill: var(--logo-icon);
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
        theme_toggle = get_theme_toggle_html(),
        theme_script = get_theme_script(),
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
    {theme_toggle}
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
    {theme_script}
</body>
</html>"##,
        base_css = get_base_css(),
        relay_name = config.relay_name,
        path = path,
        version = get_version(),
        footer_script = get_footer_script(),
        theme_toggle = get_theme_toggle_html(),
        theme_script = get_theme_script(),
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
    {theme_toggle}
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
    {theme_script}
</body>
</html>"##,
        base_css = get_base_css(),
        relay_name = config.relay_name,
        npub = npub,
        identifier = identifier,
        version = get_version(),
        footer_script = get_footer_script(),
        theme_toggle = get_theme_toggle_html(),
        theme_script = get_theme_script(),
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
    {theme_toggle}
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
    {theme_script}
</body>
</html>"##,
        base_css = get_base_css(),
        relay_name = config.relay_name,
        npub = npub,
        identifier = identifier,
        version = get_version(),
        theme_toggle = get_theme_toggle_html(),
        theme_script = get_theme_script(),
    )
}
