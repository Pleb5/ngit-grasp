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

/// Generate a generic 404 page for unknown paths
///
/// Used for any path that doesn't match a known route
pub fn get_generic_404_html(config: &Config, path: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Not Found - {relay_name}</title>
    <style>
        * {{
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', 'Roboto', 'Oxygen', 'Ubuntu', 'Cantarell', sans-serif;
            line-height: 1.6;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: #333;
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 20px;
        }}
        .container {{
            max-width: 600px;
            background: white;
            padding: 40px;
            border-radius: 12px;
            box-shadow: 0 20px 60px rgba(0,0,0,0.3);
            text-align: center;
        }}
        h1 {{
            color: #e74c3c;
            margin-bottom: 10px;
            font-size: 4em;
        }}
        h2 {{
            color: #333;
            margin-bottom: 20px;
            font-size: 1.5em;
        }}
        .path-info {{
            background: #f9f9f9;
            padding: 15px;
            border-radius: 8px;
            margin: 20px 0;
            border-left: 4px solid #e74c3c;
        }}
        code {{
            background: #f4f4f4;
            padding: 3px 8px;
            border-radius: 4px;
            font-family: 'Courier New', monospace;
            color: #667eea;
            font-size: 0.85em;
            word-break: break-all;
        }}
        .back-link {{
            margin-top: 20px;
        }}
        a {{
            color: #667eea;
            text-decoration: none;
        }}
        a:hover {{
            text-decoration: underline;
        }}
        .footer {{
            margin-top: 30px;
            padding-top: 20px;
            border-top: 1px solid #eee;
            color: #999;
            font-size: 0.9em;
        }}
    </style>
</head>
<body>
    <div class="container">
        <h1>404</h1>
        <h2>Not Found</h2>
        <p>The page you're looking for doesn't exist.</p>
        
        <div class="path-info">
            <p><strong>Requested path:</strong> <code>{path}</code></p>
        </div>
        
        <div class="back-link">
            <a href="/">← Back to {relay_name}</a>
        </div>
        
        <div class="footer">
            <p>Powered by <strong>ngit-grasp</strong></p>
        </div>
    </div>
</body>
</html>"#,
        relay_name = config.relay_name,
        path = path,
    )
}

