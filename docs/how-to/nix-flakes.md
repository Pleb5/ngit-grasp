# How-To: Configure Nix Flakes for Development

**Purpose:** Set up and use Nix flakes for ngit-grasp development  
**Difficulty:** Intermediate  
**Time:** 10 minutes

---

## Problem

You want to:
- Set up a reproducible development environment
- Avoid "works on my machine" issues
- Use Nix flakes with ngit-grasp

---

## Prerequisites

- Nix installed (2.4 or later)
- Flakes enabled in your Nix configuration

---

## Solution

### Step 1: Enable Flakes (if not already enabled)

Check if flakes are enabled:

```bash
nix flake --version
```

If you get an error, enable flakes:

```bash
# Add to ~/.config/nix/nix.conf (create if doesn't exist)
mkdir -p ~/.config/nix
echo "experimental-features = nix-command flakes" >> ~/.config/nix/nix.conf

# Or for system-wide (requires sudo):
echo "experimental-features = nix-command flakes" | sudo tee -a /etc/nix/nix.conf
```

Restart the Nix daemon:

```bash
# On NixOS:
sudo systemctl restart nix-daemon

# On macOS:
sudo launchctl stop org.nixos.nix-daemon
sudo launchctl start org.nixos.nix-daemon

# On other Linux:
sudo pkill nix-daemon
```

---

### Step 2: Enter the Development Environment

```bash
cd ngit-grasp
nix develop
```

**What this does:**
- Reads `flake.nix` in the current directory
- Downloads and builds all dependencies
- Creates a shell with Rust, Git, and other tools
- Sets environment variables

**First run:** Will take several minutes to download and build  
**Subsequent runs:** Should be instant (cached)

---

### Step 3: Verify the Environment

```bash
# Check Rust is available
rustc --version
cargo --version

# Check Git is available
git --version

# Check you're in the Nix shell
echo $IN_NIX_SHELL  # Should output "impure"
```

---

### Step 4: Work with Subprojects

ngit-grasp has a subproject (`grasp-audit`) with its own flake:

```bash
# Main project
cd ngit-grasp
nix develop  # Uses ngit-grasp/flake.nix

# Subproject
cd grasp-audit
nix develop  # Uses grasp-audit/flake.nix
```

**Important:** Each directory has its own flake and environment!

---

## Common Tasks

### Build the Project

```bash
cd grasp-audit
nix develop
cargo build
```

**Or in one command:**

```bash
cd grasp-audit
nix develop -c cargo build
```

The `-c` flag runs a command in the Nix environment and exits.

---

### Run Tests

```bash
cd grasp-audit
nix develop -c cargo test
```

---

### Build Without Entering Shell

```bash
cd grasp-audit
nix build
```

This builds the package defined in `flake.nix` outputs.

---

### Update Dependencies

```bash
# Update flake.lock (updates all inputs)
nix flake update

# Update specific input
nix flake lock --update-input nixpkgs
```

**When to update:**
- Security vulnerabilities in dependencies
- Need newer version of Rust or other tools
- Monthly maintenance

---

### Clean Nix Store

```bash
# Remove unused packages
nix-collect-garbage

# Aggressive cleanup (removes all old generations)
nix-collect-garbage -d
```

**Warning:** This will remove all old versions. You'll need to re-download if you switch branches.

---

## Troubleshooting

### "nix: command not found"

**Problem:** Nix is not installed or not in PATH

**Solution:**
```bash
# Install Nix (official installer)
sh <(curl -L https://nixos.org/nix/install) --daemon

# Add to PATH (if needed)
source ~/.nix-profile/etc/profile.d/nix.sh
```

---

### "experimental features not enabled"

**Problem:** Flakes are not enabled

**Solution:**
```bash
# Add to ~/.config/nix/nix.conf
echo "experimental-features = nix-command flakes" >> ~/.config/nix/nix.conf

# Restart Nix daemon (see Step 1)
```

---

### "nix-shell: command not found" or wrong behavior

**Problem:** Using old `nix-shell` command instead of `nix develop`

**Solution:**
```bash
# ❌ Wrong (old Nix)
nix-shell

# ✅ Correct (Nix flakes)
nix develop
```

**Why:** Flakes use `nix develop`, not `nix-shell`. The old command looks for `shell.nix` which doesn't exist.

---

### "error: getting status of '/nix/store/...': No such file or directory"

**Problem:** Nix store is corrupted or incomplete

**Solution:**
```bash
# Verify Nix store
nix-store --verify --check-contents

# Repair if needed
nix-store --repair --verify --check-contents

# If still broken, re-enter environment
nix develop --refresh
```

---

### Build fails with "cannot find crate"

**Problem:** Cargo cache is stale or corrupted

**Solution:**
```bash
# Clean Cargo cache
cargo clean

# Rebuild
nix develop -c cargo build
```

---

### "error: unable to download"

**Problem:** Network issues or cache server down

**Solution:**
```bash
# Use different substituter
nix develop --option substituters "https://cache.nixos.org"

# Or build from source (slow)
nix develop --no-substitutes
```

---

## Advanced Usage

### Use direnv for Automatic Activation

Install [direnv](https://direnv.net/) to automatically enter Nix environment:

```bash
# Install direnv
nix-env -iA nixpkgs.direnv

# Create .envrc
echo "use flake" > .envrc

# Allow direnv
direnv allow

# Now cd into directory automatically activates environment!
cd ngit-grasp  # Automatically runs 'nix develop'
```

---

### Customize the Environment

Edit `flake.nix` to add packages:

```nix
{
  devShells.default = pkgs.mkShell {
    buildInputs = with pkgs; [
      # Existing packages
      cargo
      rustc
      
      # Add your packages here
      jq        # JSON processor
      ripgrep   # Fast grep
      fd        # Fast find
    ];
  };
}
```

Then reload:

```bash
nix develop --refresh
```

---

### Pin to Specific Rust Version

Edit `flake.nix`:

```nix
{
  inputs.rust-overlay.url = "github:oxalica/rust-overlay";
  
  outputs = { self, nixpkgs, rust-overlay }:
    let
      pkgs = import nixpkgs {
        overlays = [ rust-overlay.overlays.default ];
      };
      
      # Pin to specific version
      rust = pkgs.rust-bin.stable."1.75.0".default;
    in {
      devShells.default = pkgs.mkShell {
        buildInputs = [ rust ];
      };
    };
}
```

---

## Best Practices

### DO:
- ✅ Use `nix develop` for flakes (not `nix-shell`)
- ✅ Commit `flake.lock` to version control
- ✅ Update flakes monthly
- ✅ Use `-c` flag for one-off commands
- ✅ Use direnv for automatic activation

### DON'T:
- ❌ Use `nix-shell` with flakes
- ❌ Manually edit `flake.lock`
- ❌ Ignore flake update warnings
- ❌ Mix Nix and non-Nix environments
- ❌ Commit `.direnv/` to git

---

## Quick Reference

```bash
# Enter environment
nix develop

# Run command in environment
nix develop -c cargo build

# Build package
nix build

# Update dependencies
nix flake update

# Show flake info
nix flake show

# Check flake
nix flake check

# Clean up
nix-collect-garbage
```

---

## Related Documentation

- [Getting Started Tutorial](../tutorials/getting-started.md) - First-time setup
- [Nix Flakes Manual](https://nixos.org/manual/nix/stable/command-ref/new-cli/nix3-flake.html)
- [grasp-audit README](../../grasp-audit/README.md) - Subproject docs

---

*Part of the [ngit-grasp how-to guides](./)*
