(use-modules (guix build-system cargo)
             (guix git-download)
             (guix gexp)
             (guix import crate)
             ((guix licenses) #:prefix license:)
             (guix packages)
             (guix utils)
             (gnu packages compression)
             (gnu packages commencement)
             (gnu packages gnupg)
             (gnu packages linux)
             (gnu packages nss)
             (gnu packages perl)
             (gnu packages pkg-config)
             (gnu packages rust)
             (gnu packages tls))

(define-public guix-p2p
  (package
    (name "guix-p2p")
    (version "0.1.0")
    (source (local-file "." "guix-p2p-checkout"
                        #:recursive? #t
                        #:select? (git-predicate ".")))
    (build-system cargo-build-system)
    (arguments
     (list
      #:tests? #f
      #:install-source? #f
      #:cargo-build-flags
      ''("--release"
         "--bin" "guix-p2p"
         "--bin" "guix-p2p-wrapper")
      #:cargo-install-paths ''(".")))
    (native-inputs
     (list gcc-toolchain
           pkg-config
           perl))
    (inputs
     (append (cargo-inputs-from-lockfile)
             (list libgcrypt
                   nss-certs
                   openssl
                   zstd
                   (list zstd "lib")
                   (list zstd "static"))))
    (home-page "https://codeberg.org/trevarj/guix-p2p")
    (synopsis "P2P binary substitute distribution for GNU Guix")
    (description
     "guix-p2p is a libp2p daemon and Guix substitute relay.  It installs both
the @command{guix-p2p} daemon and @command{guix-p2p-wrapper} PATH wrapper.")
    (license license:gpl3+)))

guix-p2p
