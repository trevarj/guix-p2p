(define-module (guix-p2p packages)
  #:use-module (guix build-system cargo)
  #:use-module (guix git-download)
  #:use-module (guix gexp)
  #:use-module (guix import crate)
  #:use-module ((guix licenses) #:prefix license:)
  #:use-module (guix packages)
  #:use-module (guix search-paths)
  #:use-module (guix utils)
  #:use-module (gnu packages compression)
  #:use-module (gnu packages commencement)
  #:use-module (gnu packages gnupg)
  #:use-module (gnu packages nss)
  #:use-module (gnu packages perl)
  #:use-module (gnu packages pkg-config)
  #:use-module (gnu packages tls)
  #:use-module (srfi srfi-1)
  #:use-module (srfi srfi-13)
  #:export (guix-p2p))

(define %guix-p2p-channel-root
  (or (and=> (current-filename)
             (lambda (file)
               (dirname (dirname file))))
      (and=> (search-path %load-path "guix-p2p/packages.scm")
             (lambda (file)
               (dirname (dirname file))))
      (getcwd)))

(define %guix-p2p-lockfile
  (string-append %guix-p2p-channel-root "/Cargo.lock"))

(define %guix-p2p-checkout-root
  (or (getenv "GUIX_P2P_CHECKOUT_ROOT")
      (and (file-exists? %guix-p2p-lockfile)
           (dirname (canonicalize-path %guix-p2p-lockfile)))
      (getcwd)))

(define (guix-p2p-generated-path? file)
  (any (lambda (part)
         (string-contains file part))
       '("/.git/"
         "/target/"
         "/guix-vendor/")))

(define (guix-p2p-extension-source? file)
  (or (string-suffix? "/guix/extensions" file)
      (string-contains file "/guix/extensions/")))

(define (guix-p2p-source-predicate root)
  (let ((git-file? (git-predicate root)))
    (lambda (file stat)
      (and (not (guix-p2p-generated-path? file))
           (or (guix-p2p-extension-source? file)
               (if git-file?
                   (git-file? file stat)
                   #t))))))

(define-public guix-p2p
  (package
    (name "guix-p2p")
    (version "0.1.0")
    (source (local-file %guix-p2p-checkout-root "guix-p2p-checkout"
                        #:recursive? #t
                        #:select? (guix-p2p-source-predicate
                                   %guix-p2p-checkout-root)))
    (build-system cargo-build-system)
    (arguments
     (list
      #:tests? #f
      #:install-source? #f
      #:cargo-build-flags
      ''("--release"
         "--bin" "guix-p2p"
         "--bin" "guix-p2p-wrapper")
      #:cargo-install-paths ''(".")
      #:phases
      #~(modify-phases %standard-phases
          (add-after 'install 'install-guix-extension
            (lambda _
              (unless (file-exists? "guix/extensions/substitute.scm")
                (error "missing guix-p2p substitute extension"))
              (let ((extensions (string-append #$output "/share/guix/extensions")))
                (mkdir-p extensions)
                (copy-recursively "guix/extensions" extensions)))))))
    (native-inputs
     (list gcc-toolchain
           pkg-config
           perl))
    (inputs
     (append (cargo-inputs-from-lockfile
              %guix-p2p-lockfile)
             (list libgcrypt
                   nss-certs
                   openssl
                   zstd
                   (list zstd "lib")
                   (list zstd "static"))))
    (native-search-paths
     (list $GUIX_EXTENSIONS_PATH))
    (home-page "https://codeberg.org/trevarj/guix-p2p")
    (synopsis "P2P binary substitute distribution for GNU Guix")
    (description
     "guix-p2p is a libp2p daemon and Guix substitute relay.  It installs the
@command{guix-p2p} daemon, a Guix substitute command extension, and the
@command{guix-p2p-wrapper} compatibility wrapper.")
    (license license:gpl3+)))
