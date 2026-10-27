// Command tlsproxy terminates TLS 1.3 for a Rillcache RESP backend.
package main

import (
	"context"
	"flag"
	"fmt"
	"log"
	"os"
	"os/signal"
	"syscall"
	"time"
)

var version = "dev"

func main() {
	listen := flag.String("listen", "127.0.0.1:6380", "TLS listen address")
	backend := flag.String("backend", "127.0.0.1:6379", "plaintext RESP backend address")
	certFile := flag.String("cert", "", "PEM certificate chain file")
	keyFile := flag.String("key", "", "PEM private key file")
	clientCA := flag.String("client-ca", "", "optional PEM CA bundle; enables required client-certificate authentication")
	maxConns := flag.Int("max-conns", 1024, "maximum simultaneous TLS connections")
	handshakeTimeout := flag.Duration("handshake-timeout", 5*time.Second, "TLS handshake deadline")
	dialTimeout := flag.Duration("dial-timeout", 5*time.Second, "backend dial deadline")
	shutdownTimeout := flag.Duration("shutdown-timeout", 10*time.Second, "graceful shutdown deadline")
	showVersion := flag.Bool("version", false, "print version and exit")
	flag.Parse()

	if *showVersion {
		fmt.Printf("tlsproxy %s\n", version)
		return
	}

	logger := log.New(os.Stderr, "tlsproxy: ", log.LstdFlags)
	server, err := New(Config{
		ListenAddr:       *listen,
		BackendAddr:      *backend,
		CertFile:         *certFile,
		KeyFile:          *keyFile,
		ClientCAFile:     *clientCA,
		MaxConns:         *maxConns,
		HandshakeTimeout: *handshakeTimeout,
		DialTimeout:      *dialTimeout,
		Logger:           logger,
	})
	if err != nil {
		logger.Printf("configuration: %v", err)
		os.Exit(2)
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	if err := server.Start(ctx); err != nil {
		logger.Printf("start: %v", err)
		os.Exit(1)
	}
	logger.Printf("TLS 1.3 listening on %s -> %s (no HTTP ALPN)", server.Addr(), *backend)

	<-ctx.Done()
	logger.Printf("shutting down")
	shutdownCtx, cancel := context.WithTimeout(context.Background(), *shutdownTimeout)
	defer cancel()
	if err := server.Shutdown(shutdownCtx); err != nil {
		logger.Printf("shutdown: %v", err)
		os.Exit(1)
	}
}
