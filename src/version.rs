/// Short Git commit embedded at build time.
pub const COMMIT: &str = env!("GUIX_P2P_BUILD_COMMIT");

/// Human-readable version shown by CLI and libp2p agent metadata.
pub const VERSION: &str =
    concat!(env!("CARGO_PKG_VERSION"), " (", env!("GUIX_P2P_BUILD_COMMIT"), ")");
