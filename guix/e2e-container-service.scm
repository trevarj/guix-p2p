(use-modules (gnu)
             (gnu bootloader grub)
             (gnu packages bash)
             (gnu services)
             (gnu services shepherd)
             (guix gexp))

(define debug-service-runner
  (program-file
   "guix-p2p-debug-service-runner"
   #~(begin
       (use-modules (ice-9 textual-ports))

       (define log-file "/var/log/guix-p2p-debug-service.log")

       (call-with-output-file log-file
         (lambda (port)
           (display "guix-p2p debug service started\n" port)))
       (let loop ()
         (sleep 3600)
         (loop)))))

(define debug-service
  (shepherd-service
   (provision '(guix-p2p-debug))
   (requirement '(user-processes))
   (documentation "Minimal debug service for guix-p2p container diagnosis.")
   (start #~(make-forkexec-constructor
             (list #$debug-service-runner)
             #:log-file "/var/log/guix-p2p-debug-service.log"))
   (stop #~(make-kill-destructor))))

(operating-system
  (host-name "guix-p2p-service")
  (timezone "Etc/UTC")
  (locale "en_US.utf8")
  (bootloader
   (bootloader-configuration
    (bootloader grub-bootloader)
    (targets '("/dev/null"))))
  (file-systems %base-file-systems)
  (packages (cons bash %base-packages))
  (services
   (cons (simple-service 'guix-p2p-debug-service
                         shepherd-root-service-type
                         (list debug-service))
         %base-services)))
