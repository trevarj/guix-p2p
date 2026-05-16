(define-module (guix extensions substitute)
  #:use-module ((guix scripts substitute) #:prefix builtin:)
  #:use-module (ice-9 match)
  #:use-module (srfi srfi-1)
  #:export (guix-substitute))

(define %default-socket
  "/var/cache/guix-p2p/guix-p2p.sock")

(define %default-program
  "/run/current-system/profile/bin/guix-p2p")

(define (getenv/default name default)
  (or (getenv name) default))

(define (socket? path)
  (false-if-exception
   (eq? 'socket (stat:type (stat path)))))

(define (relay-arguments args socket)
  (append args (list "--socket" socket)))

(define (maybe-exec-relay args)
  (match args
    (((or "--query" "--substitute") _ ...)
     (let ((socket (getenv/default "GUIX_P2P_SOCKET" %default-socket)))
       (when (socket? socket)
         (let ((program (getenv/default "GUIX_P2P_BIN" %default-program)))
           (apply execlp program program (relay-arguments args socket))))))))

(define (guix-substitute . args)
  (maybe-exec-relay args)
  (apply builtin:guix-substitute args))
