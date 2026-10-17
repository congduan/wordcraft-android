//! Host-side placeholder for the Android cdylib crate.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

fn main() {
    // Android starts the cdylib's `android_main`; no host executable is shipped.
}
