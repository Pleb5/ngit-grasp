{ config, lib, pkgs, ... }:

with lib;

let
  # Build ngit-grasp package (shared across all instances)
  ngit-grasp = pkgs.rustPlatform.buildRustPackage {
    pname = "ngit-grasp";
    version = "0.1.0";
    src = ../.;
    cargoLock = {
      lockFile = ../Cargo.lock;
      outputHashes = {
        "nostr-0.44.1" = "sha256-DwcWmwxNUQRR32E3hqbm7PNkGdK8LB3sGtH1Zfrkigk=";
      };
    };

    nativeBuildInputs = with pkgs; [ pkg-config ];
    buildInputs = with pkgs; [ openssl ];

    # Disable tests during Nix build (many require git in PATH for sandboxing)
    # Tests run successfully in dev environment and CI where git is available
    doCheck = false;
  };

  # Per-instance options
  instanceOptions = { name, ... }: {
    options = {
      enable = mkEnableOption "this ngit-grasp instance";

      domain = mkOption {
        type = types.str;
        example = "ngit.example.com";
        description =
          "Domain where this relay is hosted (used in GRASP validation)";
      };

      bindAddress = mkOption {
        type = types.str;
        default = "127.0.0.1";
        description = "IP address to bind to";
      };

      port = mkOption {
        type = types.port;
        default = 8080;
        description = "Port to listen on";
      };

      dataDir = mkOption {
        type = types.path;
        default = "/var/lib/ngit-grasp-${name}";
        description = "Base directory for data storage";
      };

      relayName = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "My GRASP Relay";
        description =
          "Relay name for NIP-11 (defaults to \${domain} grasp relay)";
      };

      relayDescription = mkOption {
        type = types.str;
        default = "Git Nostr Relay - a grasp implementation";
        description = "Relay description for NIP-11";
      };

      relayOwnerNsecFile = mkOption {
        type = types.nullOr types.path;
        default = null;
        example = "/persistent/ngit-grasp/relay-owner.nsec";
        description = ''
          Path to file containing relay owner's nsec (private key).
          If file doesn't exist, ngit-grasp will auto-generate a random nsec and save it.
          Takes precedence over relayOwnerNsec if both are set.
        '';
      };

      relayOwnerNsec = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "nsec1...";
        description = ''
          Relay owner's nsec (private key) for signing and authentication.
          Less secure than relayOwnerNsecFile as it ends up in nix store.
          Only used if relayOwnerNsecFile is not set.
        '';
      };

      syncBootstrapRelayUrl = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "wss://relay.ngit.dev";
        description = "Bootstrap relay URL to sync from on startup (optional)";
      };

      databaseBackend = mkOption {
        type = types.enum [ "lmdb" "nostr-db" "memory" ];
        default = "lmdb";
        description = ''
          Database backend type:
          - lmdb: LMDB backend (persistent, general purpose)
          - nostr-db: NostrDB backend (persistent, optimized for Nostr)
          - memory: In-memory database (fastest, no persistence)
        '';
      };

      metricsEnabled = mkOption {
        type = types.bool;
        default = true;
        description = "Enable Prometheus metrics endpoint at /metrics";
      };

      metricsConnectionPerIpAbuseThreshold = mkOption {
        type = types.int;
        default = 10;
        description =
          "Connections per IP before flagging as potential abuse in metrics";
      };

      metricsTopNRepos = mkOption {
        type = types.int;
        default = 10;
        description = "Number of top bandwidth repos to track in metrics";
      };

      logLevel = mkOption {
        type = types.enum [ "trace" "debug" "info" "warn" "error" ];
        default = "info";
        description = "Logging level for RUST_LOG environment variable";
      };

      syncMaxBackoffSecs = mkOption {
        type = types.int;
        default = 3600;
        description =
          "Maximum backoff time in seconds for sync relay reconnection (default: 1 hour)";
      };

      syncDisconnectCheckIntervalSecs = mkOption {
        type = types.int;
        default = 60;
        description = "Interval in seconds for checking disconnected relays";
      };

      syncBaseBackoffSecs = mkOption {
        type = types.int;
        default = 5;
        description = "Base backoff time in seconds for relay reconnection";
      };

      syncDisableNegentropy = mkOption {
        type = types.bool;
        default = false;
        description = "Disable NIP-77 negentropy sync (use REQ+EOSE instead)";
      };

      rejectedHotCacheDurationSecs = mkOption {
        type = types.int;
        default = 120;
        description =
          "Hot cache duration in seconds for rejected announcements (default: 2 minutes)";
      };

      rejectedColdIndexExpirySecs = mkOption {
        type = types.int;
        default = 604800;
        description =
          "Cold index expiry in seconds for rejected announcements (default: 7 days)";
      };

      naughtyListExpirationHours = mkOption {
        type = types.int;
        default = 12;
        description = "Hours before removing relay from naughty list";
      };

      user = mkOption {
        type = types.str;
        default = "ngit-grasp-${name}";
        description = "User account under which this instance runs";
      };

      group = mkOption {
        type = types.str;
        default = "ngit-grasp";
        description = "Group under which this instance runs";
      };
    };
  };

  # Create systemd service config for an instance
  mkService = name: cfg: {
    description = "ngit-grasp GRASP relay (${name})";
    after = [ "network.target" ];
    wantedBy = [ "multi-user.target" ];

    environment = {
      NGIT_DOMAIN = cfg.domain;
      NGIT_BIND_ADDRESS = "${cfg.bindAddress}:${toString cfg.port}";
      NGIT_GIT_DATA_PATH = "${cfg.dataDir}/git";
      NGIT_RELAY_DATA_PATH = "${cfg.dataDir}/relay";
      NGIT_RELAY_DESCRIPTION = cfg.relayDescription;
      NGIT_DATABASE_BACKEND = cfg.databaseBackend;
      NGIT_METRICS_CONNECTION_PER_IP_ABUSE_THRESHOLD =
        toString cfg.metricsConnectionPerIpAbuseThreshold;
      NGIT_METRICS_TOP_N_REPOS = toString cfg.metricsTopNRepos;
      NGIT_SYNC_MAX_BACKOFF_SECS = toString cfg.syncMaxBackoffSecs;
      NGIT_SYNC_DISCONNECT_CHECK_INTERVAL_SECS =
        toString cfg.syncDisconnectCheckIntervalSecs;
      NGIT_SYNC_BASE_BACKOFF_SECS = toString cfg.syncBaseBackoffSecs;
      NGIT_REJECTED_HOT_CACHE_DURATION_SECS =
        toString cfg.rejectedHotCacheDurationSecs;
      NGIT_REJECTED_COLD_INDEX_EXPIRY_SECS =
        toString cfg.rejectedColdIndexExpirySecs;
      NGIT_NAUGHTY_LIST_EXPIRATION_HOURS =
        toString cfg.naughtyListExpirationHours;
      RUST_LOG = cfg.logLevel;
    } // optionalAttrs (cfg.relayName != null) {
      NGIT_RELAY_NAME = cfg.relayName;
    } // optionalAttrs cfg.metricsEnabled { NGIT_METRICS_ENABLED = "true"; }
      // optionalAttrs (cfg.syncBootstrapRelayUrl != null) {
        NGIT_SYNC_BOOTSTRAP_RELAY_URL = cfg.syncBootstrapRelayUrl;
      } // optionalAttrs cfg.syncDisableNegentropy {
        NGIT_SYNC_DISABLE_NEGENTROPY = "true";
      } // optionalAttrs
      (cfg.relayOwnerNsec != null && cfg.relayOwnerNsecFile == null) {
        # Only set inline nsec if file is not specified
        NGIT_RELAY_OWNER_NSEC = cfg.relayOwnerNsec;
      };

    serviceConfig = {
      Type = "simple";
      User = cfg.user;
      Group = cfg.group;

      # Working directory where .relay-owner.nsec will be created if needed
      WorkingDirectory = cfg.dataDir;

      # Command to run
      ExecStart = if cfg.relayOwnerNsecFile != null then
      # Use nsec from file
        "${ngit-grasp}/bin/ngit-grasp --relay-owner-nsec $(cat ${cfg.relayOwnerNsecFile})"
      else
      # Let ngit-grasp auto-generate nsec in .relay-owner.nsec file in dataDir
        "${ngit-grasp}/bin/ngit-grasp";

      # Restart policy
      Restart = "always";
      RestartSec = "10s";

      # Hardening
      NoNewPrivileges = true;
      PrivateTmp = true;
      ProtectSystem = "strict";
      ProtectHome = true;
      ReadWritePaths = [ cfg.dataDir ];

      # If using nsecFile, grant read access
      ReadOnlyPaths =
        optionals (cfg.relayOwnerNsecFile != null) [ cfg.relayOwnerNsecFile ];

      # Additional hardening
      ProtectKernelTunables = true;
      ProtectKernelModules = true;
      ProtectControlGroups = true;
      RestrictAddressFamilies = [ "AF_INET" "AF_INET6" "AF_UNIX" ];
      RestrictNamespaces = true;
      LockPersonality = true;
      RestrictRealtime = true;
      RestrictSUIDSGID = true;
      PrivateDevices = true;

      # Capabilities
      CapabilityBoundingSet = "";
      AmbientCapabilities = "";

      # System call filtering
      SystemCallFilter = [ "@system-service" "~@privileged" "~@resources" ];
      SystemCallErrorNumber = "EPERM";
    };

    # Directory creation handled by systemd tmpfiles (see config section below)
  };

  enabledInstances =
    filterAttrs (_: cfg: cfg.enable) config.services.ngit-grasp;

