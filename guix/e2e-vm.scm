(use-modules (gnu)
             (gnu bootloader grub)
             (gnu packages bash)
             (gnu services)
             (gnu services networking)
             (gnu services shepherd)
             (gnu system nss))

(define guix-p2p-e2e-runner
  (program-file
   "guix-p2p-e2e-runner"
   #~(begin
       (use-modules (ice-9 popen)
                    (ice-9 textual-ports))

       (define (run . args)
         (let* ((port (apply open-pipe* OPEN_BOTH args))
                (output (get-string-all port))
                (status (close-pipe port)))
           (display output)
           (unless (zero? status)
             (format (current-error-port) "command failed: ~s~%" args)
             (exit 1))))

       (define (try-run . args)
         (let* ((port (apply open-pipe* OPEN_BOTH args))
                (output (get-string-all port))
                (status (close-pipe port)))
           (display output)
           (zero? status)))

       (define repo "/mnt/guix-p2p")
       (define target "/tmp/guix-p2p-target")
       (define base "/tmp/guix-p2p-e2e")
       (define profile "/run/current-system/profile/bin/")

       (run (string-append profile "mkdir") "-p" repo target)
       (unless (try-run (string-append profile "mount")
                        "-t" "9p" "-o" "trans=virtio,version=9p2000.L"
                        "guix_p2p" repo)
         (format (current-error-port)
                 "failed to mount shared checkout tag guix_p2p at ~a~%" repo)
         (exit 1))

       ;; This VM is a disposable test image. The raw test guix-daemon imports
       ;; substituted nars into /gnu/store, so the harness must be able to write.
       (try-run (string-append profile "mount") "-o" "remount,rw" "/gnu/store")
       (unless (try-run (string-append profile "test") "-w" "/gnu/store")
         (format (current-error-port)
                 "/gnu/store is not writable in the disposable E2E VM~%")
         (exit 1))

       (setenv "CARGO_TARGET_DIR" target)
       (setenv "GUIX_P2P_E2E_BASE" base)
       (chdir repo)
       (run (string-append profile "guix") "shell" "-m" "manifest.scm" "--"
            "cargo" "run" "-p" "guix-p2p-e2e" "--" "container-smoke"
            "--package" "hello"
            "--transport" "tcp"
            "--dashboard-bind" "0.0.0.0"
            "--hold"
            "--keep-temp"))))

(define guix-p2p-e2e-service
  (shepherd-service
   (provision '(guix-p2p-e2e))
   (requirement '(user-processes networking guix-daemon))
   (documentation "Run the guix-p2p disposable VM smoke proof.")
   (start #~(make-forkexec-constructor
             (list #$guix-p2p-e2e-runner)
             #:log-file "/var/log/guix-p2p-e2e.log"))
   (stop #~(make-kill-destructor))))

(operating-system
  (host-name "guix-p2p-e2e")
  (timezone "Etc/UTC")
  (locale "en_US.utf8")
  (bootloader
   (bootloader-configuration
    (bootloader grub-bootloader)
    (targets '("/dev/vda"))))
  (file-systems
   (cons (file-system
           (mount-point "/")
           (device (file-system-label "Guix_image"))
           (type "ext4"))
         %base-file-systems))
  (services
    (append
    (list (service dhcpcd-service-type)
          (simple-service 'guix-p2p-e2e-runner
                          shepherd-root-service-type
                          (list guix-p2p-e2e-service)))
    %base-services))
  (name-service-switch %mdns-host-lookup-nss))