/// Generate a 404 page for a non-existent repository
///
/// GRASP-01: "...and a 404 page for repositories it doesn't host"
pub fn get_404_html(config: &Config, npub: &str, identifier: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Repository Not Found - {relay_name}</title>
    <style>
        * {{
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', 'Roboto', 'Oxygen', 'Ubuntu', 'Cantarell', sans-serif;
            line-height: 1.6;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: #333;
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 20px;
        }}
        .container {{
            max-width: 600px;
            background: white;
            padding: 40px;
            border-radius: 12px;
            box-shadow: 0 20px 60px rgba(0,0,0,0.3);
            text-align: center;
        }}
        h1 {{
            color: #e74c3c;
            margin-bottom: 10px;
            font-size: 4em;
        }}
        h2 {{
            color: #333;
            margin-bottom: 20px;
            font-size: 1.5em;
        }}
        .repo-info {{
            background: #f9f9f9;
            padding: 15px;
            border-radius: 8px;
            margin: 20px 0;
            border-left: 4px solid #e74c3c;
        }}
        code {{
            background: #f4f4f4;
            padding: 3px 8px;
            border-radius: 4px;
            font-family: 'Courier New', monospace;
            color: #667eea;
            font-size: 0.85em;
            word-break: break-all;
        }}
        .back-link {{
            margin-top: 20px;
        }}
        a {{
            color: #667eea;
            text-decoration: none;
        }}
        a:hover {{
            text-decoration: underline;
        }}
        .footer {{
            margin-top: 30px;
            padding-top: 20px;
            border-top: 1px solid #eee;
            color: #999;
            font-size: 0.9em;
        }}
    </style>
</head>
<body>
    <div class="container">
        <h1>404</h1>
        <h2>Repository Not Found</h2>
        <p>The repository you're looking for doesn't exist on this GRASP server.</p>
        
        <div class="repo-info">
            <p><strong>Owner:</strong> <code>{npub}</code></p>
            <p><strong>Repository:</strong> <code>{identifier}</code></p>
        </div>
        
        <p>This repository may not have been announced to this server, or the URL may be incorrect.</p>
        
        <div class="back-link">
            <a href="/">← Back to {relay_name}</a>
        </div>
        
        <div class="footer">
            <p>Powered by <strong>ngit-grasp</strong></p>
        </div>
    </div>
</body>
</html>"#,
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
    let clone_url = format!(
        "http://{}/{}/{}.git",
        config.domain, npub, identifier
    );
    
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{identifier} - {relay_name}</title>
    <style>
        * {{
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', 'Roboto', 'Oxygen', 'Ubuntu', 'Cantarell', sans-serif;
            line-height: 1.6;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: #333;
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 20px;
        }}
        .container {{
            max-width: 800px;
            background: white;
            padding: 40px;
            border-radius: 12px;
            box-shadow: 0 20px 60px rgba(0,0,0,0.3);
        }}
        h1 {{
            color: #667eea;
            margin-bottom: 10px;
            font-size: 2em;
        }}
        h2 {{
            color: #764ba2;
            margin-top: 25px;
            margin-bottom: 15px;
            font-size: 1.3em;
            border-bottom: 2px solid #667eea;
            padding-bottom: 8px;
        }}
        .subtitle {{
            color: #666;
            margin-bottom: 25px;
            font-size: 1em;
        }}
        .repo-info {{
            background: #f9f9f9;
            padding: 15px;
            border-radius: 8px;
            margin: 15px 0;
            border-left: 4px solid #667eea;
        }}
        code {{
            background: #f4f4f4;
            padding: 3px 8px;
            border-radius: 4px;
            font-family: 'Courier New', monospace;
            color: #667eea;
            font-size: 0.85em;
            word-break: break-all;
        }}
        .clone-box {{
            background: #2d3748;
            color: #e2e8f0;
            padding: 15px;
            border-radius: 8px;
            margin: 15px 0;
            font-family: 'Courier New', monospace;
            font-size: 0.9em;
            overflow-x: auto;
        }}
        .clone-box code {{
            background: transparent;
            color: #68d391;
            padding: 0;
        }}
        ul {{
            margin: 15px 0;
            padding-left: 25px;
        }}
        li {{
            margin: 10px 0;
        }}
        a {{
            color: #667eea;
            text-decoration: none;
        }}
        a:hover {{
            text-decoration: underline;
        }}
        .client-list {{
            display: grid;
            gap: 10px;
            margin: 15px 0;
        }}
        .client-item {{
            background: #f9f9f9;
            padding: 12px 15px;
            border-radius: 8px;
            display: flex;
            justify-content: space-between;
            align-items: center;
        }}
        .badge {{
            display: inline-block;
            background: #667eea;
            color: white;
            padding: 4px 10px;
            border-radius: 12px;
            font-size: 0.8em;
        }}
        .footer {{
            margin-top: 30px;
            padding-top: 20px;
            border-top: 1px solid #eee;
            text-align: center;
            color: #999;
            font-size: 0.9em;
        }}
        .back-link {{
            margin-bottom: 20px;
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="back-link">
            <a href="/">← Back to {relay_name}</a>
        </div>
        
        <h1>📦 {identifier}</h1>
        <p class="subtitle">Git repository hosted on {relay_name}</p>
        
        <h2>📋 Repository Information</h2>
        <div class="repo-info">
            <p><strong>Owner:</strong> <code>{npub}</code></p>
            <p><strong>Repository:</strong> <code>{identifier}</code></p>
        </div>
        
        <h2>🔗 Clone this Repository</h2>
        <div class="clone-box">
            git clone <code>{clone_url}</code>
        </div>
        
        <h2>🌐 Browse with Git Nostr Clients</h2>
        <p>You can browse this repository using these Git Nostr clients:</p>
        <div class="client-list">
            <div class="client-item">
                <span><strong>gitworkshop.dev</strong> - Web-based repository browser</span>
                <a href="https://gitworkshop.dev" target="_blank">Visit →</a>
            </div>
            <div class="client-item">
                <span><strong>ngit</strong> - Command-line Git + Nostr tool</span>
                <a href="https://github.com/DanConwayDev/ngit-cli" target="_blank">GitHub →</a>
            </div>
        </div>
        
        <h2>📚 About GRASP</h2>
        <p>This repository is hosted using the <strong>GRASP</strong> (Git Relays Authorized via Signed-Nostr Proofs) protocol.</p>
        <ul>
            <li><a href="https://gitworkshop.dev/repo/grasp/01.md" target="_blank">GRASP-01 Specification</a></li>
            <li><a href="https://github.com/nostr-protocol/nips/blob/master/34.md" target="_blank">NIP-34: Git Stuff</a></li>
        </ul>
        
        <div class="footer">
            <p>Powered by <strong>ngit-grasp</strong></p>
        </div>
    </div>
</body>
</html>"#,
        relay_name = config.relay_name,
        npub = npub,
        identifier = identifier,
        clone_url = clone_url,
    )
}
