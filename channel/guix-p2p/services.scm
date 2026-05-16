(define-module (guix-p2p services)
  #:use-module (guix gexp)
  #:use-module (guix records)
  #:use-module (guix-p2p packages)
  #:use-module (gnu services)
  #:use-module (gnu services base)
  #:use-module (gnu services shepherd)
  #:use-module (ice-9 optargs)
  #:use-module (srfi srfi-1)
  #:use-module (srfi srfi-13)
  #:export (%guix-p2p-default-cache-directory
            %guix-p2p-default-socket
            <guix-p2p-configuration>
            guix-p2p-configuration
            guix-p2p-configuration?
            guix-p2p-configuration-package
            guix-p2p-configuration-cache-directory
            guix-p2p-configuration-socket
            guix-p2p-configuration-listen-address
            guix-p2p-configuration-dashboard?
            guix-p2p-configuration-dashboard-bind
            guix-p2p-configuration-dashboard-port
            guix-p2p-configuration-extra-options
            guix-p2p-service-type
            guix-p2p-enable-guix-daemon-extension
            guix-p2p-enable-guix-daemon-wrapper))

(define-public %guix-p2p-default-cache-directory
  "/var/cache/guix-p2p")

(define-public %guix-p2p-default-socket
  (string-append %guix-p2p-default-cache-directory "/guix-p2p.sock"))

(define-record-type* <guix-p2p-configuration>
  guix-p2p-configuration make-guix-p2p-configuration
  guix-p2p-configuration?
  (package guix-p2p-configuration-package
           (default guix-p2p))
  (cache-directory guix-p2p-configuration-cache-directory
                   (default %guix-p2p-default-cache-directory))
  (socket guix-p2p-configuration-socket
          (default %guix-p2p-default-socket))
  (listen-address guix-p2p-configuration-listen-address
                  (default "/ip4/0.0.0.0/udp/6881/quic-v1"))
  (dashboard? guix-p2p-configuration-dashboard?
              (default #f))
  (dashboard-bind guix-p2p-configuration-dashboard-bind
                  (default "127.0.0.1"))
  (dashboard-port guix-p2p-configuration-dashboard-port
                  (default 3030))
  (extra-options guix-p2p-configuration-extra-options
                 (default '())))

(define (guix-p2p-shepherd-services config)
  (let* ((package (guix-p2p-configuration-package config))
         (dashboard-port
          (number->string (guix-p2p-configuration-dashboard-port config)))
         (command
          `(,(file-append package "/bin/guix-p2p")
            "--daemon"
            "--listen-addr" ,(guix-p2p-configuration-listen-address config)
            "--cache-dir" ,(guix-p2p-configuration-cache-directory config)
            "--socket" ,(guix-p2p-configuration-socket config)
            ,@(if (guix-p2p-configuration-dashboard? config)
                  `("--dashboard"
                    "--dashboard-bind" ,(guix-p2p-configuration-dashboard-bind config)
                    "--dashboard-port" ,dashboard-port)
                  '())
            ,@(guix-p2p-configuration-extra-options config))))
    (list
     (shepherd-service
      (documentation "Run the guix-p2p substitute relay daemon.")
      (provision '(guix-p2p))
      (requirement '(user-processes))
      (start #~(make-forkexec-constructor '#$command
                                          #:log-file "/var/log/guix-p2p.log"))
      (stop #~(make-kill-destructor))
      (respawn? #t)))))

(define (guix-p2p-activation config)
  #~(begin
      (use-modules (guix build utils))
      (mkdir-p #$(guix-p2p-configuration-cache-directory config))))

(define-public guix-p2p-service-type
  (service-type
   (name 'guix-p2p)
   (extensions
    (list (service-extension shepherd-root-service-type
                             guix-p2p-shepherd-services)
          (service-extension activation-service-type
                             guix-p2p-activation)
          (service-extension profile-service-type
                             (compose list guix-p2p-configuration-package))))
   (default-value (guix-p2p-configuration))
   (description
    "Run @command{guix-p2p} as a persistent substitute relay daemon.")))

(define (guix-p2p-integration-environment? value)
  (any (lambda (prefix)
         (string-prefix? prefix value))
       '("GUIX="
         "GUIX_EXTENSIONS_PATH="
         "GUIX_P2P_BIN="
         "GUIX_P2P_SOCKET="
         "REAL_GUIX=")))

(define (guix-p2p-environment-value name environment)
  (let ((prefix (string-append name "=")))
    (any (lambda (value)
           (and (string-prefix? prefix value)
                (string-drop value (string-length prefix))))
         environment)))

(define*-public (guix-p2p-enable-guix-daemon-extension
                 config
                 #:key
                 (extensions "/run/current-system/profile/share/guix/extensions")
                 (guix-p2p-bin "/run/current-system/profile/bin/guix-p2p")
                 (socket %guix-p2p-default-socket))
  "Return CONFIG with guix-daemon resolving the guix-p2p substitute extension."
  (let* ((environment (guix-configuration-environment config))
         (existing-extensions
          (guix-p2p-environment-value "GUIX_EXTENSIONS_PATH" environment))
         (extensions-path
          (if existing-extensions
              (string-append extensions ":" existing-extensions)
              extensions)))
    (guix-configuration
     (inherit config)
     (environment
      (cons* (string-append "GUIX_EXTENSIONS_PATH=" extensions-path)
             (string-append "GUIX_P2P_BIN=" guix-p2p-bin)
             (string-append "GUIX_P2P_SOCKET=" socket)
             (remove guix-p2p-integration-environment?
                     environment))))))

(define*-public (guix-p2p-enable-guix-daemon-wrapper
                 config
                 #:key
                 (wrapper "/run/current-system/profile/bin/guix-p2p-wrapper")
                 (guix-p2p-bin "/run/current-system/profile/bin/guix-p2p")
                 (socket %guix-p2p-default-socket)
                 (real-guix "/run/current-system/profile/bin/guix"))
  "Return CONFIG with guix-daemon invoking guix-p2p-wrapper as its Guix program."
  (guix-configuration
   (inherit config)
   (environment
    (cons* (string-append "GUIX=" wrapper)
           (string-append "GUIX_P2P_BIN=" guix-p2p-bin)
           (string-append "GUIX_P2P_SOCKET=" socket)
           (string-append "REAL_GUIX=" real-guix)
           (remove guix-p2p-integration-environment?
                   (guix-configuration-environment config))))))
