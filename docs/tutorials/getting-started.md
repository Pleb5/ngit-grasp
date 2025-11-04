# Tutorial: Getting Started with ngit-grasp

**Purpose:** Learn the basics of ngit-grasp through hands-on setup  
**Time:** 15-20 minutes  
**Prerequisites:** Basic Git and command-line knowledge

---

## What You'll Learn

By the end of this tutorial, you will:
- ✅ Have a working ngit-grasp development environment
- ✅ Understand the basic project structure
- ✅ Run the test suite successfully
- ✅ Know where to go next

---

## Step 1: Clone the Repository

First, get the source code:

```bash
git clone https://gitworkshop.dev/ngit-grasp
cd ngit-grasp
```

**What just happened?** You cloned the ngit-grasp repository from the GRASP-enabled Git server.

---

## Step 2: Set Up Nix Development Environment

ngit-grasp uses Nix flakes for reproducible development environments.

```bash
# Enter the development environment
nix develop

# You should see a new shell with all dependencies available
```

**What just happened?** Nix read `flake.nix` and created a shell with:
- Rust toolchain (cargo, rustc)
- Git
- All required system libraries

**Tip:** If `nix develop` doesn't work, you might be using an old Nix version. See the [Nix Flakes How-To](../how-to/nix-flakes.md) for help.

---

## Step 3: Explore the Project Structure

Take a look around:

```bash
# View the project structure
ls -la

# Key directories:
# - src/          - Main ngit-grasp source code (coming soon)
# - grasp-audit/  - Compliance testing tool (working)
# - docs/         - Documentation (you are here!)
```

**What you're seeing:**
- `grasp-audit/` is a **subproject** with its own Cargo workspace
- Main ngit-grasp server implementation is planned but not yet started
- Documentation uses Diátaxis framework (tutorials, how-to, reference, explanation)

---

## Step 4: Work with grasp-audit

The compliance testing tool is the first working component. Let's try it:

```bash
# Navigate to grasp-audit
cd grasp-audit

# Enter its development environment
nix develop

# Build the project
cargo build

# Run unit tests
cargo test
```

**What just happened?** 
- `grasp-audit` has its own `flake.nix` for isolated dependencies
- Unit tests run without external dependencies
- Integration tests (marked `#[ignore]`) require a Nostr relay

---

## Step 5: Run Your First Audit (Optional)

If you want to try the audit tool against a real relay:

```bash
# In a separate terminal, start a test relay:
docker run --rm -p 7000:7000 scsibug/nostr-rs-relay

# Back in grasp-audit directory:
cargo test --ignored -- --test-threads=1
```

**What just happened?** Integration tests connected to the relay on port 7000 and verified GRASP compliance.

**Note:** This step is optional. The relay must be running for these tests to pass.

---

## Step 6: Explore the Code

Let's look at a simple example:

```bash
# From grasp-audit directory
cat examples/simple_audit.rs
```

This shows how to use the `grasp-audit` library to check GRASP compliance.

---

## Step 7: Read the Documentation

Now that you have a working setup, explore the documentation:

```bash
# From project root
cd ..
ls docs/
```

**Recommended reading order:**
1. [Architecture Overview](../explanation/architecture.md) - Understand the design
2. [Inline Authorization](../explanation/inline-authorization.md) - Key decision
3. [Git Protocol Reference](../reference/git-protocol.md) - Technical details

---

## What You've Accomplished

Congratulations! You now have:

✅ A working Nix development environment  
✅ Built and tested the grasp-audit tool  
✅ Understanding of the project structure  
✅ Knowledge of where to find more information

---

## Next Steps

### If you want to contribute:
1. Read [Architecture Overview](../explanation/architecture.md)
2. Check open issues on the repository
3. Review [Design Decisions](../explanation/decisions.md)

### If you want to deploy:
1. Follow [Deployment How-To](../how-to/deploy.md)
2. Review [Configuration Reference](../reference/configuration.md)

### If you want to understand GRASP:
1. Read [GRASP Protocol Reference](../reference/grasp-protocol.md)
2. Review [Comparison with ngit-relay](../explanation/comparison.md)

### If you want to run compliance tests:
1. Follow [Running Your First Audit Tutorial](first-audit.md)
2. Review [Compliance Testing How-To](../how-to/test-compliance.md)

---

## Troubleshooting

### "nix develop" doesn't work
- You might need Nix with flakes enabled
- See [Nix Flakes How-To](../how-to/nix-flakes.md)

### Build errors in grasp-audit
- Make sure you're in the `grasp-audit` directory
- Run `nix develop` first
- Check that you have network access (Cargo needs to download crates)

### Tests fail
- Unit tests should always pass
- Integration tests (`--ignored`) require a relay on port 7000
- Use `--test-threads=1` for integration tests

---

## Summary

You've successfully set up ngit-grasp and learned:
- How to use Nix flakes for development
- The project structure (main server + grasp-audit tool)
- How to build and test the code
- Where to find documentation

**Ready for more?** Try the [First Audit Tutorial](first-audit.md) next!

---

*Part of the [ngit-grasp tutorials](./)*  
*Next: [Running Your First Audit](first-audit.md)*
