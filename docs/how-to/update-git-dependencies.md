# How to Update Git Dependencies in Cargo

ngit-grasp uses git dependencies (such as our fork of nostr-sdk) instead of crates.io releases. When updating any git dependency, you need to update the hash in both the Nix build files.

## Prerequisites

- Access to update `Cargo.toml` and `Cargo.lock`
- Nix installed locally

## Steps

### 1. Update Cargo.toml and Cargo.lock

Update the git dependency in `Cargo.toml` (change the git URL, branch, rev, or tag):

```toml
[dependencies]
some-crate = { git = "https://github.com/user/repo", branch = "main" }
```

Then update `Cargo.lock`:

```bash
cargo update -p some-crate
```

### 2. Try Building with Nix

```bash
nix build .#default
```

This will **fail** with an error like:

```
error: hash mismatch in fixed-output derivation '/nix/store/...-some-crate-0.1.0':
  specified: sha256-DwcWmwxNUQRR32E3hqbm7PNkGdK8LB3sGtH1Zfrkigk=
  got:      sha256-XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX=
```

### 3. Copy the Correct Hash

The hash in the "got:" line is the **correct** hash you need. It's already in SRI format (base64 with `sha256-` prefix).

### 4. Update Both Nix Files

You must update the hash in **BOTH** files:

#### `flake.nix`:
```nix
cargoLock = {
  lockFile = ./Cargo.lock;
  outputHashes = {
    "some-crate-0.1.0" = "sha256-XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX=";  # ← Update this
  };
};
```

#### `nix/module.nix`:
```nix
cargoLock = {
  lockFile = ../Cargo.lock;
  outputHashes = {
    "some-crate-0.1.0" = "sha256-XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX=";  # ← Update this
  };
};
```

**⚠️ CRITICAL:** Both hashes must match exactly!

**Note:** The key format is `"crate-name-version"` as it appears in `Cargo.lock`. Check your `Cargo.lock` to find the exact string:

```toml
[[package]]
name = "some-crate"
version = "0.1.0"
source = "git+https://github.com/user/repo?branch=main#abc123"
```

Use `"some-crate-0.1.0"` as the key.

### 5. Verify the Build

```bash
nix build .#default
```

Should complete successfully without hash errors.

### 6. Test Locally

```bash
nix develop
cargo test
```

Verify all tests still pass with the updated dependency.

## Common Mistakes

### ❌ Using base32 Format

```nix
# WRONG - this is base32 Nix hash format:
"sha256-02cawkx6bxfi3bn1sb5ws8cn9wzcwsk8cdv1vx8h8lad1jdic1qg"
```

Modern Nix requires SRI format (base64):

```nix
# CORRECT - SRI format with base64 and = padding:
"sha256-DwcWmwxNUQRR32E3hqbm7PNkGdK8LB3sGtH1Zfrkigk="
```

### ❌ Forgetting to Update Both Files

If you only update `flake.nix` OR `nix/module.nix`, the NixOS module deployment will fail because they both build the package independently.

### ❌ Converting Hashes Manually

Don't try to convert hashes yourself. Always use the hash Nix provides in the error message - it's already in the correct format.

### ❌ Wrong Key Format in outputHashes

The key must match exactly what appears in `Cargo.lock`:

```toml
# From Cargo.lock:
[[package]]
name = "nostr"
version = "0.44.1"

# Use in flake.nix/module.nix:
"nostr-0.44.1" = "sha256-..."  # ✅ Correct
"nostr" = "sha256-..."          # ❌ Wrong
```

## Background

The `outputHashes` field is needed because we use git dependencies instead of crates.io releases. Nix requires fixed-output derivations to have known hashes for reproducibility.

When the git repository changes (new commits, different branch/tag, etc.), the hash changes. Nix compares the expected hash (in our files) with the actual hash (computed from the source) and fails if they don't match.

This is a **security feature** - it prevents supply chain attacks by ensuring you get exactly the code you expect.

## Examples

### Example 1: Updating nostr-sdk fork

```bash
# 1. Update to newer commit
cd /path/to/ngit-grasp
cargo update -p nostr

# 2. Try to build with Nix (will fail with hash)
nix build .#default

# 3. Copy the hash from error message
# got: sha256-ABC123...=

# 4. Update both files:
# flake.nix and nix/module.nix
"nostr-0.44.1" = "sha256-ABC123...=";

# 5. Build again
nix build .#default
```

### Example 2: Adding new git dependency

```bash
# 1. Add to Cargo.toml
[dependencies]
new-crate = { git = "https://github.com/user/repo", tag = "v1.0" }

# 2. Update Cargo.lock
cargo update -p new-crate

# 3. Try to build (will fail)
nix build .#default

# 4. Add to both flake.nix and nix/module.nix:
outputHashes = {
  "nostr-0.44.1" = "sha256-...";
  "new-crate-1.0.0" = "sha256-...";  # ← Add from error message
};
```

## See Also

- [Nix Manual: Cargo.lock Hash](https://nixos.org/manual/nixpkgs/stable/#rust-cargo-lock-hash)
- [SRI Hash Format](https://www.w3.org/TR/SRI/)