in {
  options.services.ngit-grasp = mkOption {
    type = types.attrsOf (types.submodule instanceOptions);
    default = { };
    description = ''
      ngit-grasp GRASP relay instances.

      Multiple instances can be configured with different domains and ports.
      Each instance runs as a separate systemd service.
    '';
    example = literalExpression ''
      {
        production = {
          enable = true;
          domain = "ngit.example.com";
          port = 8082;
          dataDir = "/persistent/ngit-production";
        };
        
        testing = {
          enable = true;
          domain = "ngit-test.example.com";
          port = 8083;
          dataDir = "/persistent/ngit-testing";
        };
      }
    '';
  };

  config = mkIf (enabledInstances != { }) {
    # Create users for all enabled instances
    users.users = mapAttrs' (name: cfg:
      nameValuePair cfg.user {
        isSystemUser = true;
        group = cfg.group;
        description = "ngit-grasp service user (${name})";
        home = cfg.dataDir;
      }) enabledInstances;

    # Create shared group (all instances use the same group by default)
    users.groups.ngit-grasp = { };

    # Create systemd services for all enabled instances
    systemd.services = mapAttrs'
      (name: cfg: nameValuePair "ngit-grasp-${name}" (mkService name cfg))
      enabledInstances;

    # Create data directories with proper ownership using tmpfiles
    # This runs as root before the service starts
    systemd.tmpfiles.rules = flatten (mapAttrsToList (name: cfg: [
      "d ${cfg.dataDir} 0750 ${cfg.user} ${cfg.group} -"
      "d ${cfg.dataDir}/git 0750 ${cfg.user} ${cfg.group} -"
      "d ${cfg.dataDir}/relay 0750 ${cfg.user} ${cfg.group} -"
    ]) enabledInstances);
  };
}
