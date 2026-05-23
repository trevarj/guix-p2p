(define-module (guix extensions substitute)
  #:use-module ((guix scripts substitute) #:prefix builtin:)
  #:use-module (guix base64)
  #:use-module ((guix build utils) #:select (delete-file-recursively))
  #:use-module (ice-9 match)
  #:use-module (ice-9 rdelim)
  #:use-module (rnrs io ports)
  #:use-module (srfi srfi-11)
  #:use-module (srfi srfi-13)
  #:export (guix-substitute))

(define %default-socket
  "/var/cache/guix-p2p/guix-p2p.sock")

(define (getenv/default name default)
  (or (getenv name) default))

(define (socket? path)
  (false-if-exception
   (eq? 'socket (stat:type (stat path)))))

(define (terminal-substitute-reply? line)
  (let ((trimmed (string-trim-both line)))
    (or (string=? trimmed "not-found")
        (string-prefix? "success " trimmed)
        (string-prefix? "hash-mismatch " trimmed))))

(define (substitute-destination line)
  (match (string-tokenize line)
    (("substitute" _ destination) destination)
    (_ #f)))

(define (open-relay-socket socket-path)
  (let ((sock (socket AF_UNIX SOCK_STREAM 0)))
    (connect sock AF_UNIX socket-path)
    sock))

(define (call-with-relay-socket socket-path proc)
  (let ((socket-port #f))
    (dynamic-wind
      (lambda ()
        (set! socket-port (open-relay-socket socket-path)))
      (lambda ()
        (proc socket-port))
      (lambda ()
        (when socket-port
          (false-if-exception (close-port socket-port)))))))

(define (write-line port line)
  (display line port)
  (newline port))

(define (write-reply-line port line)
  (write-line port line)
  (force-output port))

(define (write-trace-line line)
  (write-line (current-output-port) line)
  (force-output (current-output-port)))

(define (reply-port)
  (or (false-if-exception (fdopen 4 "w0"))
      (current-output-port)))

(define (remove-destination path)
  (when (file-exists? path)
    (delete-file-recursively path)))

(define (relay-stdin-to-socket mode socket-port)
  (write-line socket-port
              (match mode
                ('query "mode: query")
                ('substitute "mode: substitute")))
  (let loop ((destinations '()))
    (match (read-line)
      ((? eof-object?)
       (force-output socket-port)
       (shutdown socket-port 1)
       (reverse destinations))
      (line
       (write-line socket-port line)
       (loop (match (and (eq? mode 'substitute)
                         (substitute-destination line))
               (#f destinations)
               (destination (cons destination destinations))))))))

(define (open-nar-destination destinations)
  (match destinations
    (() (error "received nar data without a destination"))
    ((destination rest ...)
     (remove-destination destination)
     (values destination rest (open-file destination "wb")))))

(define (handle-relay-output socket-port reply-port destinations)
  (let loop ((destinations destinations)
             (expected-terminal-replies (length destinations))
             (nar-destination #f)
             (nar-port #f)
             (terminal-replies 0))
    (match (read-line socket-port)
      ((? eof-object?)
       (when nar-port
         (close-port nar-port)
         (error "daemon socket closed before finishing nar" nar-destination))
       (when (< terminal-replies expected-terminal-replies)
         (error "daemon socket closed before substitute returned a terminal reply"))
       #t)
      (line
       (cond
        ((string-prefix? "fd4:" line)
         (let ((data (string-drop line 4)))
           (write-reply-line reply-port data)
           (loop destinations
                 expected-terminal-replies
                 nar-destination
                 nar-port
                 (if (terminal-substitute-reply? data)
                     (+ terminal-replies 1)
                     terminal-replies))))
        ((string-prefix? "out:" line)
         (write-trace-line (string-drop line 4))
         (loop destinations expected-terminal-replies nar-destination nar-port terminal-replies))
        ((string-prefix? "nar:" line)
         (let-values (((destination rest port)
                       (if nar-port
                           (values nar-destination destinations nar-port)
                           (open-nar-destination destinations))))
           (put-bytevector port (base64-decode (string-drop line 4)))
           (loop rest expected-terminal-replies destination port terminal-replies)))
        ((string=? line "nar-end")
         (unless nar-port
           (error "received nar-end without a destination"))
         (close-port nar-port)
         (loop destinations expected-terminal-replies #f #f terminal-replies))
        (else
         ;; Backward-compatible fallback for legacy unprefixed socket replies.
         (write-reply-line reply-port line)
         (loop destinations
               expected-terminal-replies
               nar-destination
               nar-port
               (if (terminal-substitute-reply? line)
                   (+ terminal-replies 1)
                   terminal-replies))))))))

(define (handle-query-reply socket-port reply-port)
  (let loop ()
    (match (read-line socket-port)
      ((? eof-object?)
       (error "daemon socket closed before query reply completed"))
      (line
       (cond
        ((string-prefix? "fd4:" line)
         (let ((data (string-drop line 4)))
           (write-reply-line reply-port data)
           (unless (string-null? data)
             (loop))))
        ((string-prefix? "out:" line)
         (write-trace-line (string-drop line 4))
         (loop))
        (else
         ;; Backward-compatible fallback for legacy unprefixed socket replies.
         (write-reply-line reply-port line)
         (unless (string-null? line)
           (loop))))))))

(define (relay-query-through-socket socket-port reply-port)
  (write-line socket-port "mode: query")
  (force-output socket-port)
  (let loop ()
    (match (read-line)
      ((? eof-object?)
       (shutdown socket-port 1)
       #t)
      (line
       ;; Guix keeps substitute --query alive and expects a reply for each
       ;; query before it necessarily closes stdin.
       (write-line socket-port line)
       (force-output socket-port)
       (handle-query-reply socket-port reply-port)
       (loop)))))

(define (relay-through-socket args socket-path)
  (match args
    (("--query" _ ...)
     (call-with-relay-socket
      socket-path
      (lambda (socket-port)
        (let ((reply-port (reply-port)))
          (relay-query-through-socket socket-port reply-port)
          #t))))
    (("--substitute" _ ...)
     (call-with-relay-socket
      socket-path
      (lambda (socket-port)
        (let* ((reply-port (reply-port))
               (destinations (relay-stdin-to-socket 'substitute socket-port)))
          (handle-relay-output socket-port reply-port destinations)
          #t))))
    (_ #f)))

(define (maybe-relay-through-socket args)
  (match args
    (((or "--query" "--substitute") _ ...)
     (let ((socket (getenv/default "GUIX_P2P_SOCKET" %default-socket)))
       (and (socket? socket)
            (relay-through-socket args socket))))
    (_ #f)))

(define (guix-substitute . args)
  (unless (maybe-relay-through-socket args)
    (apply builtin:guix-substitute args)))
