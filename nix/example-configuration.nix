# Example NixOS configuration using ngit-grasp module
# 
# Usage:
#   1. Add to your server's flake.nix inputs:
#      inputs.ngit-grasp.url = "github:DanConwayDev/ngit-grasp";
#
#   2. Import the module in your configuration:
#      imports = [ inputs.ngit-grasp.nixosModules.default ];
#
#   3. Configure the service (example below)

{ inputs, ... }:

{
  imports = [ inputs.ngit-grasp.nixosModules.default ];

  services.ngit-grasp = {
    enable = true;
    domain = "ngit.danconwaydev.com";
    
    # Network
    bindAddress = "127.0.0.1";
    port = 8082;  # Same port as current ngit-relay for Caddy compatibility
    
    # Storage (reuse existing persistent path pattern)
    dataDir = "/persistent/ngit-danconwaydev-com-ngit-grasp";
    
    # Identity
    relayName = "DanConwayDev's ngit-grasp";
    relayDescription = "personal instance of ngit-grasp, a Rust GRASP implementation with proactive sync";
    
    # Option 1: Use nsec file (recommended - more secure)
    relayOwnerNsecFile = "/persistent/ngit-danconwaydev-com-ngit-grasp/relay-owner.nsec";
    
    # Option 2: Inline nsec (less secure, ends up in nix store)
    # relayOwnerNsec = "nsec1...";
    
    # Option 3: Auto-generate (default if neither above is set)
    # ngit-grasp will create .relay-owner.nsec in dataDir automatically
    
    # Sync
    syncBootstrapRelayUrl = "wss://relay.ngit.dev";
    
    # Metrics
    metricsEnabled = true;
    
    # Logging
    logLevel = "info";  # Options: trace, debug, info, warn, error
  };

  # Caddy reverse proxy (unchanged from current setup)
  services.caddy.virtualHosts."ngit.danconwaydev.com" = {
    extraConfig = ''
      reverse_proxy 127.0.0.1:8082 {
        header_down X-Real-IP {http.request.remote}
        header_down X-Forwarded-For {http.request.remote}
      }
    '';
  };
}
