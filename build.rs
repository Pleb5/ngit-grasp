use std::process::Command;

fn main() {
    // Get the short git commit hash
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let commit = String::from_utf8_lossy(&output.stdout);
            let commit = commit.trim();
            println!("cargo:rustc-env=GIT_COMMIT_SHORT={}", commit);
        }
    }

    // Re-run if HEAD changes (new commits)
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads/");
}
