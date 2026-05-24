(use-modules (guix packages)
             (gnu)
             (gnu packages base)
             (gnu packages commencement)
             (gnu packages crypto)
             (gnu packages gnupg)
             (gnu packages nss)
             (gnu packages package-management)
             (gnu packages rust)
             (gnu packages ssh)
             (gnu packages tls)
             (gnu packages virtualization)
             (gnu packages perl)
             (gnu packages pkg-config))

(packages->manifest
 (list
  gcc-toolchain
  binutils
  pkg-config
  nss-certs
  guix
  openssh
  qemu

  ;; Rust toolchain used by hosted CI and benchmark workflows.
  rust
  (list rust "cargo")
  (list rust "tools")
  (list rust "rust-src")

  ;; Native libraries and helpers needed by sys crates and build scripts.
  openssl
  libgcrypt
  perl))
