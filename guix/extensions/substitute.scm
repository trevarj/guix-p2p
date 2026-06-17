(define-module (guix extensions substitute)
  #:use-module ((guix scripts substitute) #:prefix builtin:)
  #:use-module (guix base64)
  #:use-module ((guix serialization) #:select (restore-file))
  #:use-module ((guix build utils) #:select (delete-file-recursively))
  #:use-module (ice-9 match)
  #:use-module (ice-9 rdelim)
  #:use-module (rnrs io ports)
  #:use-module (srfi srfi-1)
  #:use-module (srfi srfi-11)
  #:use-module (srfi srfi-13)
  #:export (guix-substitute))

(define %default-socket
  "/var/cache/guix-p2p/guix-p2p.sock")

(define %default-routing
  "builtin-first")

(define (getenv/default name default)
  (or (getenv name) default))

(define (substitute-routing)
  (getenv/default "GUIX_P2P_SUBSTITUTE_ROUTING" %default-routing))

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
     (let* ((template (string-copy
                       (string-append (or (getenv "TMPDIR") "/tmp")
                                      "/guix-p2p-substitute-XXXXXX")))
            (port (mkstemp! template)))
       (values destination rest port template)))))

(define (restore-nar-destination nar-port nar-temp-path destination)
  (close-port nar-port)
  (dynamic-wind
    (const #t)
    (lambda ()
      (remove-destination destination)
      (call-with-input-file nar-temp-path
        (lambda (port)
          (restore-file port destination))))
    (lambda ()
      (false-if-exception (delete-file nar-temp-path)))))

(define (handle-relay-output socket-port reply-port destinations)
  (let loop ((destinations destinations)
             (expected-terminal-replies (length destinations))
             (nar-destination #f)
             (nar-port #f)
             (nar-temp-path #f)
             (terminal-replies 0))
    (match (read-line socket-port)
      ((? eof-object?)
       (when nar-port
         (close-port nar-port)
         (when nar-temp-path
           (false-if-exception (delete-file nar-temp-path)))
         (error "daemon socket closed before finishing nar" nar-destination))
       (when (< terminal-replies expected-terminal-replies)
         (error "daemon socket closed before substitute returned a terminal reply"))
      #t)
     (line
      (cond
        ((string=? line "fd4-empty:")
         (write-reply-line reply-port "")
         (loop destinations
               expected-terminal-replies
               nar-destination
               nar-port
               nar-temp-path
               terminal-replies))
        ((string-prefix? "fd4:" line)
         (let ((data (string-drop line 4)))
           (write-reply-line reply-port data)
           (let ((terminal-replies*
                  (if (terminal-substitute-reply? data)
                      (+ terminal-replies 1)
                      terminal-replies)))
             (if (and (> expected-terminal-replies 0)
                      (>= terminal-replies* expected-terminal-replies)
                      (not nar-port))
                 #t
                 (loop destinations
                       expected-terminal-replies
                       nar-destination
                       nar-port
                       nar-temp-path
                       terminal-replies*)))))
        ((string-prefix? "out:" line)
         (write-trace-line (string-drop line 4))
         (loop destinations
               expected-terminal-replies
               nar-destination
               nar-port
               nar-temp-path
               terminal-replies))
        ((string-prefix? "nar:" line)
         (let-values (((destination rest port temp-path)
                       (if nar-port
                           (values nar-destination destinations nar-port nar-temp-path)
                           (open-nar-destination destinations))))
           (put-bytevector port (base64-decode (string-drop line 4)))
           (loop rest expected-terminal-replies destination port temp-path terminal-replies)))
        ((string=? line "nar-end")
         (unless nar-port
           (error "received nar-end without a destination"))
         (restore-nar-destination nar-port nar-temp-path nar-destination)
         (loop destinations expected-terminal-replies #f #f #f terminal-replies))
        (else
         ;; Backward-compatible fallback for legacy unprefixed socket replies.
         (write-reply-line reply-port line)
         (loop destinations
               expected-terminal-replies
               nar-destination
               nar-port
               nar-temp-path
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
        ((string=? line "fd4-empty:")
         (write-reply-line reply-port "")
         (loop))
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

(define (relay-query-through-socket socket-port reply-port mode-line)
  (write-line socket-port mode-line)
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

(define (relay-substitute-through-socket socket-port reply-port mode-line)
  (write-line socket-port mode-line)
  (force-output socket-port)
  (let loop ()
    (match (read-line)
      ((? eof-object?)
       (shutdown socket-port 1)
       #t)
      (line
       ;; Guix keeps substitute --substitute alive while waiting for the
       ;; terminal reply, so handle one substitute request before reading the
       ;; next stdin line.
       (write-line socket-port line)
       (force-output socket-port)
       (let ((destination (substitute-destination line)))
         (unless destination
           (error "invalid substitute command" line))
         (handle-relay-output socket-port reply-port (list destination)))
       (loop)))))

(define (relay-through-socket args socket-path force-p2p-only?)
  (match args
    (("--query" _ ...)
     (call-with-relay-socket
      socket-path
      (lambda (socket-port)
        (let ((reply-port (reply-port)))
          (relay-query-through-socket
           socket-port
           reply-port
           (if force-p2p-only? "mode: query-p2p-only" "mode: query"))
          #t))))
    (("--substitute" _ ...)
     (call-with-relay-socket
      socket-path
      (lambda (socket-port)
        (let ((reply-port (reply-port)))
          (relay-substitute-through-socket
           socket-port
           reply-port
           (if force-p2p-only? "mode: substitute-p2p-only" "mode: substitute"))
          #t))))
    (_ #f)))

(define (string-lines text)
  (let ((port (open-input-string text)))
    (let loop ((lines '()))
      (match (read-line port)
        ((? eof-object?) (reverse lines))
        (line (loop (cons line lines)))))))

(define (non-empty-lines text)
  (filter (lambda (line) (not (string-null? line)))
          (string-lines text)))

(define (drop-final-empty-line lines)
  (if (and (pair? lines) (string-null? (last lines)))
      (drop-right lines 1)
      lines))

(define (query-data-lines text)
  ;; Query replies end with a blank line, but "info" records may contain an
  ;; empty deriver field. Drop only the final terminator.
  (drop-final-empty-line (string-lines text)))

(define (call-builtin-substitute args input)
  (let ((output (open-output-string)))
    ;; Capture the built-in substituter reply instead of writing it to fd 4.
    (parameterize ((builtin:%reply-file-descriptor #f)
                   (current-input-port (open-input-string input))
                   (current-output-port output))
      (apply builtin:guix-substitute args))
    (get-output-string output)))

(define (query-command line)
  (match (string-tokenize line)
    ((command _ ...) command)
    (_ #f)))

(define (query-paths line)
  (match (string-tokenize line)
    ((_ paths ...) paths)
    (_ '())))

(define (missing-paths requested present)
  (filter (lambda (path) (not (member path present))) requested))

(define (info-present-paths lines)
  (let loop ((remaining lines)
             (paths '()))
    (match remaining
      (() (reverse paths))
      ((path deriver ref-count rest ...)
       (let* ((count (or (string->number ref-count) 0))
              (after-refs (drop rest count)))
         (match after-refs
           ((_download-size _nar-size tail ...)
            (loop tail (cons path paths)))
           (_
            (reverse (cons path paths)))))))))

(define (socket-query-lines socket-path mode-line line)
  (call-with-relay-socket
   socket-path
   (lambda (socket-port)
     (write-line socket-port mode-line)
     (write-line socket-port line)
     (force-output socket-port)
     (shutdown socket-port 1)
     (let loop ((lines '()))
       (match (read-line socket-port)
         ((? eof-object?) (reverse lines))
         (socket-line
          (cond
           ((string=? socket-line "fd4-empty:")
            (loop (cons "" lines)))
           ((string-prefix? "fd4:" socket-line)
            (let ((data (string-drop socket-line 4)))
              (if (string-null? data)
                  (reverse lines)
                  (loop (cons data lines)))))
           ((string-prefix? "out:" socket-line)
            (write-trace-line (string-drop socket-line 4))
            (loop lines))
           (else
            (if (string-null? socket-line)
                (reverse lines)
                (loop (cons socket-line lines)))))))))))

(define (write-query-lines reply-port lines)
  (for-each (lambda (line) (write-reply-line reply-port line)) lines))

(define (write-query-end reply-port)
  (write-reply-line reply-port ""))

(define (handle-builtin-first-query-line socket-path reply-port line)
  (let* ((command (query-command line))
         (requested (query-paths line))
         (builtin-lines
          (query-data-lines
           (call-builtin-substitute '("--query") (string-append line "\n"))))
         (present
          (match command
            ("have" builtin-lines)
            ("info" (info-present-paths builtin-lines))
            (_ requested)))
         (missing (missing-paths requested present))
         (p2p-lines
          (if (and socket-path
                   (not (null? missing))
                   (member command '("have" "info")))
              (socket-query-lines
               socket-path
               "mode: query-p2p-only"
               (string-append command " " (string-join missing " ")))
              '())))
    (write-query-lines reply-port builtin-lines)
    (write-query-lines reply-port p2p-lines)
    (write-query-end reply-port)))

(define (substitute-terminal-line lines)
  (find terminal-substitute-reply? lines))

(define (not-found-reply? line)
  (and line (string=? (string-trim-both line) "not-found")))

(define (relay-single-substitute-through-socket socket-path reply-port line)
  (call-with-relay-socket
   socket-path
   (lambda (socket-port)
     (write-line socket-port "mode: substitute-p2p-only")
     (write-line socket-port line)
     (force-output socket-port)
     (shutdown socket-port 1)
     (let ((destination (substitute-destination line)))
       (unless destination
         (error "invalid substitute command" line))
       (handle-relay-output socket-port reply-port (list destination))))))

(define (handle-builtin-first-substitute-line socket-path reply-port line)
  (let* ((builtin-lines
          (non-empty-lines
           (call-builtin-substitute '("--substitute") (string-append line "\n"))))
         (terminal (substitute-terminal-line builtin-lines)))
    (if (and socket-path (not-found-reply? terminal))
        (relay-single-substitute-through-socket socket-path reply-port line)
        (for-each (lambda (reply) (write-reply-line reply-port reply))
                  builtin-lines))))

(define (builtin-first-through-socket args socket-path)
  (match args
    (("--query" _ ...)
     (let ((reply-port (reply-port)))
       (let loop ()
         (match (read-line)
           ((? eof-object?) #t)
           (line
            (handle-builtin-first-query-line socket-path reply-port line)
            (loop))))))
    (("--substitute" _ ...)
     (let ((reply-port (reply-port)))
       (let loop ()
         (match (read-line)
           ((? eof-object?) #t)
           (line
            (handle-builtin-first-substitute-line socket-path reply-port line)
            (loop))))))
    (_ #f)))

(define (maybe-relay-through-socket args)
  (match args
    (((or "--query" "--substitute") _ ...)
     (let* ((socket (getenv/default "GUIX_P2P_SOCKET" %default-socket))
            (socket-path (and (socket? socket) socket)))
       (match (substitute-routing)
         ("p2p-first"
          (and socket-path (relay-through-socket args socket-path #f)))
         ("p2p-only"
          (and socket-path (relay-through-socket args socket-path #t)))
         ("builtin-first"
          (builtin-first-through-socket args socket-path))
         (_
          (builtin-first-through-socket args socket-path)))))
    (_ #f)))

(define (guix-substitute . args)
  (unless (maybe-relay-through-socket args)
    (apply builtin:guix-substitute args)))
