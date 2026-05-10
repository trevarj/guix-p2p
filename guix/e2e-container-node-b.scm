(use-modules (gnu)
             (gnu bootloader grub)
             (gnu packages bash)
             (gnu services)
             (gnu services shepherd)
             (guix gexp))

(define guix-p2p-node-b-runner
  (program-file
   "guix-p2p-node-b-runner"
   #~(begin
       (define project-dir "/src")
       (define binary (string-append project-dir "/target/release/guix-p2p"))
       (define cache-dir "/var/lib/guix-p2p-node-b")
       (define socket "/run/guix-p2p-node-b.sock")
       (define bootstrap-peers (getenv "GUIX_P2P_BOOTSTRAP_PEERS"))
       (define base-args
         (list binary
               "--daemon"
               "--listen-addr" "/ip4/127.0.0.1/tcp/6882"
               "--cache-dir" cache-dir
               "--socket" socket
               "--dashboard"
               "--dashboard-bind" "127.0.0.1"
               "--dashboard-port" "3032"
               "--policy" "p2p-only"))
       (define args
         (if (and bootstrap-peers (not (string=? bootstrap-peers "")))
             (append base-args (list "--bootstrap-peers" bootstrap-peers))
             base-args))

       (unless (access? binary X_OK)
         (format (current-error-port)
                 "guix-p2p binary is missing or not executable: ~a~%"
                 binary)
         (exit 1))

       (apply execl binary args))))

(define guix-p2p-node-b-service
  (shepherd-service
   (provision '(guix-p2p-node-b))
   (requirement '(user-processes))
   (documentation "Run guix-p2p Node B for container diagnosis.")
   (start #~(make-forkexec-constructor
             (list #$guix-p2p-node-b-runner)
             #:log-file "/var/log/guix-p2p-node-b.log"))
   (stop #~(make-kill-destructor))))

(operating-system
  (host-name "guix-p2p-node-b")
  (timezone "Etc/UTC")
  (locale "en_US.utf8")
  (bootloader
   (bootloader-configuration
    (bootloader grub-bootloader)
    (targets '("/dev/null"))))
  (file-systems %base-file-systems)
  (packages (cons bash %base-packages))
  (services
   (cons (simple-service 'guix-p2p-node-b-service
                         shepherd-root-service-type
                         (list guix-p2p-node-b-service))
         %base-services)))
