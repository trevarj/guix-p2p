(use-modules (guix packages)
             (gnu)
             (gnu packages base)
             (gnu packages commencement)
             (gnu packages crypto)
             (gnu packages gnupg)
             (gnu packages nss)
             (gnu packages rust)
             (gnu packages tls)
             (gnu packages perl)
             (gnu packages pkg-config))

(packages->manifest
 (list
  gcc-toolchain
  binutils
  pkg-config
  nss-certs

  ;; Rust toolchain used by hosted CI and benchmark workflows.
  rust
  (list rust "cargo")
  (list rust "tools")
  (list rust "rust-src")

  ;; Native libraries and helpers needed by sys crates and build scripts.
  openssl
  libgcrypt
  perl))
