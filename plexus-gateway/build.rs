/// Build script for plexus-gateway.
///
/// When the `embed-frontend` feature is enabled, this automatically runs
/// `npm run build` in the frontend directory so the compiled assets are
/// available for rust-embed to include in the binary.

fn main() {
    #[cfg(feature = "embed-frontend")]
    {
        use std::process::Command;

        let frontend_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plexus-frontend");

        // Re-run this build script if frontend source changes.
        println!("cargo:rerun-if-changed=../plexus-frontend/src");
        println!("cargo:rerun-if-changed=../plexus-frontend/index.html");
        println!("cargo:rerun-if-changed=../plexus-frontend/package.json");

        // Only rebuild if dist is missing or we're doing a release build.
        let dist = frontend_dir.join("dist");
        if !dist.exists() || std::env::var("PROFILE").unwrap_or_default() == "release" {
            eprintln!("Building frontend...");

            let npm_install = Command::new("npm")
                .arg("install")
                .current_dir(&frontend_dir)
                .status()
                .expect("failed to run npm install — is Node.js installed?");
            assert!(npm_install.success(), "npm install failed");

            let npm_build = Command::new("npm")
                .arg("run")
                .arg("build")
                .current_dir(&frontend_dir)
                .status()
                .expect("failed to run npm run build");
            assert!(npm_build.success(), "npm run build failed");
        }
    }
}
